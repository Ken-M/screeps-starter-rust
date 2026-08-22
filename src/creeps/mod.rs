mod builder;
mod harvester;
mod hauler;
mod miner;
mod worker;
mod repairer;
mod upgrader;

use crate::constants::*;
use crate::mem::{self, MemoryExt};
use crate::util::*;
use log::*;
use screeps::action_error_codes::{CreepMoveByPathErrorCode, HarvestErrorCode};
use screeps::enums::StructureObject;
use screeps::local::Position;
use screeps::pathfinder::SearchResults;
use screeps::prelude::*;
use screeps::{find, game, look, Creep, Part};
use std::collections::HashMap;

#[derive(PartialEq, Debug)]
enum AttackerKind {
    SHORT,
    RANGED,
    NONE,
}

/// 採取先が1つも見つからなかったとき、次に再探索するまで待つ tick 数。
const HARVEST_RETRY_BACKOFF: u32 = 10;

pub const ROLE_MINER: &str = "miner";
pub const ROLE_HAULER: &str = "hauler";
pub const ROLE_HARVESTER: &str = "harvester";
pub const ROLE_HARVESTER_SPAWN: &str = "harvester_spawn";
pub const ROLE_HARVESTER_MINERAL: &str = "harvester_mineral";
pub const ROLE_CARRIER_MINERAL: &str = "carrier_mineral";
/// 建設・修理・アップグレードを兼ねる余剰労働力。
/// 旧実装の builder / upgrader / repairer を統合したもの。
pub const ROLE_WORKER: &str = "worker";

/// 1 tick に配置転換してよい creep の数。
/// 一度に動かすとハンチングするので少しずつ寄せる。
const ROLE_REBALANCE_PER_TICK: i32 = 1;

/// 目標のロール構成。優先度の高い順に並べる。
///
/// 旧実装は if-else の連鎖で、基準が「固定値3」「総数比」「絶対値1000」「総数13超」と
/// バラバラだった。さらにカウンタが加算のみで既存 creep を見直さないため、分布が
/// 「どの順序で creep が死んで生まれたか」に依存する経路依存の値になり、目標比率に
/// 収束しなかった。建設ラッシュ中に育ったコロニーは建設が終わっても永久に builder 過多。
///
/// 目標を1箇所の関数として定義し、毎tick「目標 − 現状」の差分で配分する。
/// ロール構成を決めるのに要る、その tick のコロニーの状況。
pub struct ColonyState {
    /// 見えている creep の総数。
    pub total_creeps: i32,
    /// ターミナルを持っているか (鉱物の搬出先)。
    pub has_terminal: bool,
    /// 隣に container が完成している source の数。
    ///
    /// 静的採掘者はこの container に住み着いて掘り続ける。container が無い source に
    /// 置いても降ろす先が無いので、その source には採掘者を割り当てない。
    pub sources_with_container: i32,
    /// Extractor を持っているか。
    ///
    /// 鉱物採取には Extractor (RCL6) が要る。旧実装は creep 総数だけで
    /// harvester_mineral を割り当てていたため、Extractor の無い部屋でも
    /// 1体が鉱物採取に就き、採取先を見つけられないまま待機し続けていた。
    pub has_extractor: bool,
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
    ///
    /// do_spawn と creep_loop の両方が使うため、素直に呼ぶと部屋 × 構造物 ×
    /// source の走査が 1 tick に2回走る。
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

        let mut sources_with_container = 0;

        for room in game::rooms().values() {
            if room.terminal().is_some() {
                has_terminal = true;
            }

            let structures = room_structures(&room);
            if structures
                .iter()
                .any(|s| s.structure_type() == screeps::StructureType::Extractor)
            {
                has_extractor = true;
            }

            for source in room.find(find::SOURCES, None) {
                let has_container = structures.iter().any(|s| {
                    s.structure_type() == screeps::StructureType::Container
                        && s.pos().is_near_to(source.pos())
                });
                if has_container {
                    sources_with_container += 1;
                }
            }
        }

        Self {
            total_creeps: game::creeps().values().count() as i32,
            has_terminal,
            has_extractor,
            sources_with_container,
        }
    }
}

pub fn role_targets(state: &ColonyState) -> Vec<(&'static str, i32)> {
    let total = state.total_creeps;
    let has_terminal = state.has_terminal;
    let has_extractor = state.has_extractor;
    let t = total.max(1);

    vec![
        // 静的採掘者。container のある source に1体ずつ。
        // WORK 5個で source の再生速度と釣り合うので、それ以上は要らない。
        (ROLE_MINER, state.sources_with_container),
        // spawn/extension への補給は最優先。ここが切れると何も回らない。
        (ROLE_HARVESTER_SPAWN, std::cmp::min(3, t)),
        // 運搬者。採掘者が掘った分を運ぶ。採掘者がいて初めて意味を持つ。
        // 往復距離が長い部屋ほど多く要るが、まずは採掘者と同数から。
        (ROLE_HAULER, state.sources_with_container),
        // 採取本体。採掘者が立ち上がるまでの主力で、立ち上がった後も
        // container の無い source を拾う。
        (ROLE_HARVESTER, std::cmp::max(2, t * 3 / 10)),
        // 余剰労働力。内訳 (建設 / 修理 / アップグレード) はその時の仕事の
        // 有無で自動的に決まるので、ここでは総量だけを決める。
        (ROLE_WORKER, std::cmp::max(1, t * 4 / 10)),
        // 鉱物系は Extractor があって初めて成立する。無い部屋で割り当てても
        // 採取先が存在せず、その creep は待機し続けるだけになる。
        (
            ROLE_HARVESTER_MINERAL,
            if has_extractor && t > 10 { 1 } else { 0 },
        ),
        (
            ROLE_CARRIER_MINERAL,
            if has_extractor && has_terminal && t > 10 { 1 } else { 0 },
        ),
    ]
}

