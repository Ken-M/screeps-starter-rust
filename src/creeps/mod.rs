//! creep のロール管理とディスパッチ。
//!
//! ロールは「miner / hauler / worker」の3本柱 + 状況限定ロール (defender / 鉱物系)。
//!
//! 旧世代 (harvester / harvester_spawn が自力で source へ歩き、Memory の
//! `harvesting` フラグと `target_pos` で採取⇄配達を往復するステートマシン) は
//! 退役した。miner が source に張り付いて掘り、hauler が運び、worker が
//! 建設・修理・アップグレードを担う。container が無い間も miner は地面に
//! 落とし、hauler が拾うので、移行ギャップは無い。

mod builder;
mod defender;
mod hauler;
mod miner;
mod worker;
mod repairer;
mod upgrader;

use crate::mem::{self, MemoryExt};
use crate::util::*;
use log::*;
use screeps::prelude::*;
use screeps::{game, Creep, Part};
use std::collections::HashMap;

#[derive(PartialEq, Debug)]
enum AttackerKind {
    SHORT,
    RANGED,
    NONE,
}

pub const ROLE_MINER: &str = "miner";
pub const ROLE_HAULER: &str = "hauler";
pub const ROLE_WORKER: &str = "worker";
pub const ROLE_UPGRADER: &str = "upgrader";
pub const ROLE_DEFENDER: &str = "defender";
pub const ROLE_HARVESTER_MINERAL: &str = "harvester_mineral";
pub const ROLE_CARRIER_MINERAL: &str = "carrier_mineral";

/// 1 tick に配置転換してよい creep の数。
/// 一度に動かすとハンチングするので少しずつ寄せる。
const ROLE_REBALANCE_PER_TICK: i32 = 1;

/// miner の残り寿命がこれを切ったら、後継の生産を始める (先行生産)。
/// spawn 所要 (7パーツ×3=21tick) + 職場までの歩行 (この部屋で最大約50tick、
/// miner は MOVE 1 なので4倍遅い) + 余裕。
pub const MINER_PRESPAWN_LEAD: u32 = 250;

/// 部屋の道路がこの本数以上なら「道路網あり」とみなす。
/// 幹線 (spawn⇄source⇄controller) の舗装がおおむね完了する本数。
/// body の MOVE 比率の切り替えと hauler の目標数に使う。
pub const ROAD_NETWORK_MIN: usize = 20;

/// 滞留エネルギーがこの量を超えるごとに worker を1体追加する。
/// worker 1体 (WORK 2 前後) が寿命 1500 tick で消費できる量の目安より
/// やや小さめに取り、滞留に対して増員が先行するようにする。
const WORKER_SURPLUS_ENERGY_STEP: i32 = 1500;

/// 滞留エネルギーがこの量を超えるごとに hauler を1体追加する。
/// 増員は消費側 (upgrader / worker) だけだと輸送が追いつかず、滞留が
/// 発散する (実測: hauler を 4→3 に減らした直後、backlog 4k→15k)。
/// 消費側 (STEP 1500) より緩い傾斜で運び手も追従させる。
const HAULER_SURPLUS_ENERGY_STEP: i32 = 4000;

/// ロール構成を決めるのに要る、その tick のコロニーの状況。
pub struct ColonyState {
    /// 見えている creep の総数。
    pub total_creeps: i32,
    /// 自分の source の総数。
    pub total_sources: i32,
    /// ターミナルを持っているか (鉱物の搬出先)。
    pub has_terminal: bool,
    /// Extractor を持っているか (鉱物採取は RCL6 の Extractor が前提)。
    pub has_extractor: bool,
    /// 攻撃能力を持つ敵が可視範囲にいるか。
    pub hostiles_present: bool,
    /// 残り寿命が MINER_PRESPAWN_LEAD を切った miner の数。
    /// この分だけ miner の目標を一時的に増やし、後継を先行生産する。
    /// 旧個体が死ねば目標もカウントも同時に戻るので、恒常的な過剰にはならない。
    pub miners_expiring: i32,
    /// 生きている miner / hauler の実数。ロジスティクス崩壊の検知に使う。
    pub num_miners: i32,
    pub num_haulers: i32,
    /// 滞留している余剰エネルギー (container / storage の在庫 + 地面の落下分)。
    /// spawn / extension は生産用の備蓄なので数えない。
    /// worker の増員判断に使う: 採取が消費を上回っている間はここが積み上がる。
    pub energy_backlog: i32,
    /// 道路網 (幹線舗装) が完成しているか。body の MOVE 比率と
    /// hauler の目標数 (少数大型化) の切り替えに使う。
    pub has_road_network: bool,
    /// controller 脇の補給 container があるか。専任 upgrader
    /// (座って WORK 全振り) はこれが前提。
    pub has_controller_stock: bool,
    /// 専任 upgrader が claim できる席の総数 (搬入レーンを除いた
    /// container 周りの歩けるマス)。席より多く作っても座れずに
    /// container 周りを徘徊して搬入を妨げるだけなので、目標の上限になる。
    pub upgrader_seats: i32,
}

