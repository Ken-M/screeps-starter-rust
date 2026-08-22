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

        for room in game::rooms().values() {
            if room.terminal().is_some() {
                has_terminal = true;
            }

            if room_structures(&room)
                .iter()
                .any(|s| s.structure_type() == screeps::StructureType::Extractor)
            {
                has_extractor = true;
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
        for creep in game::creeps().values() {
            total_creeps += 1;
            if let Ok(Some(role)) = creep.memory().string(crate::mem::keys::ROLE) {
                if role == ROLE_MINER {
                    let ttl = creep.ticks_to_live().unwrap_or(u32::MAX);
                    if ttl < MINER_PRESPAWN_LEAD {
                        miners_expiring += 1;
                    }
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
        }
    }
}

/// 目標のロール構成。優先度の高い順に並べる。
///
/// 目標は creep 総数の比率ではなく、コロニーの構造 (source 数・敵の有無・
/// 施設の有無) から決める。総数比だと「総数が目標を決め、目標が総数を決める」
/// 循環になり、人口の上限が恣意的な定数に縛られる。
pub fn role_targets(state: &ColonyState) -> Vec<(&'static str, i32)> {
    vec![
        // 防衛。攻撃能力を持つ敵がいるときだけ。何より優先する。
        (ROLE_DEFENDER, if state.hostiles_present { 2 } else { 0 }),
        // 静的採掘者。source に1体ずつ + 寿命が近い個体の後継。
        // WORK 5個で source の再生速度と釣り合うので、1 source 1体で足りる。
        (ROLE_MINER, state.total_sources + state.miners_expiring),
        // 運搬。source からの搬出2系統 + 拠点内の補給に1〜2体。
        (ROLE_HAULER, state.total_sources + 2),
        // 余剰労働力 (建設・修理・アップグレード)。
        (ROLE_WORKER, state.total_sources * 2 + 1),
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
            "harvester" | "harvester_spawn" | "builder" | "upgrader" | "repairer"
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
    let duty_alive = roster
        .iter()
        .any(|i| i.role == ROLE_WORKER && i.creep.memory().bool(crate::mem::keys::UPGRADE_DUTY));
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
        }
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
}