/// 今いちばん不足しているロール。生産する body を決めるのに使う。
///
/// 旧実装は creep を作ってから creep_loop がロールを割り当てていたため、
/// body は常に同じ汎用構成 (MOVE/MOVE/CARRY/WORK の繰り返し) だった。
/// 静的採掘者は WORK 偏重、運搬者は CARRY 偏重が正解なので、
/// 生産時点でロールを決めて body を合わせる。
pub fn most_needed_role(state: &ColonyState) -> &'static str {
    let mut counts: HashMap<String, i32> = HashMap::new();
    for creep in game::creeps().values() {
        if let Ok(Some(role)) = creep.memory().string("role") {
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

/// この creep がそのロールをこなせる body を持っているか。
///
/// 旧実装は body を一切見ずに列挙順の先頭3体を無条件で harvester_spawn にしていた。
/// WORK 1個・ATTACK 10個の creep が採取係になり得る。
fn can_fill_role(creep: &Creep, role: &str) -> bool {
    let mut work = 0;
    let mut carry = 0;
    for part in creep.body().iter() {
        if part.hits() == 0 {
            continue;
        }
        match part.part() {
            Part::Work => work += 1,
            Part::Carry => carry += 1,
            _ => {}
        }
    }

    match role {
        // 運搬だけなので CARRY があればよい。
        ROLE_CARRIER_MINERAL | ROLE_HAULER => carry > 0,
        // 静的採掘者は掘って隣の container に移すだけ。
        ROLE_MINER => work > 0,
        // 採取・建設・修理・アップグレードには WORK と CARRY の両方が要る。
        _ => work > 0 && carry > 0,
    }
}

/// 不足が最も大きく、かつこの creep がこなせるロールを選ぶ。
fn pick_role(creep: &Creep, targets: &[(&'static str, i32)], counts: &HashMap<String, i32>) -> String {
    let mut best: Option<(&'static str, i32)> = None;

    for (role, target) in targets.iter() {
        if !can_fill_role(creep, role) {
            continue;
        }
        let current = counts.get(*role).copied().unwrap_or(0);
        let deficit = target - current;
        if deficit <= 0 {
            continue;
        }
        // 同点なら targets の並び順 (優先度) が勝つ。
        if best.map_or(true, |(_, b)| deficit > b) {
            best = Some((role, deficit));
        }
    }

    // どこも埋まっているなら採取に回す。
    // 旧実装は catch-all が repairer で、緊急時に全員が repairer 化していた。
    match best {
        Some((role, _)) => role.to_string(),
        None => {
            if can_fill_role(creep, ROLE_HARVESTER) {
                ROLE_HARVESTER.to_string()
            } else {
                ROLE_CARRIER_MINERAL.to_string()
            }
        }
    }
}

/// ターゲットの有効期限を経路長から決める。
///
/// 旧実装は 5〜20 の固定値だった。カウントダウンは移動中に毎tick減るが、
/// 基本 body は MOVE 比 1/2 なので平地1マスに2tick かかる。storage 用の初期値 10 だと
/// 5マス進むごとにフル再探索を払う計算になり、部屋の対角 (最大約70マス) には
/// いつまでも到達できなかった。
/// 経路長 × 2 に余裕を足した値にする。
fn path_ttl_from(path: &[Position]) -> i32 {
    let steps = path.len() as i32;
    (steps * 2 + 5).clamp(5, 200)
}

fn reset_source_target(
    creep: &Creep,
    is_harvester: bool,
    harvest_kind: &ResourceKind,
) -> (SearchResults, Position) {
    debug!("harvesting : reset_source_target");

    if is_harvester == true {
        // active sourceをチェック.
        let res = find_nearest_active_source(&creep, harvest_kind, false);
        debug!(
            "harvesting : find_nearest_active_source result:{:?}",
            res.path()
        );

        let path = res.path();
        if path.len() > 0 && res.incomplete() == false {
            let last_pos = *(path.last().unwrap());
            let json_str = serde_json::to_string(&last_pos).unwrap();
            creep.memory().set("target_pos", json_str);
            creep.memory().set("target_pos_count", path_ttl_from(&path));
            creep.memory().set("will_harvest_from_storage", false);
            creep.memory().del("nothing_to_harvest");

            debug!(
                "harvesting : target_pos:{:?}",
                creep.memory().string("target_pos")
            );

            return (res, last_pos);
        }

        // storageをチェック.
        if *harvest_kind == ResourceKind::ENERGY {
            let res = find_nearest_stored_source(&creep, harvest_kind, true);

            let path = res.path();
            if path.len() > 0 && res.incomplete() == false {
                let last_pos = *(path.last().unwrap());
                let json_str = serde_json::to_string(&last_pos).unwrap();
                creep.memory().set("target_pos", json_str);
                creep.memory().set("target_pos_count", path_ttl_from(&path));
                creep.memory().set("will_harvest_from_storage", true);
                creep.memory().del("nothing_to_harvest");

                debug!(
                    "harvesting : target_pos:{:?}",
                    creep.memory().string("target_pos")
                );

                return (res, last_pos);
            }
        }
    } else {
        // storageをチェック.
        let res = find_nearest_stored_source(&creep, harvest_kind, false);

        let path = res.path();
        if path.len() > 0 && res.incomplete() == false {
            let last_pos = *(path.last().unwrap());
            let json_str = serde_json::to_string(&last_pos).unwrap();
            creep.memory().set("target_pos", json_str);
            creep.memory().set("target_pos_count", path_ttl_from(&path));
            creep.memory().set("will_harvest_from_storage", true);
            creep.memory().del("nothing_to_harvest");

            debug!(
                "harvesting : target_pos:{:?}",
                creep.memory().string("target_pos")
            );

            return (res, last_pos);
        }

        // active sourceをチェック.
        let res = find_nearest_active_source(&creep, harvest_kind, true);
        debug!(
            "harvesting : find_nearest_active_source result:{:?}",
            res.path()
        );

        let path = res.path();
        if path.len() > 0 && res.incomplete() == false {
            let last_pos = *(path.last().unwrap());
            let json_str = serde_json::to_string(&last_pos).unwrap();
            creep.memory().set("target_pos", json_str);
            creep.memory().set("target_pos_count", path_ttl_from(&path));
            creep.memory().set("will_harvest_from_storage", false);
            creep.memory().del("nothing_to_harvest");

            debug!(
                "harvesting : target_pos:{:?}",
                creep.memory().string("target_pos")
            );

            return (res, last_pos);
        }
    }

    //　やむなく枯渇sourceを選ぶ.
    let res = find_nearest_exhausted_source(&creep, harvest_kind);

    let path = res.path();
    if path.len() > 0 {
        let last_pos = *(path.last().unwrap());
        let json_str = serde_json::to_string(&last_pos).unwrap();
        creep.memory().set("target_pos", json_str);
        creep.memory().set("target_pos_count", path_ttl_from(&path));
        creep.memory().set("will_harvest_from_storage", true);
        creep.memory().del("nothing_to_harvest");

        debug!(
            "harvesting : target_pos:{:?}",
            creep.memory().string("target_pos")
        );

        return (res, last_pos);
    }

    // 全部ダメならその場待機。
    //
    // 旧実装はここで「自分の現在地への経路探索」という無意味なコストを払い、
    // 次tickも同じフル再探索 (全室スキャン6回 + 経路探索3回) を無限に繰り返して
    // いた。採取先が無い状況は数tickで変わらないので、しばらく探索を止める。
    creep.memory().set("nothing_to_harvest", true);
    creep.memory().set(
        "harvest_retry_at",
        (game::time() + HARVEST_RETRY_BACKOFF) as i32,
    );
    return (empty_search_for(&creep), creep.pos());
}

/// 攻撃パーツを持つ creep の戦闘処理。
///
/// 戻り値は「この tick の移動を消費したか」。Screeps は 1 tick に attack と transfer と
/// move を同時に発行できるので、攻撃しただけなら経済の仕事も続けてよい。
///
/// 旧実装は攻撃が成功した時点で true を返し、呼び出し側が `continue` で creep の仕事を
/// 丸ごと飛ばしていた。spawn は creep 総数の 1/3 に攻撃パーツを配るので、偵察目的の敵が
/// 1体入ってくるだけで艦隊の大半が採取も建設も止めていた。
/// さらに採取ステートマシンが使う `target_pos` に敵座標を、`harvesting` に true を
/// 書き込んで状態を壊していたため、敵が消えた次の tick に「敵がいた座標を採取ターゲット
/// として扱う」無駄な往復が起きていた。攻撃用のキーは分離する。
fn attacker_routine(creep: &Creep, kind: &AttackerKind) -> bool {
    debug!("check enemies {}", creep.name());
    let room = creep.room().expect("room is not visible to you");
    let enemies = room_hostiles(&room);

    if enemies.is_empty() {
        creep.memory().del("attack_target_pos");
        return false;
    }

    // 射程内の敵がいれば撃つ。移動は消費しない。
    let mut attacked = false;
    for enemy in enemies.iter() {
        let r = match kind {
            AttackerKind::SHORT => creep.attack(enemy).is_ok(),
            AttackerKind::RANGED => creep.ranged_attack(enemy).is_ok(),
            AttackerKind::NONE => false,
        };

        if r {
            info!("attack to enemy!!");
            attacked = true;
            break;
        }
    }

    if attacked {
        // 撃てたなら近づく必要はない。移動は経済側に譲る。
        return false;
    }

    // 射程外なので接近する。ここで初めて移動を消費する。
    // RANGED の射程は 3。旧実装は 2 を指定しており、わざわざ近接圏まで
    // 踏み込みに行っていた。
    let range: u32 = match kind {
        AttackerKind::SHORT => 1,
        AttackerKind::RANGED => 3,
        AttackerKind::NONE => 1,
    };

    let res = find_nearest_enemy(&creep, range);
    let path = res.path();

    if path.len() > 0 {
        let last_pos = *(path.last().unwrap());
        let json_str = serde_json::to_string(&last_pos).unwrap();
        // 採取用の target_pos とは別のキーに書く。
        creep.memory().set("attack_target_pos", json_str);

        let move_result = move_by_search_result(&creep, &res);
        if move_result.is_ok() {
            info!("move to enemy: {:?}", move_result);
            return true;
        }
    }

    return false;
}

fn get_role_and_attacker_kind(creep: &Creep) -> (String, AttackerKind) {
    let mut attacker_kind: AttackerKind = AttackerKind::NONE;
    let role = creep.memory().string("role");
    let mut role_string = String::from("none");

    // attacker kind check.
    let body_list = creep.body();
    for body_part in body_list {
        if body_part.part() == Part::Attack {
            attacker_kind = AttackerKind::SHORT;
            break;
        } else if body_part.part() == Part::RangedAttack {
            attacker_kind = AttackerKind::RANGED;
            break;
        }
    }

    if let Ok(object) = role {
        if let Some(object) = object {
            role_string = object;
        } else {
            role_string = String::from("none");
        }
    }

    return (role_string, attacker_kind);
}

/// 1 tick 分の creep の情報。
///
/// body も Memory も JS 越しの取得なので、tick 内で何度も読み直すと高くつく。
/// 旧実装は集計ループと実処理ループでそれぞれ body を読み、さらに再配分が
/// 目標ロールの数だけ全 creep を舐め直していたため、1体の body を 1 tick に
/// 10 回近く読むことがあった。最初に1回だけ読んで使い回す。
struct CreepInfo {
    creep: Creep,
    name: String,
    role: String,
    attacker_kind: AttackerKind,
    work: u32,
    carry: u32,
}

impl CreepInfo {
    /// この creep がそのロールをこなせる body を持っているか。
    fn can_fill(&self, role: &str) -> bool {
        match role {
            ROLE_CARRIER_MINERAL | ROLE_HAULER => self.carry > 0,
            ROLE_MINER => self.work > 0,
            _ => self.work > 0 && self.carry > 0,
        }
    }
}

pub fn creep_loop() {
    // ロール別の実数。旧実装は7個の個別変数だったが、目標との差分で配分するには
    // 名前で引ける形の方が扱いやすい。
    let mut role_counts: HashMap<String, i32> = HashMap::new();

    let mut opt_num_attackable_short: i32 = 0;
    let mut opt_num_attackable_long: i32 = 0;

    let mut cap_worker_carry: u128 = 0;

    // ロール構成の判断材料は tick 内で不変なので 1 回だけ観測する。
    let colony = ColonyState::observe();
    let total_creeps = colony.total_creeps as usize;

    // creep ごとの情報を1回だけ集める。
    let mut roster: Vec<CreepInfo> = Vec::with_capacity(total_creeps);

    for creep in game::creeps().values() {
        let name = creep.name();
        let cmem = creep.memory();

        let role = cmem
            .string("role")
            .ok()
            .flatten()
            .unwrap_or_else(|| String::from("none"));

        // body の走査はここ1回だけ。攻撃能力とパーツ数を同時に数える。
        let mut work = 0;
        let mut carry = 0;
        let mut attacker_kind = AttackerKind::NONE;
        for part in creep.body().iter() {
            if part.hits() == 0 {
                continue;
            }
            match part.part() {
                Part::Work => work += 1,
                Part::Carry => carry += 1,
                Part::Attack => {
                    if attacker_kind == AttackerKind::NONE {
                        attacker_kind = AttackerKind::SHORT;
                    }
                }
                Part::RangedAttack => attacker_kind = AttackerKind::RANGED,
                _ => {}
            }
        }

        match attacker_kind {
            AttackerKind::SHORT => opt_num_attackable_short += 1,
            AttackerKind::RANGED => opt_num_attackable_long += 1,
            AttackerKind::NONE => {}
        }

        if role != "none" {
            *role_counts.entry(role.clone()).or_insert(0) += 1;
            if role == ROLE_HARVESTER || role == ROLE_HARVESTER_SPAWN {
                cap_worker_carry += creep.store().get_capacity(None) as u128;
            }
        }

        roster.push(CreepInfo {
            creep,
            name,
            role,
            attacker_kind,
            work,
            carry,
        });
    }

    // 採取係が枯渇したら全ロールを白紙に戻して再編成する。
    //
    // 旧実装はロールを消すだけでカウンタを据え置いていたため、直後の再割り当てが
    // 「creepのロールはゼロなのにカウンタは満室」という状態で走り、upgrader も
    // builder も repairer も充足済みと誤判定してスキップ、cap_worker_carry も
    // 古い大きな値のままで harvester が1体も作られず、残り全員が catch-all の
    // repairer に落ちていた。採取係が枯渇した瞬間に全creepが最も重い委譲チェーンを
    // 走るという、最悪のタイミングで最悪の挙動になっていた。
    // ロールを消すならカウンタも同じ地点まで巻き戻す。
    let harvester_total = role_counts.get(ROLE_HARVESTER).copied().unwrap_or(0)
        + role_counts.get(ROLE_HARVESTER_SPAWN).copied().unwrap_or(0);
    if (harvester_total <= 2) && (total_creeps > harvester_total as usize) {
        warn!("harvesters depleted; resetting all roles");
        for info in roster.iter_mut() {
            info.creep.memory().del("role");
            info.role = String::from("none");
        }

        role_counts.clear();
        cap_worker_carry = 0;
    }

    // 目標に寄せる再配分。
    //
    // 新規 creep への割り当てだけでは、いったんできた偏りが直らない。総数が減っても
    // 過剰なロールはそのまま残るので、余っているロールから足りないロールへ
    // 少しずつ移す。1 tick に動かすのは1体だけにしてハンチングを避ける。
    {
        let targets = role_targets(&colony);

        let mut moved = 0;
        for (role, target) in targets.iter() {
            if moved >= ROLE_REBALANCE_PER_TICK {
                break;
            }
            let current = role_counts.get(*role).copied().unwrap_or(0);
            if current >= *target {
                continue;
            }

            // 最も余っているロールを探す。
            let surplus_role = targets
                .iter()
                .filter(|(r, t)| role_counts.get(*r).copied().unwrap_or(0) > *t)
                .max_by_key(|(r, t)| role_counts.get(*r).copied().unwrap_or(0) - t)
                .map(|(r, _)| *r);

            let Some(surplus_role) = surplus_role else {
                break;
            };

            // 余っているロールの creep を1体、こなせるなら移す。
            // 集めておいたロスターから選ぶので、ここで body も Memory も読み直さない。
            let victim = roster
                .iter_mut()
                .find(|i| i.role == surplus_role && i.can_fill(role));

            if let Some(victim) = victim {
                info!("rebalance {}: {} -> {}", victim.name, surplus_role, role);
                victim.creep.memory().set("role", *role);
                // 配達途中の状態を持ち越さない。
                victim.creep.memory().del("target_pos");
                victim.creep.memory().del("target_pos_count");
                victim.role = role.to_string();

                *role_counts.entry(surplus_role.to_string()).or_insert(0) -= 1;
                *role_counts.entry(role.to_string()).or_insert(0) += 1;
                moved += 1;
            }
        }
    }

    for info in roster.iter() {
        let creep = &info.creep;
        let name = info.name.clone();
        debug!("running creep {}, cpu:{}", name, game::cpu::get_used());

        // memory は JS の getter なので 1 回だけ取って使い回す。
        let cmem = creep.memory();

        let mut harvest_kind: ResourceKind = ResourceKind::ENERGY;
        let mut is_harvester = false;

        let mut role_string = info.role.clone();
        let attacker_kind = &info.attacker_kind;

        if role_string == "none" {
            let targets = role_targets(&colony);
            role_string = pick_role(&creep, &targets, &role_counts);

            cmem.set("role", role_string.as_str());
            *role_counts.entry(role_string.clone()).or_insert(0) += 1;

            if role_string == ROLE_HARVESTER || role_string == ROLE_HARVESTER_SPAWN {
                cap_worker_carry += creep.store().get_capacity(None) as u128;
            }
        }

        // 採取ロールかどうか。
        // BUG-5: 旧実装は harvester_spawn がこの match から漏れており、
        // is_harvester == false のまま「storage優先 → active source 後回し」という
        // harvester とは逆の探索順で動いていた。storage も container も無い
        // RCL1〜2 では、最初の3体が全員この経路で空スキャンを毎回払っていた。
        is_harvester = matches!(
            role_string.as_str(),
            ROLE_HARVESTER | ROLE_HARVESTER_SPAWN | ROLE_HARVESTER_MINERAL
        );

        if role_string == ROLE_HARVESTER_MINERAL {
            harvest_kind = ResourceKind::MINELALS;
        }

        info!("role:{:?}:atk:{:?}", role_string, attacker_kind);

        if creep.spawning() {
            continue;
        }

        //// atacker check.
        // 戻り値は「移動を消費したか」。攻撃しただけなら経済の仕事も続ける。
        if *attacker_kind != AttackerKind::NONE {
            let moved_to_enemy = attacker_routine(creep, attacker_kind);

            if moved_to_enemy {
                continue;
            }
        }

        //// harvest resrouce kind.
        if role_string == String::from("harvester_mineral")
            || role_string == String::from("carrier_mineral")
        {
            harvest_kind = ResourceKind::MINELALS;
        }

        // 配達できる荷 (今のロールが扱う資源) の量。
        //
        // 旧実装は採取フェーズへの復帰条件に get_used_capacity(None)、つまり
        // 「全資源が空か」を使っていた。ロール変更でミネラルを抱えたままエネルギー系の
        // ロールになったcreepは、配達先をエネルギーでしか探さないので必ず失敗し、
        // かつ荷が残っているので採取フェーズにも戻れず、寿命が尽きるまで毎tick
        // 全室スキャンを回し続けるデッドロックに陥っていた。
        // 判定を「今のロールで配達できる荷があるか」に変える。
        let store = creep.store();
        let deliverable: u32 = make_resoucetype_list(&harvest_kind)
            .iter()
            .map(|rt| store.get_used_capacity(Some(*rt)))
            .sum();
        let total_used = store.get_used_capacity(None);
        let free_capacity = store.get_free_capacity(None);

        if cmem.bool("harvesting") {
            if (free_capacity == 0)
                || ((cmem.bool("nothing_to_harvest")) && (deliverable > 0))
            {
                cmem.set("harvesting", false);
                cmem.del("target_pos");
                cmem.del("will_harvest_from_storage");
                cmem.del("nothing_to_harvest");
            }
        } else {
            if deliverable == 0 {
                // 配達できる荷が無いなら採取に戻る。ただし配達できない荷で満杯だと
                // 採取もできないので、その場合だけ捨てて詰まりを解消する。
                if total_used > 0 && free_capacity == 0 {
                    for resource in store.store_types() {
                        if !check_resouce_type_kind_matching(&resource, &harvest_kind) {
                            warn!(
                                "{} is stuck with undeliverable {:?}; dropping",
                                name, resource
                            );
                            let _ = creep.drop(resource, None);
                            break;
                        }
                    }
                }

                cmem.set("harvesting", true);
                cmem.del("target_pos");
                cmem.del("harvested_from_storage");
                cmem.del("harvested_from_terminal");
                cmem.del("harvested_from_link");
                cmem.del("nothing_to_harvest");
            }
        }

        // 採取先が無いと分かった直後は、再探索を数tick止める。
        // 探索はフルで走ると全室スキャン6回 + 経路探索3回に達するため、
        // 状況が変わらないうちに毎tick繰り返すのは純粋な浪費になる。
        if cmem.bool("harvesting") && cmem.bool("nothing_to_harvest") {
            let retry_at = creep
                .memory()
                .i32("harvest_retry_at")
                .unwrap_or(Some(0))
                .unwrap_or(0);
            if (game::time() as i32) < retry_at {
                debug!("{} waiting for harvest retry at {}", name, retry_at);
                continue;
            }
            cmem.del("harvest_retry_at");
        }

        if cmem.bool("harvesting") {
            debug!("harvesting {}", name);

            let check_string = cmem.string("target_pos");
            debug!("harvesting string{:?}", check_string);

            let mut defined_target_pos = creep.pos();
            let mut path_search_result;

            match check_string {
                Ok(v) => {
                    match v {
                        Some(v) => {
                            let defined_target_obj: Result<Position, serde_json::Error> =
                                serde_json::from_str(v.as_str());

                            match defined_target_obj {
                                Ok(object) => {
                                    defined_target_pos = object;
                                    debug!("harvesting decided:{}", defined_target_pos);
                                    path_search_result = find_path(&creep, &defined_target_pos, 0);
                                    debug!(
                                        "harvesting decided path:{:?}",
                                        path_search_result.path()
                                    );

                                    // ターゲット座標のある部屋を見る。旧実装は creep が
                                    // 今いる部屋を見ていたため、他室のターゲットに対して
                                    // 自室の同じ座標を調べてしまい、誤検知 (自室にたまたま
                                    // creep がいると「取られた」と誤判定) と検知漏れ
                                    // (他室の本当のターゲット上の creep を見逃す) の
                                    // 両方が起きていた。
                                    let look_result = game::rooms()
                                        .get(defined_target_pos.room_name())
                                        .map(|target_room| {
                                            target_room.look_for_at_xy(
                                                look::CREEPS,
                                                defined_target_pos.x().u8(),
                                                defined_target_pos.y().u8(),
                                            )
                                        })
                                        .unwrap_or_default();

                                    for one_result in look_result {
                                        if one_result.name() != creep.name() {
                                            debug!("re-check source :{}", defined_target_pos);
                                            cmem.del("target_pos");

                                            let reset_result = reset_source_target(
                                                &creep,
                                                is_harvester,
                                                &harvest_kind,
                                            );
                                            path_search_result = reset_result.0;
                                            defined_target_pos = reset_result.1;

                                            break;
                                        }
                                    }
                                }

                                Err(_err) => {
                                    //ロードに成功して値もあったけどDeSerializeできなかった.
                                    let reset_result =
                                        reset_source_target(&creep, is_harvester, &harvest_kind);
                                    path_search_result = reset_result.0;
                                    defined_target_pos = reset_result.1;
                                }
                            }
                        }

                        None => {
                            //ロードに成功したけど値がない.
                            let reset_result =
                                reset_source_target(&creep, is_harvester, &harvest_kind);
                            path_search_result = reset_result.0;
                            defined_target_pos = reset_result.1;
                        }
                    }
                }

                //ロードに失敗(key自体がない).
                Err(_err) => {
                    let reset_result = reset_source_target(&creep, is_harvester, &harvest_kind);
                    path_search_result = reset_result.0;
                    defined_target_pos = reset_result.1;
                }
            }

            let mut is_harvested = false;
            let resource_type_list = make_resoucetype_list(&harvest_kind);

            // check dropped source.
            let resources = &creep
                .room()
                .expect("room is not visible to you")
                .find(find::DROPPED_RESOURCES, None);

            for resource in resources.iter() {
                if creep.pos().is_near_to(resource.pos())
                    && check_resouce_type_kind_matching(&resource.resource_type(), &harvest_kind)
                {
                    if let Err(r) = creep.pickup(resource) {
                        warn!("couldn't pick-up dropped resrouces: {:?}", r);
                        continue;
                    }
                    is_harvested = true;
                    break;
                }
            }

            // check ruins.
            if is_harvested == false {
                let ruins = &creep
                    .room()
                    .expect("room is not visible to you")
                    .find(find::RUINS, None);

                for ruin in ruins.iter() {
                    if creep.pos().is_near_to(ruin.pos()) {
                        for resource_type in resource_type_list.iter() {
                            if ruin.store().get_used_capacity(Some(*resource_type)) > 0 {
                                if let Err(r) = creep.withdraw(ruin, *resource_type, None) {
                                    warn!("couldn't withdraw from RUINs: {:?}", r);
                                    break;
                                }
                                is_harvested = true;
                                break;
                            }
                        }
                    }

                    if is_harvested == true {
                        break;
                    }
                }
            }

            // check tombstones.
            if is_harvested == false {
                let tombstones = &creep
                    .room()
                    .expect("room is not visible to you")
                    .find(find::TOMBSTONES, None);

                for tombstone in tombstones.iter() {
                    if creep.pos().is_near_to(tombstone.pos()) {
                        for resource_type in resource_type_list.iter() {
                            if tombstone.store().get_used_capacity(Some(*resource_type)) > 0 {
                                if let Err(r) = creep.withdraw(tombstone, *resource_type, None) {
                                    warn!("couldn't withdraw from TOMBSTONES: {:?}", r);
                                    break;
                                }
                                is_harvested = true;
                                break;
                            }
                        }
                    }

                    if is_harvested == true {
                        break;
                    }
                }
            }

            //  check sources active.
            if is_harvested == false && harvest_kind == ResourceKind::ENERGY {
                let sources = &creep
                    .room()
                    .expect("room is not visible to you")
                    .find(find::SOURCES_ACTIVE, None);

                for source in sources.iter() {
                    if creep.pos().is_near_to(source.pos()) {
                        if let Err(r) = creep.harvest(source) {
                            warn!("couldn't harvest from ActiveSource: {:?}", r);
                            continue;
                        }
                        is_harvested = true;
                        break;
                    }
                }
            }

            if is_harvested == false && harvest_kind == ResourceKind::MINELALS {
                let sources = &creep
                    .room()
                    .expect("room is not visible to you")
                    .find(find::MINERALS, None);

                for source in sources.iter() {
                    if creep.pos().is_near_to(source.pos()) {
                        match creep.harvest(source) {
                            Ok(()) | Err(HarvestErrorCode::Tired) => {
                                // Tired は採掘クールダウン中なのでその場維持 (旧実装と同じ扱い).
                            }
                            Err(r) => {
                                info!("couldn't harvest from Minerals: {:?}", r);
                                continue;
                            }
                        }
                        is_harvested = true;
                        break;
                    }
                }
            }

            //  storage.
            if is_harvested == false && cmem.bool("will_harvest_from_storage") == true {
                let structures = &creep
                    .room()
                    .expect("room is not visible to you")
                    .find(find::STRUCTURES, None);

                for structure in structures.iter() {
                    if creep.pos().is_near_to(structure.pos()) {
                        for resource_type in resource_type_list.iter() {
                            if check_stored(structure, &resource_type, 0) {
                                match structure {
                                    StructureObject::StructureContainer(container) => {
                                        if let Err(r) =
                                            creep.withdraw(container, *resource_type, None)
                                        {
                                            warn!("couldn't withdraw from container: {:?}", r);
                                            break;
                                        }
                                        cmem.set("harvested_from_storage", true);
                                        is_harvested = true;
                                        break;
                                    }

                                    StructureObject::StructureStorage(storage) => {
                                        if let Err(r) =
                                            creep.withdraw(storage, *resource_type, None)
                                        {
                                            warn!("couldn't withdraw from storage: {:?}", r);
                                            break;
                                        }
                                        cmem.set("harvested_from_storage", true);
                                        is_harvested = true;
                                        break;
                                    }

                                    StructureObject::StructureTerminal(terminal) => {
                                        if harvest_kind == ResourceKind::ENERGY {
                                            if terminal
                                                .store()
                                                .get_used_capacity(Some(*resource_type))
                                                > TERMINAL_KEEP_ENERGY
                                            {
                                                if let Err(r) = creep.withdraw(
                                                    terminal,
                                                    *resource_type,
                                                    Some(std::cmp::min(
                                                        terminal
                                                            .store()
                                                            .get_used_capacity(Some(
                                                                *resource_type,
                                                            ))
                                                            - TERMINAL_KEEP_ENERGY,
                                                        creep.store().get_free_capacity(None)
                                                            as u32,
                                                    )),
                                                ) {
                                                    warn!(
                                                        "couldn't withdraw from terminal: {:?}",
                                                        r
                                                    );
                                                    break;
                                                }
                                                cmem.set("harvested_from_terminal", true);
                                                is_harvested = true;
                                                break;
                                            }
                                        }
                                    }

                                    StructureObject::StructureLink(link) => {
                                        if let Err(r) = creep.withdraw(link, *resource_type, None) {
                                            warn!("couldn't withdraw from link: {:?}", r);
                                            break;
                                        }
                                        cmem.set("harvested_from_link", true);
                                        is_harvested = true;
                                        break;
                                    }

                                    StructureObject::StructureLab(lab) => {
                                        if harvest_kind == ResourceKind::MINELALS {
                                            if let Err(r) =
                                                creep.withdraw(lab, *resource_type, None)
                                            {
                                                warn!("couldn't withdraw from lab: {:?}", r);
                                                break;
                                            }
                                            cmem.set("harvested_from_storage", true);
                                            is_harvested = true;
                                            break;
                                        }
                                    }

                                    _ => {
                                        //do nothing
                                    }
                                }
                            }
                        }

                        if is_harvested == true {
                            break;
                        }
                    }
                }
            }

            if is_harvested == false {
                if creep.pos() == defined_target_pos {
                    debug!("already arrived, but can't harvest!!!");
                    cmem.del("target_pos");
                } else {
                    let res = move_by_search_result(&creep, &path_search_result);

                    if let Err(e) = res {
                        info!("couldn't move to source: {:?}", e);
                        // move_by_path は経路が使えない (creep が経路上にいない等) とき
                        // NotFound を返す。旧 move_to の NoPath に相当するので、
                        // 同じくターゲットを捨てて次tickに選び直す。
                        if e == CreepMoveByPathErrorCode::NotFound {
                            cmem.del("target_pos");
                        }
                    }
                }

                let mut target_pos_count = creep
                    .memory()
                    .i32("target_pos_count")
                    .unwrap_or(Some(10))
                    .unwrap_or(10);
                target_pos_count -= 1;
                if target_pos_count <= 0 {
                    cmem.del("target_pos");
                    cmem.del("target_pos_count");
                } else {
                    cmem.set("target_pos_count", target_pos_count);
                }
            }
        } else {
            debug!("TASK role:{:?}", role_string);

            let sources = &creep
                .room()
                .expect("room is not visible to you")
                .find(find::SOURCES_ACTIVE, None);

            let mut is_finished = false;

            let flee_count = creep
                .memory()
                .i32("fleeing_count")
                .unwrap_or(Some(0))
                .unwrap_or(0);

            if flee_count <= 0 {
                for source in sources.iter() {
                    if creep.pos().is_near_to(source.pos()) {
                        info!("fleeing from source!!");

                        let result = find_flee_path_from_active_source(&creep);
                        debug!(
                            "fleeing from source!!:{},{},{:?}",
                            result.ops(),
                            result.cost(),
                            result.path()
                        );

                        let res = move_by_search_result(&creep, &result);
                        debug!("fleeing from source!!:{:?}", res);

                        if res.is_ok() {
                            cmem.set("fleeing_count", 5);
                            is_finished = true;
                        }

                        break;
                    }
                }
            } else {
                cmem.set("fleeing_count", flee_count - 1);
            }

            if is_finished {
                continue;
            }

            match role_string.as_str() {
                "miner" => {
                    miner::run_miner(&creep);
                }

                "hauler" => {
                    hauler::run_hauler(&creep);
                }

                "harvester" => {
                    harvester::run_harvester(&creep);
                }

                "harvester_spawn" => {
                    harvester::run_harvester_spawn(&creep);
                }

                "harvester_mineral" => {
                    harvester::run_harvester_mineral(&creep);
                }

                "carrier_mineral" => {
                    harvester::run_carrier_mineral(&creep);
                }

                "worker" => {
                    worker::run_worker(&creep);
                }

                // 旧ロール名。生きている creep が持っている間は worker として扱う。
                "builder" | "upgrader" | "repairer" => {
                    worker::run_worker(&creep);
                }

                "attacker" => {}

                "none" => {
                    error!("no role info");
                }

                &_ => {
                    error!("no role info");
                }
            }
        }
    }

    // check number of each type creeps.
    let root = mem::root();
    let n = |role: &str| role_counts.get(role).copied().unwrap_or(0);
    root.set("num_worker", n(ROLE_WORKER));
    root.set("num_harvester", n(ROLE_HARVESTER));
    root.set("num_harvester_spawn", n(ROLE_HARVESTER_SPAWN));
    root.set("num_harvester_mineral", n(ROLE_HARVESTER_MINERAL));
    root.set("num_carrier_mineral", n(ROLE_CARRIER_MINERAL));


    root.set("opt_num_attackable_short", opt_num_attackable_short);
    root.set("opt_num_attackable_long", opt_num_attackable_long);

    root.set("total_num", total_creeps as i32);
    root.set("cap_worker_carry", cap_worker_carry as i32);
}