thread_local! {
    /// tick 内で共有するコロニー観測。詳細は `ColonyState::observe()`。
    static COLONY_CACHE: std::cell::RefCell<Option<std::rc::Rc<ColonyState>>> =
        std::cell::RefCell::new(None);
}

/// tick 先頭でキャッシュを捨てる。
pub fn clear_colony_cache() {
    COLONY_CACHE.with(|c| *c.borrow_mut() = None);
}

impl ColonyState {
    /// 経済が壊滅しているか (採掘か運搬のどちらかが途絶えた)。
    ///
    /// どちらが欠けてもエネルギー収入はゼロになる: miner がいなければ
    /// 掘れず、hauler がいなければ spawn へ届かない。この状態では
    /// 防衛より復旧を優先しないと、生産に使えるエネルギーが二度と
    /// 貯まらないデススパイラルに入る。
    pub fn economy_collapsed(&self) -> bool {
        self.num_miners == 0 || self.num_haulers == 0
    }

    /// tick 内で1回だけ観測する。
    /// do_spawn と creep_loop の両方が使うためキャッシュする。
    pub fn observe() -> std::rc::Rc<Self> {
        COLONY_CACHE.with(|cache| {
            if let Some(cached) = cache.borrow().as_ref() {
                return std::rc::Rc::clone(cached);
            }
            let state = std::rc::Rc::new(Self::measure());
            *cache.borrow_mut() = Some(std::rc::Rc::clone(&state));
            state
        })
    }

    fn measure() -> Self {
        let mut has_terminal = false;
        let mut has_extractor = false;
        let mut total_sources = 0;
        let mut hostiles_present = false;
        let mut energy_backlog = 0;
        let mut total_roads = 0usize;
        let mut has_controller_stock = false;
        let mut upgrader_seats = 0;

        for room in game::rooms().values() {
            upgrader_seats += upgrader::claimable_seats(&room).len() as i32;
            if room.terminal().is_some() {
                has_terminal = true;
            }

            for structure in room_structures(&room).iter() {
                match structure.structure_type() {
                    screeps::StructureType::Extractor => has_extractor = true,
                    screeps::StructureType::Road => total_roads += 1,
                    screeps::StructureType::Container | screeps::StructureType::Storage => {
                        if let Some(store) = structure.as_has_store() {
                            energy_backlog += store
                                .store()
                                .get_used_capacity(Some(screeps::ResourceType::Energy))
                                as i32;
                        }
                        if is_controller_stock(structure) {
                            has_controller_stock = true;
                        }
                    }
                    _ => {}
                }
            }

            // 採掘者が container 満杯時に溢れさせた分。放置するたび毎tick減衰で
            // 蒸発するので、これが積もっている = 消費側の人手不足のサイン。
            for resource in room.find(screeps::find::DROPPED_RESOURCES, None) {
                if resource.resource_type() == screeps::ResourceType::Energy {
                    energy_backlog += resource.amount() as i32;
                }
            }

            total_sources += room.find(screeps::find::SOURCES, None).len() as i32;

            if !hostiles_present {
                hostiles_present = room_hostiles(&room).iter().any(|enemy| {
                    enemy.body().iter().any(|p| {
                        p.hits() > 0
                            && matches!(
                                p.part(),
                                Part::Attack | Part::RangedAttack | Part::Work | Part::Heal
                            )
                    })
                });
            }
        }

        let mut total_creeps = 0;
        let mut miners_expiring = 0;
        let mut num_miners = 0;
        let mut num_haulers = 0;
        for creep in game::creeps().values() {
            total_creeps += 1;
            if let Ok(Some(role)) = creep.memory().string(crate::mem::keys::ROLE) {
                if role == ROLE_MINER {
                    num_miners += 1;
                    let ttl = creep.ticks_to_live().unwrap_or(u32::MAX);
                    if ttl < MINER_PRESPAWN_LEAD {
                        miners_expiring += 1;
                    }
                } else if role == ROLE_HAULER {
                    num_haulers += 1;
                }
            }
        }

        Self {
            total_creeps,
            total_sources,
            has_terminal,
            has_extractor,
            hostiles_present,
            miners_expiring,
            num_miners,
            num_haulers,
            energy_backlog,
            has_road_network: total_roads >= ROAD_NETWORK_MIN,
            has_controller_stock,
            upgrader_seats,
        }
    }
}

/// 余剰エネルギーによる worker の増員数。
///
/// 実測 (RCL2, source 2): 基本の worker 5 では消費が採取 20/tick に追いつかず、
/// container 2基が満杯 (2000×2) のまま地面に 6000 超が積もり、減衰で毎tick
/// 数エネルギーを捨て続けていた。滞留量に比例して worker を足し、余剰を
/// アップグレード進捗へ変換する。
///
/// 滞留が減れば目標も自然に下がり、過剰分は寿命切れで消えるので発振しない
/// (spawn は目標を超えては生産しない)。
fn surplus_workers(state: &ColonyState) -> i32 {
    (state.energy_backlog / WORKER_SURPLUS_ENERGY_STEP).min(state.total_sources * 3)
}

/// 余剰エネルギーによる hauler の増員数。
///
/// worker / upgrader の増員 (surplus_workers) は消費能力を増やすが、
/// エネルギーは hauler が届けて初めて消費される。運び手を固定したまま
/// 消費者だけ増やすと、滞留は source 脇 container と地面に積もり続け、
/// 落下分は毎 tick 減衰で蒸発する。滞留に比例して hauler も足す。
/// 上限は source 数 (輸送需要の源は結局 source の産出量なので)。
fn surplus_haulers(state: &ColonyState) -> i32 {
    (state.energy_backlog / HAULER_SURPLUS_ENERGY_STEP).min(state.total_sources)
}

/// 専任 upgrader の上限。
///
/// 総収入 (source 1本 ≒ 10/tick) ではなく「補給 container まで実際に届く量」
/// が制約になる。この部屋は spawn/source 側と controller が岩壁で分断され、
/// 唯一の通路が西回り約60〜80歩の大迂回路のため、hauler 1体の実効搬入は
/// 4〜5/tick、spawn/extension との分け合いで stock への定常供給は 4〜10/tick
/// 程度 (実測: 定常 3.7/tick、滞留食い潰しのバーストで 9.7/tick)。
/// upgrader 1体 (WORK 6) の消費は 6/tick なので、sources+1 = 3体 (容量
/// 18/tick) でバースト受け入れの余裕を残しつつ、恒常的に飢える個体
/// (1世代 800 energy の無駄) を作らない。
/// 将来: stock の枯渇率を Memory で計測し、供給実測に追従させる。
fn upgrader_cap(state: &ColonyState) -> i32 {
    state.total_sources + 1
}

/// 専任 upgrader の目標数。増員込みの希望数を、収入上限 (upgrader_cap) と
/// 席数 (claimable_seats) の両方で頭打ちにする。席より多く作ると、座れない
/// 個体が container 周りを徘徊して搬入を妨げる。
fn upgrader_target(state: &ColonyState) -> i32 {
    if !state.has_controller_stock {
        return 0;
    }
    (state.total_sources + surplus_workers(state))
        .min(upgrader_cap(state))
        .min(state.upgrader_seats)
}

/// 目標のロール構成。優先度の高い順に並べる。
///
/// 目標は creep 総数の比率ではなく、コロニーの構造 (source 数・敵の有無・
/// 施設の有無) から決める。総数比だと「総数が目標を決め、目標が総数を決める」
/// 循環になり、人口の上限が恣意的な定数に縛られる。
pub fn role_targets(state: &ColonyState) -> Vec<(&'static str, i32)> {
    vec![
        // 防衛。攻撃能力を持つ敵がいるときだけ。何より優先する。
        //
        // ただし経済が壊滅している間は作らない。収入がゼロの状態で防衛
        // creep を逐次投入しても、spawn に貯まった端から最小 body が出て
        // 消耗するだけで、敵 (TOUGH + HEAL 持ちの invader 隊) には無力。
        // 実測: invader 4体の襲来中に defender を8体投入し、その間に
        // 経済 creep が全滅して spawn 169/300・tower 0 の詰みに陥った。
        // rampart と tower で籠城できる間に収入を立て直す方が確実に立ち直る。
        (
            ROLE_DEFENDER,
            if state.hostiles_present && !state.economy_collapsed() {
                2
            } else {
                0
            },
        ),
        // 静的採掘者。source に1体ずつ + 寿命が近い個体の後継。
        // WORK 5個で source の再生速度と釣り合うので、1 source 1体で足りる。
        (ROLE_MINER, state.total_sources + state.miners_expiring),
        // 運搬。source からの搬出2系統 + 拠点内の補給。
        // 道路網が完成すると body が CARRY2:MOVE1 の大型になるので、
        // 同じ輸送量を少ない体数で運べる (spawn 予算と CPU の節約)。
        // エネルギーが滞留しているときは輸送も詰まっているので増員する。
        (
            ROLE_HAULER,
            state.total_sources
                + if state.has_road_network { 1 } else { 2 }
                + surplus_haulers(state),
        ),
        // 専任アップグレード係。controller 脇の補給 container の隣に座り、
        // withdraw と upgrade を同 tick に併用して WORK 全振り body を回す。
        // container が無いうちは 0 (worker の名前パリティ傾斜が代替)。
        // 増員は収入上限と席数で頭打ち (詳細は upgrader_target)。
        (ROLE_UPGRADER, upgrader_target(state)),
        // 汎用労働力 (建設・修理、暇ならアップグレード)。
        // 専任 upgrader が立っている間はその分を差し引き、upgrader の上限から
        // あふれた増員分はこちらへ戻す (機動力があり建設・修理もこなすので、
        // 補給 container の周りに滞留しない)。
        // 合計は従来の sources*2 + 1 + 増員 と同じに保つ。
        (
            ROLE_WORKER,
            if state.has_controller_stock {
                let overflow =
                    state.total_sources + surplus_workers(state) - upgrader_target(state);
                state.total_sources + 1 + overflow
            } else {
                state.total_sources * 2 + 1 + surplus_workers(state)
            },
        ),
        // 鉱物系は Extractor (RCL6) があって初めて成立する。
        // 注意: 旧世代の採取ステートマシン退役に伴い、鉱物の採取側は
        // 未実装に戻っている。Extractor 解禁 (RCL6) までに mineral 専用の
        // miner/hauler を実装すること。それまで目標は 0 のまま。
        (ROLE_HARVESTER_MINERAL, 0),
        (
            ROLE_CARRIER_MINERAL,
            if state.has_extractor && state.has_terminal { 1 } else { 0 },
        ),
    ]
}

/// 目標の合計 = 目指す人口。spawn はこれを上限として生産する。
pub fn total_role_target(state: &ColonyState) -> i32 {
    role_targets(state).iter().map(|(_, n)| n).sum()
}

/// 今いちばん不足しているロール。生産する body を決めるのに使う。
pub fn most_needed_role(state: &ColonyState) -> &'static str {
    let mut counts: HashMap<String, i32> = HashMap::new();
    for creep in game::creeps().values() {
        if let Ok(Some(role)) = creep.memory().string(crate::mem::keys::ROLE) {
            *counts.entry(role).or_insert(0) += 1;
        }
    }

    for (role, target) in role_targets(state) {
        if counts.get(role).copied().unwrap_or(0) < target {
            return role;
        }
    }

    ROLE_WORKER
}

/// 1 tick 分の creep の情報。
///
/// body も Memory も JS 越しの取得なので、tick 内で何度も読み直すと高くつく。
/// 最初に1回だけ読んで使い回す。
struct CreepInfo {
    creep: Creep,
    name: String,
    role: String,
    attacker_kind: AttackerKind,
    work: u32,
    carry: u32,
    attack: u32,
}

impl CreepInfo {
    /// この creep がそのロールをこなせる body を持っているか。
    fn can_fill(&self, role: &str) -> bool {
        match role {
            ROLE_CARRIER_MINERAL | ROLE_HAULER => self.carry > 0,
            ROLE_MINER => self.work > 0,
            ROLE_DEFENDER => self.attack > 0,
            _ => self.work > 0 && self.carry > 0,
        }
    }
}

/// 不足が最も大きく、かつこの creep がこなせるロールを選ぶ。
fn pick_role(
    info: &CreepInfo,
    targets: &[(&'static str, i32)],
    counts: &HashMap<String, i32>,
) -> String {
    let mut best: Option<(&'static str, i32)> = None;

    for (role, target) in targets.iter() {
        if !info.can_fill(role) {
            continue;
        }
        let current = counts.get(*role).copied().unwrap_or(0);
        let deficit = target - current;
        if deficit <= 0 {
            continue;
        }
        if best.is_none_or(|(_, b)| deficit > b) {
            best = Some((role, deficit));
        }
    }

    // どこも埋まっていなければ余剰労働力として働く。
    match best {
        Some((role, _)) => role.to_string(),
        None => {
            if info.can_fill(ROLE_WORKER) {
                ROLE_WORKER.to_string()
            } else if info.can_fill(ROLE_HAULER) {
                ROLE_HAULER.to_string()
            } else {
                ROLE_DEFENDER.to_string()
            }
        }
    }
}

/// 敵がいれば撃つ。戻り値は「この tick の移動を消費したか」。
///
/// Screeps は 1 tick に attack と transfer と move を同時に発行できるので、
/// 撃っただけなら経済の仕事も続けてよい。攻撃用の Memory キーは採取系とは
/// 分離してある (過去に target_pos を共有して採取状態を壊していた)。
fn attacker_routine(creep: &Creep, kind: &AttackerKind) -> bool {
    let Some(room) = creep.room() else {
        return false;
    };
    let enemies = room_hostiles(&room);

    if enemies.is_empty() {
        creep.memory().del(crate::mem::keys::ATTACK_TARGET_POS);
        return false;
    }

    for enemy in enemies.iter() {
        let hit = match kind {
            AttackerKind::SHORT => creep.attack(enemy).is_ok(),
            AttackerKind::RANGED => creep.ranged_attack(enemy).is_ok(),
            AttackerKind::NONE => false,
        };
        if hit {
            info!("attack to enemy!!");
            // 撃てたなら近づく必要はない。移動は経済側に譲る。
            return false;
        }
    }

    false
}

pub fn creep_loop() {
    let colony = ColonyState::observe();
    let total_creeps = colony.total_creeps as usize;

    // creep ごとの情報とロール別実数を1回だけ集める。
    let mut role_counts: HashMap<String, i32> = HashMap::new();
    let mut roster: Vec<CreepInfo> = Vec::with_capacity(total_creeps);

    for creep in game::creeps().values() {
        let name = creep.name();
        let cmem = creep.memory();

        let mut role = cmem
            .string(crate::mem::keys::ROLE)
            .ok()
            .flatten()
            .unwrap_or_else(|| String::from("none"));

        // 退役したロール名を持つ creep は未割り当てとして扱い、即座に
        // 新体制のロールへ振り直す。
        if matches!(
            role.as_str(),
            "harvester" | "harvester_spawn" | "builder" | "repairer"
        ) {
            cmem.del(crate::mem::keys::ROLE);
            // 旧ステートマシンの残骸も掃除しておく。
            for key in crate::mem::keys::LEGACY_HARVEST_KEYS {
                cmem.del(key);
            }
            role = String::from("none");
        }

        // body の走査はここ1回だけ。
        let mut work = 0;
        let mut carry = 0;
        let mut attack = 0;
        let mut attacker_kind = AttackerKind::NONE;
        for part in creep.body().iter() {
            if part.hits() == 0 {
                continue;
            }
            match part.part() {
                Part::Work => work += 1,
                Part::Carry => carry += 1,
                Part::Attack => {
                    attack += 1;
                    if attacker_kind == AttackerKind::NONE {
                        attacker_kind = AttackerKind::SHORT;
                    }
                }
                Part::RangedAttack => {
                    attack += 1;
                    attacker_kind = AttackerKind::RANGED;
                }
                _ => {}
            }
        }

        if role != "none" {
            *role_counts.entry(role.clone()).or_insert(0) += 1;
        }

        roster.push(CreepInfo {
            creep,
            name,
            role,
            attacker_kind,
            work,
            carry,
            attack,
        });
    }

    // 目標に寄せる再配分。余っているロールから足りないロールへ、1 tick に
    // 1体だけ移す (まとめて動かすとハンチングする)。
    let targets = role_targets(&colony);
    {
        let mut moved = 0;
        for (role, target) in targets.iter() {
            if moved >= ROLE_REBALANCE_PER_TICK {
                break;
            }
            let current = role_counts.get(*role).copied().unwrap_or(0);
            if current >= *target {
                continue;
            }

            let surplus_role = targets
                .iter()
                .filter(|(r, t)| role_counts.get(*r).copied().unwrap_or(0) > *t)
                .max_by_key(|(r, t)| role_counts.get(*r).copied().unwrap_or(0) - t)
                .map(|(r, _)| *r);

            let Some(surplus_role) = surplus_role else {
                break;
            };

            let victim = roster
                .iter_mut()
                .find(|i| i.role == surplus_role && i.can_fill(role));

            if let Some(victim) = victim {
                info!("rebalance {}: {} -> {}", victim.name, surplus_role, role);
                victim.creep.memory().set(crate::mem::keys::ROLE, *role);
                victim.creep.memory().del(crate::mem::keys::UPGRADE_DUTY);
                victim.creep.memory().del(crate::mem::keys::MINE_AT);
                victim.creep.memory().del(crate::mem::keys::UPGRADE_SEAT);
                victim.creep.memory().del(crate::mem::keys::REPAIR_AT);
                victim.role = role.to_string();

                *role_counts.entry(surplus_role.to_string()).or_insert(0) -= 1;
                *role_counts.entry(role.to_string()).or_insert(0) += 1;
                moved += 1;
            }
        }
    }

    // 固定アップグレード係を1体維持する。建設が続いても controller の進捗が
    // 完全には止まらないようにする。Memory フラグで粘着させる (tick ごとに
    // 指名が移ると、controller までの道中で指名が外れて誰も到着しない)。
    let duty_alive = roster.iter().any(|i| {
        i.role == ROLE_UPGRADER
            || (i.role == ROLE_WORKER && i.creep.memory().bool(crate::mem::keys::UPGRADE_DUTY))
    });
    if !duty_alive {
        if let Some(candidate) = roster.iter().find(|i| i.role == ROLE_WORKER) {
            info!("{} takes upgrade duty", candidate.name);
            candidate.creep.memory().set(crate::mem::keys::UPGRADE_DUTY, true);
        }
    }

    // 実処理。
    for info in roster.iter_mut() {
        debug!("running creep {}, cpu:{}", info.name, game::cpu::get_used());

        if info.creep.spawning() {
            continue;
        }

        // ロールが未割り当てなら今決める。
        if info.role == "none" {
            let role = pick_role(info, &targets, &role_counts);
            info.creep.memory().set(crate::mem::keys::ROLE, role.as_str());
            *role_counts.entry(role.clone()).or_insert(0) += 1;
            info.role = role;
        }

        let creep = &info.creep;

        // 攻撃パーツ持ちは、敵がいれば手番のついでに撃つ (defender 以外)。
        // defender は自前で追撃まで行うので二重に撃たない。
        if info.attacker_kind != AttackerKind::NONE && info.role != ROLE_DEFENDER {
            attacker_routine(creep, &info.attacker_kind);
        }

        match info.role.as_str() {
            "miner" => miner::run_miner(creep),
            "hauler" => hauler::run_hauler(creep),
            "defender" => defender::run_defender(creep),
            "worker" => {
                worker::run_worker(creep, creep.memory().bool(crate::mem::keys::UPGRADE_DUTY));
            }
            "upgrader" => upgrader::run_dedicated_upgrader(creep),

            // 鉱物系: 旧採取ステートマシンの退役により採取側が未実装。
            // Extractor 解禁 (RCL6) までに専用実装を入れる。それまでは
            // 目標 0 なので通常ここには来ないが、来ても遊ばせない。
            "harvester_mineral" | "carrier_mineral" => {
                worker::run_worker(creep, false);
            }

            other => {
                warn!("{} has unknown role {:?}; treating as worker", info.name, other);
                worker::run_worker(creep, false);
            }
        }
    }

    // 統計 (観測用)。
    let root = mem::root();
    let n = |role: &str| role_counts.get(role).copied().unwrap_or(0);
    root.set(crate::mem::keys::NUM_MINER, n(ROLE_MINER));
    root.set(crate::mem::keys::NUM_HAULER, n(ROLE_HAULER));
    root.set(crate::mem::keys::NUM_WORKER, n(ROLE_WORKER));
    root.set(crate::mem::keys::NUM_DEFENDER, n(ROLE_DEFENDER));
    root.set(crate::mem::keys::TOTAL_NUM, total_creeps as i32);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(sources: i32, hostiles: bool, expiring: i32) -> ColonyState {
        ColonyState {
            total_creeps: 10,
            total_sources: sources,
            has_terminal: false,
            has_extractor: false,
            hostiles_present: hostiles,
            miners_expiring: expiring,
            // 既定は経済が回っている状態。壊滅時の分岐は専用のテストで見る。
            num_miners: sources,
            num_haulers: sources,
            energy_backlog: 0,
            has_road_network: false,
            has_controller_stock: false,
            // 既定は席数が制約にならない値。席の制約は専用のテストで見る。
            upgrader_seats: 8,
        }
    }

    fn target_of(st: &ColonyState, role: &str) -> i32 {
        role_targets(st).iter().find(|(n, _)| *n == role).unwrap().1
    }

    #[test]
    fn 補給containerがあればupgrader専任が立ち_総数は変わらない() {
        let mut st = state(2, false, 0);
        let base_total = total_role_target(&st);
        assert_eq!(target_of(&st, ROLE_UPGRADER), 0);

        st.has_controller_stock = true;
        assert_eq!(target_of(&st, ROLE_UPGRADER), 2);
        assert_eq!(target_of(&st, ROLE_WORKER), 3);
        // worker から振り替えるだけで人口は増やさない。
        assert_eq!(total_role_target(&st), base_total);
    }

    #[test]
    fn 余剰エネルギーの増員は専任upgraderに乗る() {
        let mut st = state(2, false, 0);
        st.has_controller_stock = true;
        let base = target_of(&st, ROLE_UPGRADER);
        st.energy_backlog = WORKER_SURPLUS_ENERGY_STEP;
        assert_eq!(target_of(&st, ROLE_UPGRADER), base + 1);
    }

    #[test]
    fn upgraderは席数を超えて作らない() {
        let mut st = state(2, false, 0);
        st.has_controller_stock = true;
        st.energy_backlog = 10_000; // surplus 6 → 希望 8, 収入上限 4

        // 実測の詰まり時を再現: 歩けるマス4つ − 搬入レーン1 = 席3。
        st.upgrader_seats = 3;
        assert_eq!(target_of(&st, ROLE_UPGRADER), 3);
        // 席からあふれた分も worker へ。合計は不変。
        assert_eq!(
            target_of(&st, ROLE_UPGRADER) + target_of(&st, ROLE_WORKER),
            2 * 2 + 1 + 6
        );
    }

    #[test]
    fn upgrader増員は供給上限で頭打ちになり_あふれはworkerへ() {
        let mut st = state(2, false, 0);
        st.has_controller_stock = true;

        // 実測の停滞時を再現: backlog 10k → surplus 6。西回り迂回路の
        // 実効供給では upgrader (WORK6) は sources+1 = 3体が上限。
        st.energy_backlog = 10_000;
        assert_eq!(target_of(&st, ROLE_UPGRADER), 3);
        // あふれた5体は worker (基本 3) へ戻る。
        assert_eq!(target_of(&st, ROLE_WORKER), 3 + 5);
        // upgrader + worker の合計は上限導入前 (sources*2 + 1 + 増員) と同じ。
        assert_eq!(
            target_of(&st, ROLE_UPGRADER) + target_of(&st, ROLE_WORKER),
            2 * 2 + 1 + 6
        );
    }

    #[test]
    fn 道路網が完成するとhaulerは少数大型に切り替わる() {
        let mut st = state(2, false, 0);
        assert_eq!(target_of(&st, ROLE_HAULER), 4);
        st.has_road_network = true;
        assert_eq!(target_of(&st, ROLE_HAULER), 3);
    }

    #[test]
    fn エネルギーが滞留するとhaulerも増える() {
        let mut st = state(2, false, 0);
        let base = target_of(&st, ROLE_HAULER);

        // 増員の入口。1 STEP 分の滞留で +1。
        st.energy_backlog = HAULER_SURPLUS_ENERGY_STEP;
        assert_eq!(target_of(&st, ROLE_HAULER), base + 1);

        // STEP 未満なら基本数のまま。
        st.energy_backlog = HAULER_SURPLUS_ENERGY_STEP - 1;
        assert_eq!(target_of(&st, ROLE_HAULER), base);

        // 実測の発散時 (backlog 15k) でも上限は source 数まで。
        st.energy_backlog = 15_000;
        assert_eq!(target_of(&st, ROLE_HAULER), base + st.total_sources);
    }

    fn worker_target(st: &ColonyState) -> i32 {
        role_targets(st)
            .iter()
            .find(|(n, _)| *n == ROLE_WORKER)
            .unwrap()
            .1
    }

    #[test]
    fn 平時は防衛と鉱物がゼロ() {
        let targets = role_targets(&state(2, false, 0));
        let get = |r: &str| targets.iter().find(|(n, _)| *n == r).unwrap().1;
        assert_eq!(get(ROLE_DEFENDER), 0);
        assert_eq!(get(ROLE_HARVESTER_MINERAL), 0);
        assert_eq!(get(ROLE_CARRIER_MINERAL), 0);
    }

    #[test]
    fn 敵がいれば防衛が立ち_最優先に並ぶ() {
        let targets = role_targets(&state(2, true, 0));
        assert_eq!(targets[0], (ROLE_DEFENDER, 2));
    }

    #[test]
    fn 経済が壊滅している間は防衛より復旧を優先する() {
        // miner か hauler が途絶えると収入がゼロになり、防衛 creep を
        // 逐次投入しても最小 body が溶けるだけになる (実測のデススパイラル)。
        let mut st = state(2, true, 0);
        assert_eq!(target_of(&st, ROLE_DEFENDER), 2);

        st.num_haulers = 0;
        assert!(st.economy_collapsed());
        assert_eq!(target_of(&st, ROLE_DEFENDER), 0);
        // 復旧側の目標は据え置き (spawn はこちらを作るようになる)。
        assert_eq!(target_of(&st, ROLE_MINER), 2);

        st.num_haulers = 2;
        st.num_miners = 0;
        assert_eq!(target_of(&st, ROLE_DEFENDER), 0);

        // 経済が戻れば防衛も戻る。
        st.num_miners = 2;
        assert_eq!(target_of(&st, ROLE_DEFENDER), 2);
    }

    #[test]
    fn 人口目標はロール目標の合計と一致する() {
        let st = state(2, false, 0);
        let sum: i32 = role_targets(&st).iter().map(|(_, n)| n).sum();
        assert_eq!(total_role_target(&st), sum);
        // source 2 なら miner2 + hauler4 + worker5 = 11
        assert_eq!(sum, 11);
    }

    #[test]
    fn 寿命間近のminerの数だけ目標が一時的に増える() {
        let base = total_role_target(&state(2, false, 0));
        let bumped = total_role_target(&state(2, false, 1));
        assert_eq!(bumped, base + 1);
    }

    #[test]
    fn エネルギーが滞留するとworkerが増える() {
        let mut st = state(2, false, 0);
        let base = worker_target(&st);

        // 実測時の状況を再現: container 満杯 2000×2 + 地面 6000 = 10000。
        st.energy_backlog = 10_000;
        assert_eq!(worker_target(&st), base + 6);

        // 滞留が掃ければ基本数に戻る。
        st.energy_backlog = 1_400;
        assert_eq!(worker_target(&st), base);
    }

    #[test]
    fn 滞留による増員には上限がある() {
        let mut st = state(2, false, 0);
        let base = worker_target(&st);

        // storage が積み上がる将来 (RCL4+) でも無制限には増やさない。
        st.energy_backlog = 1_000_000;
        assert_eq!(worker_target(&st), base + st.total_sources * 3);
    }
}
