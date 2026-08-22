use crate::constants::*;
use log::*;
use screeps::constants::*;
use screeps::enums::StructureObject;
use screeps::local::{Position, RoomName, RoomXY};
use screeps::objects::{ConstructionSite, Resource, RoomPosition, Source};
use screeps::pathfinder::{
    search, search_many, MultiRoomCostResult, SearchGoal, SearchOptions, SearchResults,
};
use screeps::prelude::*;
use screeps::look::LookResult;
use screeps::{game, CostMatrix, LocalCostMatrix, LocalRoomTerrain};

use std::cell::RefCell;
use std::cmp::*;
use std::rc::Rc;
use std::{collections::HashMap, u32, u8};

use lazy_static::lazy_static;
use std::sync::RwLock;

thread_local! {
    /// tick 内で共有する全可視部屋の建造物リスト。詳細は `all_structures()`。
    static STRUCTURE_CACHE: RefCell<Option<Rc<Vec<StructureObject>>>> = RefCell::new(None);
    /// tick 内で共有する部屋ごとの建造物リスト。詳細は `room_structures()`。
    static ROOM_STRUCTURE_CACHE: RefCell<HashMap<RoomName, Rc<Vec<StructureObject>>>> =
        RefCell::new(HashMap::new());
    /// tick 内で共有する部屋ごとの敵リスト。詳細は `room_hostiles()`。
    static HOSTILE_CACHE: RefCell<HashMap<RoomName, Rc<Vec<screeps::objects::Creep>>>> =
        RefCell::new(HashMap::new());
    /// tick 内で共有する部屋ごとの地形。詳細は `room_terrain()`。
    static TERRAIN_CACHE: RefCell<HashMap<RoomName, Rc<LocalRoomTerrain>>> =
        RefCell::new(HashMap::new());
    /// tick 内で共有する仕事の有無。詳細は `work_summary()`。
    static WORK_SUMMARY_CACHE: RefCell<Option<Rc<WorkSummary>>> = RefCell::new(None);
    /// creep が目指している立ち位置。詳細は `claim_target()`。
    static TARGET_CLAIMS: RefCell<HashMap<Position, u32>> = RefCell::new(HashMap::new());
}

thread_local! {
    /// この探索では味方 creep を通行不可として扱うか。
    ///
    /// 通常は味方を「待てば退く」前提の低コスト (MY_CREEP_COST) で通すが、
    /// その経路は実際には占有マスへ入れず移動が失敗する。1マス幅の袋小路では
    /// 入る creep と出る creep が向かい合い、双方が毎tick同じ経路を計算して
    /// 同じ失敗を繰り返す千日手になる。
    /// 数tick動けなかった creep の探索ではこれを立て、味方を壁として扱わせる。
    /// 迂回路があればそちらへ、無ければ探索が incomplete になりその場で待つ
    /// (少なくとも押し合いはしない。塞いでいる creep は積載が満ちれば退く)。
    static HARD_BLOCK_MY_CREEPS: std::cell::Cell<bool> = std::cell::Cell::new(false);
}

/// スタックした creep の探索モードを設定する。creep の処理の前後で必ず対にして呼ぶ。
pub fn set_hard_block_my_creeps(on: bool) {
    HARD_BLOCK_MY_CREEPS.with(|f| f.set(on));
}

/// 味方 creep のマスに乗せるコスト。
///
/// 旧実装は敵味方を問わず creep のいるマスを 0xff (通行不可) にしていた。
/// source の周りは元々マスが少ないので、2〜3体立つだけでその source が到達不能
/// 扱いになり、経路探索が別の source へ弾かれる。結果として近い source が
/// 空いていても遠い source に creep が集まる、という偏りが起きていた。
///
/// 味方は待てば退くので、避けたいが通れる程度のコストにする。
/// 平地コストが 2 なので、これは「4マスぶん迂回する価値がある」という重み。
const MY_CREEP_COST: u8 = 8;

/// 予約1件あたりの上乗せコスト。
///
/// 当初は定額20だった。これには2つ問題があった。
/// 1. 何体予約していても20のままで、予約が積み重なるほど詰まるという情報が
///    経路探索に伝わらない。
/// 2. creep がすでにその source の周りに固まっている場合、そこから別の source
///    まで歩くコスト (17マスなら34) より予約済みマスに突っ込むコスト (20) の
///    ほうが安く、経路探索は合理的に殺到を選んでしまう。実際に7体が同じ1マスを
///    目指す状況が観測された。
///
/// 件数比例にする。1件で50 (=25マスぶんの迂回に相当) なので、隣接マスに空きが
/// あればそちらへ、その source の空きマスが全て予約済みなら別の source へ逸れる。
/// 上限254で頭打ちにし、全マスが予約済みでも経路自体は見つかるようにする。
const CLAIMED_TARGET_COST_PER_CLAIM: u32 = 50;

/// この立ち位置を目指すことを宣言する。
pub fn claim_target(pos: Position) {
    TARGET_CLAIMS.with(|c| *c.borrow_mut().entry(pos).or_insert(0) += 1);
}

/// 宣言を取り下げる。目的地を選び直す前に呼ぶ。
pub fn release_target(pos: Position) {
    TARGET_CLAIMS.with(|c| {
        let mut m = c.borrow_mut();
        if let Some(n) = m.get_mut(&pos) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                m.remove(&pos);
            }
        }
    });
}

/// このマスを目指している creep がいるか。
pub fn is_target_claimed(pos: Position) -> bool {
    TARGET_CLAIMS.with(|c| c.borrow().contains_key(&pos))
}

/// tick をまたいで持ち越さない。creep_loop が毎tick作り直す。
pub fn clear_target_claims() {
    TARGET_CLAIMS.with(|c| c.borrow_mut().clear());
}

const ROOM_SIZE_X: u8 = 50;
const ROOM_SIZE_Y: u8 = 50;
const ROOM_AREA: usize = (ROOM_SIZE_X as usize) * (ROOM_SIZE_Y as usize);

/// 静的コスト層を作り直す間隔 (tick)。
/// 地形・道路・壁・建設現場はこの程度の間隔で見直せば十分。
const STATIC_LAYER_TTL: u32 = 100;

/// RoomXY を 0..2500 の添字へ。ビットマップ用。
fn xy_index(pos: RoomXY) -> usize {
    (pos.y.u8() as usize) * (ROOM_SIZE_X as usize) + (pos.x.u8() as usize)
}

/// 静的コスト層のキャッシュ。値は (構築した tick, マトリクス)。
type Data = HashMap<RoomName, (u32, LocalCostMatrix)>;

type ConstructionProgressAverage = HashMap<RoomName, u128>;
type StructureHpAverage = HashMap<RoomName, u128>;

type ConstructionProgressMin = HashMap<RoomName, u128>;
type StructureHpMin = HashMap<RoomName, u128>;

lazy_static! {
    static ref MAP_CACHE: RwLock<Data> = RwLock::new(HashMap::new());
    static ref CONSTRUCTION_PROGRESS_AVERAGE_CACHE: RwLock<ConstructionProgressAverage> =
        RwLock::new(HashMap::new());
    static ref STRUCTURE_HP_AVERAGE_CACHE: RwLock<StructureHpAverage> = RwLock::new(HashMap::new());
    static ref CONSTRUCTION_PROGRESS_MIN_CACHE: RwLock<ConstructionProgressMin> =
        RwLock::new(HashMap::new());
    static ref STRUCTURE_HP_MIN_CACHE: RwLock<StructureHpMin> = RwLock::new(HashMap::new());
}

/// 0.23 で消えた `creep.move_by_path_search_result()` の代替。
///
/// 当初は経路の1歩目に対して `creep.move_to()` を呼んでいたが、`move_to` は内部で
/// 独自に経路探索をやり直すため、`search_many` で高価に求めた経路を捨てて
/// もう一度探索していた (1 creep 1 tick あたり探索2回)。
///
/// `SearchResults::opaque_path()` は JS 配列をそのまま返すので、Rust 側の
/// `Vec<Position>` 変換も挟まずに `move_by_path` へ渡せる。探索は1回で済む。
pub fn move_by_search_result(
    creep: &screeps::objects::Creep,
    res: &SearchResults,
) -> Result<(), screeps::action_error_codes::CreepMoveByPathErrorCode> {
    creep.move_by_path(res.opaque_path().as_ref())
}

/// 探索対象が1つも無いときに返す「空の探索結果」。
///
/// ゴールが空のまま `search_many` を呼ぶと、PathFinder は目標を見つけられないまま
/// 既定の 2000 ops を必ず使い切る。creep 数 × 探索回数だけこれが起きるため、
/// 何もしないのに数万 ops を溶かしていた。
/// 自分の現在地を範囲0のゴールにすると即座に (ops ほぼ0で) 空経路が返る。
/// 呼び出し側は一律 `path().len() > 0` で判定しているので挙動も変わらない。
pub fn empty_search_for(creep: &screeps::objects::Creep) -> SearchResults {
    empty_search(creep)
}

fn empty_search(creep: &screeps::objects::Creep) -> SearchResults {
    search(creep.pos(), creep.pos(), 0, Some(default_search_options()))
}

fn xy(x: u8, y: u8) -> RoomXY {
    RoomXY::checked_new(x, y).expect("coordinate out of range")
}

/// 敵の射程圏内のマスに危険度を上乗せする。
///
/// 旧実装は `cur_cost + 10` をそのまま代入していた。`cur_cost < 0xff` のガードしか
/// ないため cur_cost が 246 以上だと u8 を越え、release ビルド (overflow-checks off)
/// ではラップして「最も危険なマスが最も安いマス」に反転していた。
/// 254 で頭打ちにする (255 は通行不可の予約値なので使わない)。
fn danger_cost(cur_cost: u8) -> u8 {
    cur_cost.saturating_add(10).min(254)
}

/// 指定座標に自分の Extractor があるか。
/// 座標が属する部屋を引いてから look するので、他室のミネラルでも正しく判定できる。
fn has_my_extractor_at(pos: Position) -> bool {
    let Some(room) = game::rooms().get(pos.room_name()) else {
        // 視界が無い部屋は判定できない。採取先として選ばない。
        return false;
    };

    room.look_for_at_xy(look::STRUCTURES, pos.x().u8(), pos.y().u8())
        .iter()
        .any(|s| s.structure_type() == StructureType::Extractor && check_my_structure(s))
}

/// creep の現在室 + 見えている他室から `find` した結果を連結して返す.
/// find 定数は Copy ではないためクロージャで都度生成する.
fn find_all_rooms<T>(
    creep: &screeps::objects::Creep,
    make_ty: impl Fn() -> T,
) -> Vec<T::Item>
where
    T: find::FindConstant,
{
    let home = creep.room().expect("room is not visible to you");
    let mut item_list = home.find(make_ty(), None);

    for room_item in game::rooms().values() {
        if room_item.name() != home.name() {
            item_list.extend(room_item.find(make_ty(), None));
        }
    }

    item_list
}

/// 全可視部屋の建造物リスト (tick 単位でキャッシュ)。
///
/// `find(STRUCTURES)` はこの AI で最も多用され、かつ最も重い呼び出し。
/// `find_nearest_*` の内側から creep ごと・探索ごとに呼ばれるため、creep 14 体規模で
/// 1 tick に数百回に達していた。返るオブジェクトは 1 個ずつ `structureType` getter を
/// 通るので、JS 境界越えは数万回になる。
///
/// 連結結果は creep によらず同じなので、tick 内で 1 回だけ作って共有する。
/// `Rc` で返すのは、呼び出しごとに数百個の JS ハンドルを clone しないため。
/// キャッシュは `clear_init_flag()` が tick 先頭で捨てる。
fn all_structures() -> Rc<Vec<StructureObject>> {
    STRUCTURE_CACHE.with(|cache| {
        if let Some(cached) = cache.borrow().as_ref() {
            return Rc::clone(cached);
        }

        let mut list = Vec::new();
        for room in game::rooms().values() {
            list.extend(room_structures(&room).iter().cloned());
        }

        let list = Rc::new(list);
        *cache.borrow_mut() = Some(Rc::clone(&list));
        list
    })
}

/// 可視範囲にどんな仕事が存在するかの要約。tick 単位でキャッシュする。
///
/// ロールの委譲チェーン (harvester -> builder -> repairer -> upgrader) は、
/// 各段が「自分の仕事があるか」を確かめるために `find_nearest_*` を呼んでいた。
/// これは PathFinder 探索を伴うため、仕事が無いと確認するだけで 1 creep あたり
/// 最大11回の探索が走っていた。しかも spawn と extension が満タンで建設現場も
/// 無い平和な状態こそが定常状態なので、全 creep が毎tickこれを払っていた。
///
/// 有無の判定だけなら、既にキャッシュ済みの構造物リストを1回舐めれば足りる。
pub struct WorkSummary {
    /// 建設現場があるか。
    pub has_construction: bool,
    /// 修理対象 (壁を除く) があるか。
    pub has_repair_target: bool,
    /// エネルギーの搬入先 (extension / spawn / tower に空きがある) があるか。
    pub has_energy_sink: bool,
}

pub fn work_summary() -> Rc<WorkSummary> {
    WORK_SUMMARY_CACHE.with(|cache| {
        if let Some(cached) = cache.borrow().as_ref() {
            return Rc::clone(cached);
        }

        let mut has_construction = false;
        let mut has_repair_target = false;
        let mut has_energy_sink = false;

        for room in game::rooms().values() {
            if !has_construction && !room.find(find::MY_CONSTRUCTION_SITES, None).is_empty() {
                has_construction = true;
            }

            for st in room_structures(&room).iter() {
                if !has_repair_target
                    && st.structure_type() != StructureType::Wall
                    && check_repairable(st)
                {
                    has_repair_target = true;
                }

                if !has_energy_sink
                    && matches!(
                        st.structure_type(),
                        StructureType::Extension | StructureType::Spawn | StructureType::Tower
                    )
                    && check_transferable(st, &ResourceType::Energy, None)
                {
                    has_energy_sink = true;
                }

                if has_repair_target && has_energy_sink {
                    break;
                }
            }

            if has_construction && has_repair_target && has_energy_sink {
                break;
            }
        }

        let summary = Rc::new(WorkSummary {
            has_construction,
            has_repair_target,
            has_energy_sink,
        });
        *cache.borrow_mut() = Some(Rc::clone(&summary));
        summary
    })
}

/// 1部屋分の敵 creep リスト (tick 単位でキャッシュ)。
///
/// 旧実装は creep ごとに `find(HOSTILE_CREEPS)` を呼んでいた。敵の顔ぶれは tick 内で
/// 変わらないので、部屋あたり1回で足りる。
pub fn room_hostiles(room: &screeps::objects::Room) -> Rc<Vec<screeps::objects::Creep>> {
    let name = room.name();

    HOSTILE_CACHE.with(|cache| {
        if let Some(cached) = cache.borrow().get(&name) {
            return Rc::clone(cached);
        }

        let list = Rc::new(room.find(find::HOSTILE_CREEPS, None));
        cache.borrow_mut().insert(name, Rc::clone(&list));
        list
    })
}

/// 1部屋分の建造物リスト (tick 単位でキャッシュ)。
///
/// 「自室だけを見たい」呼び出し側 (tower の修理対象選び、harvester の補給先探し、
/// spawn のエネルギー集計など) 用。同じ部屋を 1 tick に何度も find し直すのを防ぐ。
/// 特に `run_harvester_spawn` は同一関数内で 2 回 find していた。
pub fn room_structures(room: &screeps::objects::Room) -> Rc<Vec<StructureObject>> {
    let name = room.name();

    ROOM_STRUCTURE_CACHE.with(|cache| {
        if let Some(cached) = cache.borrow().get(&name) {
            return Rc::clone(cached);
        }

        let list = Rc::new(room.find(find::STRUCTURES, None));
        cache.borrow_mut().insert(name, Rc::clone(&list));
        list
    })
}

fn default_search_options() -> SearchOptions<fn(RoomName) -> MultiRoomCostResult> {
    SearchOptions::new(calc_room_cost as fn(RoomName) -> MultiRoomCostResult)
        .plain_cost(2)
        .swamp_cost(10)
        // 既定は 2000。到達不能なゴールを狙うと毎回これを使い切るので絞る。
        // 1部屋内の移動なら 500 で十分足りる。
        .max_ops(500)
        // 既定は 16 部屋。この AI は隣接部屋までしか扱わない。
        .max_rooms(2)
        // plain_cost を 2 にしているのに既定の 1.2 のままだと、ヒューリスティックが
        // 実コストの半分程度に過小評価され、A* がほぼ Dijkstra 化して ops を浪費する。
        // 平地コストに合わせる。
        .heuristic_weight(2.0)
}

fn search_goals<T: HasPosition>(list: &[(T, u32)]) -> Vec<SearchGoal> {
    list.iter()
        .map(|(item, range)| SearchGoal::new(item.pos(), *range))
        .collect()
}

pub fn clear_init_flag() {
    crate::creeps::clear_colony_cache();
    clear_target_claims();
    STRUCTURE_CACHE.with(|cache| *cache.borrow_mut() = None);
    ROOM_STRUCTURE_CACHE.with(|cache| cache.borrow_mut().clear());
    HOSTILE_CACHE.with(|cache| cache.borrow_mut().clear());
    TERRAIN_CACHE.with(|cache| cache.borrow_mut().clear());
    WORK_SUMMARY_CACHE.with(|cache| *cache.borrow_mut() = None);

    // MAP_CACHE (静的コスト層) は TTL で管理するのでここでは消さない。
    // 毎 tick 消すと部屋あたりの find(STRUCTURES) が毎 tick 走ってしまう。

    let mut construction_progress_average = CONSTRUCTION_PROGRESS_AVERAGE_CACHE.write().unwrap();
    construction_progress_average.clear();

    let mut structure_hp_average = STRUCTURE_HP_AVERAGE_CACHE.write().unwrap();
    structure_hp_average.clear();

    let mut construction_progress_min = CONSTRUCTION_PROGRESS_MIN_CACHE.write().unwrap();
    construction_progress_min.clear();

    let mut structure_hp_min = STRUCTURE_HP_MIN_CACHE.write().unwrap();
    structure_hp_min.clear();
}

#[derive(PartialEq, Debug)]
pub enum ResourceKind {
    ENERGY,
    MINELALS,
    POWER,
    COMMODITIES,
}

pub fn calc_average(room_name: &RoomName) {
    let mut construction_progress_average = CONSTRUCTION_PROGRESS_AVERAGE_CACHE.write().unwrap();
    let mut structure_hp_average = STRUCTURE_HP_AVERAGE_CACHE.write().unwrap();

    let mut construction_progress_min = CONSTRUCTION_PROGRESS_MIN_CACHE.write().unwrap();
    let mut structure_hp_min = STRUCTURE_HP_MIN_CACHE.write().unwrap();

    let room = game::rooms().get(*room_name);

    match room {
        Some(room_obj) => {
            let structures = room_obj.find(find::STRUCTURES, None);
            let construction_sites = room_obj.find(find::MY_CONSTRUCTION_SITES, None);

            let mut total_hp: u128 = 0;
            let mut hp_min: u128 = 0;

            let mut struct_count: u128 = 0;

            for chk_struct in structures {
                let cur_hp = get_hp(&chk_struct);

                match cur_hp {
                    Some(hp) => {
                        struct_count += 1 as u128;
                        total_hp += hp as u128;

                        if (hp_min > hp as u128) || (hp_min == 0) {
                            hp_min = hp as u128;
                        }
                    }
                    None => {}
                }
            }

            let mut sum_of_progress: u128 = 0;
            let mut progress_min: u128 = 0;
            let mut construction_count: u128 = 0;

            for construction_site in construction_sites.iter() {
                let left_progress = construction_site.progress_total() as u128
                    - construction_site.progress() as u128;
                sum_of_progress += left_progress;
                construction_count += 1;

                if (progress_min > left_progress) || (progress_min == 0) {
                    progress_min = left_progress;
                }
            }

            if struct_count > 0 {
                structure_hp_average.insert(*room_name, total_hp / struct_count);

                structure_hp_min.insert(*room_name, hp_min);
                info!(
                    "{:?}: structure_hp_average:{:?}/min:{:?}",
                    room_name,
                    total_hp / struct_count,
                    hp_min
                );
            } else {
                structure_hp_average.insert(*room_name, 0);
                structure_hp_min.insert(*room_name, 0);
            }

            if construction_count > 0 {
                construction_progress_average
                    .insert(*room_name, sum_of_progress / construction_count);
                construction_progress_min.insert(*room_name, progress_min);

                info!(
                    "{:?}: construction_progress_average:{:?}:min:{:?}",
                    *room_name,
                    sum_of_progress / construction_count,
                    progress_min
                );
            } else {
                construction_progress_average.insert(*room_name, 0);
                // MIN 側にも入れないと、下の get_construction_progress_average が
                // 常にキャッシュミス扱いになり calc_average を毎回呼び直してしまう。
                construction_progress_min.insert(*room_name, 0);
            }
        }

        None => {}
    }
}

pub fn get_hp_average(room_name: &RoomName) -> (u128, u128) {
    {
        let structure_hp_average = STRUCTURE_HP_AVERAGE_CACHE.read().unwrap();
        let cache_value = structure_hp_average.get(&room_name);

        let structure_hp_min = STRUCTURE_HP_MIN_CACHE.read().unwrap();
        let cache_value_min = structure_hp_min.get(&room_name);

        match cache_value {
            Some(value) => {
                // use cached value.

                match cache_value_min {
                    Some(value_min) => {
                        return (*value, *value_min);
                    }

                    None => {}
                }
            }
            None => {}
        }
    }

    calc_average(room_name);

    {
        let structure_hp_average = STRUCTURE_HP_AVERAGE_CACHE.read().unwrap();
        let cache_value = structure_hp_average.get(&room_name);

        let structure_hp_min = STRUCTURE_HP_MIN_CACHE.read().unwrap();
        let cache_value_min = structure_hp_min.get(&room_name);

        match cache_value {
            Some(value) => {
                // use cached value.

                match cache_value_min {
                    Some(value_min) => {
                        return (*value, *value_min);
                    }

                    None => {}
                }
            }
            None => {}
        }
    }

    return (0, 0);
}

pub fn get_construction_progress_average(room_name: &RoomName) -> (u128, u128) {
    {
        let construction_progress_average = CONSTRUCTION_PROGRESS_AVERAGE_CACHE.read().unwrap();
        let cache_value = construction_progress_average.get(&room_name);

        // MIN キャッシュを読む。旧実装は2箇所とも AVERAGE を読んでおり、
        // MIN キャッシュは書かれるだけの dead store になっていた。その結果
        // builder の閾値 (stats.0 + stats.1) / 2 が「平均と最小の中間」ではなく
        // 単なる平均になり、建設優先度の制御が設計どおり効いていなかった。
        let construction_progress_min = CONSTRUCTION_PROGRESS_MIN_CACHE.read().unwrap();
        let cache_value_min = construction_progress_min.get(&room_name);

        match cache_value {
            Some(value) => {
                // use cached value.

                match cache_value_min {
                    Some(value_min) => {
                        return (*value, *value_min);
                    }

                    None => {}
                }
            }
            None => {}
        }
    }

    calc_average(room_name);

    {
        let construction_progress_average = CONSTRUCTION_PROGRESS_AVERAGE_CACHE.read().unwrap();
        let cache_value = construction_progress_average.get(&room_name);

        // MIN キャッシュを読む。旧実装は2箇所とも AVERAGE を読んでおり、
        // MIN キャッシュは書かれるだけの dead store になっていた。その結果
        // builder の閾値 (stats.0 + stats.1) / 2 が「平均と最小の中間」ではなく
        // 単なる平均になり、建設優先度の制御が設計どおり効いていなかった。
        let construction_progress_min = CONSTRUCTION_PROGRESS_MIN_CACHE.read().unwrap();
        let cache_value_min = construction_progress_min.get(&room_name);

        match cache_value {
            Some(value) => {
                // use cached value.

                match cache_value_min {
                    Some(value_min) => {
                        return (*value, *value_min);
                    }

                    None => {}
                }
            }
            None => {}
        }
    }

    return (0, 0);
}

/// 部屋のコストマトリクスを組み立てる。
///
/// 中身は寿命の異なる2層に分かれる。
///   静的層: 地形 / 道路 / 壁 / ランパート / 建設現場 — 数百 tick 変わらない
///   動的層: creep の位置 / 敵の危険域 — 毎 tick 変わる
/// 旧実装は両方まとめて毎 tick 再構築していた。静的層を tick をまたいで保持すれば、
/// 部屋あたりの `find(STRUCTURES)` と `find(MY_CONSTRUCTION_SITES)` が
/// STATIC_LAYER_TTL に 1 回で済む。
fn calc_room_cost(room_name: RoomName) -> MultiRoomCostResult {
    let Some(room_obj) = game::rooms().get(room_name) else {
        // 視界の無い部屋は既定コストのまま。
        return MultiRoomCostResult::Default;
    };

    let mut cost_matrix = static_layer(&room_obj, room_name);

    // ここから動的層。tick ごとに作り直す。
    apply_dynamic_layer(&room_obj, &mut cost_matrix);

    MultiRoomCostResult::CostMatrix(CostMatrix::from(cost_matrix))
}

/// 静的層 (地形・構造物・建設現場・source周辺) を組み立てる。TTL 付きでキャッシュする。
fn static_layer(room_obj: &screeps::objects::Room, room_name: RoomName) -> LocalCostMatrix {
    let now = game::time();

    {
        let cache = MAP_CACHE.read().unwrap();
        if let Some((built_at, matrix)) = cache.get(&room_name) {
            if now.saturating_sub(*built_at) < STATIC_LAYER_TTL {
                return matrix.clone();
            }
        }
    }

    debug!("Room:{}, rebuilding static cost layer.", room_name);

    let mut cost_matrix = LocalCostMatrix::default();

    // 地形を wasm 線形メモリへ写して以降の参照を JS 呼び出しなしにする。
    let terrain = room_terrain(room_obj);

    // 道路とランパートの位置をビットマップ化しておく。
    // 旧実装は内側ループで同じマスに対し look_for_at_xy を2回呼んでいた。
    let mut is_road = vec![false; ROOM_AREA];
    let structures = room_structures(room_obj);

    for chk_struct in structures.iter() {
        let pos_xy = chk_struct.pos().xy();
        let idx = xy_index(pos_xy);

        if chk_struct.structure_type() == StructureType::Road {
            is_road[idx] = true;
            // Favor roads over plain tiles
            cost_matrix.set(pos_xy, 1);
        } else if chk_struct.structure_type() != StructureType::Container
            && (chk_struct.structure_type() != StructureType::Rampart
                || check_my_structure(chk_struct) == false)
        {
            // Can't walk through non-walkable buildings
            cost_matrix.set(pos_xy, 0xff);
        }
    }

    // ConstructionSiteの通行不可なものをマーク.
    for construction_site in room_obj.find(find::MY_CONSTRUCTION_SITES, None) {
        if construction_site.structure_type() != StructureType::Road
            && construction_site.structure_type() != StructureType::Container
            && construction_site.structure_type() != StructureType::Rampart
        {
            // Can't walk through non-walkable construction sites.
            cost_matrix.set(construction_site.pos().xy(), 0xff);
        }
    }

    // active sourceの周辺はコストをあげる (採取待ちで詰まりやすいため).
    for chk_item in room_obj.find(find::SOURCES_ACTIVE, None).iter() {
        let (sx, sy) = (chk_item.pos().x().u8() as i8, chk_item.pos().y().u8() as i8);

        for dx in -1..=1i8 {
            for dy in -1..=1i8 {
                let nx = (sx + dx).clamp(0, ROOM_SIZE_X as i8 - 1) as u8;
                let ny = (sy + dy).clamp(0, ROOM_SIZE_Y as i8 - 1) as u8;
                let new_xy = xy(nx, ny);

                // すでに通行不可としてマークされているマスは触らない.
                if cost_matrix.get(new_xy) == 0xff {
                    continue;
                }

                if terrain.get_xy(new_xy) != Terrain::Wall {
                    cost_matrix.set(new_xy, 11);
                } else if is_road[xy_index(new_xy)] {
                    //Road かつ Wall.
                    cost_matrix.set(new_xy, 2);
                }
            }
        }
    }

    {
        let mut cache = MAP_CACHE.write().unwrap();
        cache.insert(room_name, (now, cost_matrix.clone()));
    }

    cost_matrix
}

/// 動的層 (creep の位置と敵の危険域) を重ねる。
fn apply_dynamic_layer(room_obj: &screeps::objects::Room, cost_matrix: &mut LocalCostMatrix) {
    let terrain = room_terrain(room_obj);

    // 自分のランパートの位置をビットマップ化。危険域の判定で使う。
    let mut has_my_rampart = vec![false; ROOM_AREA];
    let mut is_road = vec![false; ROOM_AREA];
    for s in room_structures(room_obj).iter() {
        let idx = xy_index(s.pos().xy());
        match s.structure_type() {
            StructureType::Rampart => {
                if check_my_structure(s) {
                    has_my_rampart[idx] = true;
                }
            }
            StructureType::Road => is_road[idx] = true,
            _ => {}
        }
    }

    // 他の creep がすでに目指しているマスを避ける。予約数に比例して重くする。
    TARGET_CLAIMS.with(|claims| {
        for (pos, count) in claims.borrow().iter() {
            if pos.room_name() != room_obj.name() {
                continue;
            }
            let xy = pos.xy();
            let cur = cost_matrix.get(xy);
            if cur < 0xff {
                let add = (count * CLAIMED_TARGET_COST_PER_CLAIM).min(250) as u8;
                cost_matrix.set(xy, cur.saturating_add(add).min(254));
            }
        }
    });

    // creep のいるマス。敵は本当の障害物だが、味方は待てば退く。
    for creep in room_obj.find(find::CREEPS, None) {
        if creep.my() {
            let xy = creep.pos().xy();
            if HARD_BLOCK_MY_CREEPS.with(|f| f.get()) {
                // スタック中の creep の探索。味方を壁として扱い、実際に通れる
                // 迂回路だけを返させる。
                cost_matrix.set(xy, 0xff);
            } else {
                let cur = cost_matrix.get(xy);
                if cur < 0xff {
                    cost_matrix.set(xy, cur.max(MY_CREEP_COST));
                }
            }
            continue;
        }

        cost_matrix.set(creep.pos().xy(), 0xff);

        // enemyの射程圏内は、Rampartが無い限りコストをあげる.
        // 旧実装は match で上書きし続けていたため body の最後に現れた
        // 攻撃パーツが勝ち、[RangedAttack, Attack] の順だとレンジャーを
        // 射程1と誤認していた。最大値を採る。
        let enemy_range: i8 = creep
            .body()
            .iter()
            .filter(|bp| bp.hits() > 0)
            .map(|bp| match bp.part() {
                Part::RangedAttack => 3,
                Part::Attack => 1,
                _ => 0,
            })
            .max()
            .unwrap_or(1)
            .max(1);

        // 半径 r を中心に置くには -r..=r を回す。旧実装は 2r を入れた変数で
        // 0..=2r を回して -2r していたため、危険域が敵の左上にだけ広がり
        // 右下は完全にノーマークだった。
        let (ex, ey) = (creep.pos().x().u8() as i8, creep.pos().y().u8() as i8);

        for dx in -enemy_range..=enemy_range {
            for dy in -enemy_range..=enemy_range {
                let nx = (ex + dx).clamp(0, ROOM_SIZE_X as i8 - 1) as u8;
                let ny = (ey + dy).clamp(0, ROOM_SIZE_Y as i8 - 1) as u8;
                let new_xy = xy(nx, ny);
                let idx = xy_index(new_xy);

                let cur_cost = cost_matrix.get(new_xy);
                // すでに通行不可としてマークされているマスは触らない.
                if cur_cost == 0xff {
                    continue;
                }
                // 自分のランパートの下は安全なので上乗せしない。
                if has_my_rampart[idx] {
                    continue;
                }

                // 壁の上は道路があるときだけ通れる。
                if terrain.get_xy(new_xy) == Terrain::Wall && !is_road[idx] {
                    continue;
                }

                cost_matrix.set(new_xy, danger_cost(cur_cost));
            }
        }
    }
}


pub fn check_walkable(position: &RoomPosition) -> bool {
    let chk_room = game::rooms().get(position.room_name());

    if let Some(room) = chk_room {
        let objects = room.look_at(position);

        for object in objects {
            match object {
                LookResult::Creep(_creep) => {
                    return false;
                }

                LookResult::Terrain(terrain) => {
                    if terrain == Terrain::Wall {
                        return false;
                    }
                }

                LookResult::Structure(structure) => {
                    let structure: StructureObject = structure.into();
                    if structure.structure_type() != StructureType::Container
                        && (structure.structure_type() != StructureType::Rampart
                            || check_my_structure(&structure) == false)
                    {
                        return false;
                    }
                }

                _ => {
                    // check next.
                }
            }
        }
    }

    return true;
}

pub fn check_my_structure(structure: &StructureObject) -> bool {
    match structure.as_owned() {
        Some(my_structure) => {
            return my_structure.my();
        }

        None => {
            //not my structure.
            return false;
        }
    }
}

pub fn check_transferable(
    structure: &StructureObject,
    resource_type: &ResourceType,
    capacity_rate: Option<f64>,
) -> bool {
    match structure.as_owned() {
        Some(my_structure) => {
            if my_structure.my() == false {
                return false;
            }

            match structure.as_transferable() {
                Some(_transf) => {
                    match structure.as_has_store() {
                        Some(has_store) => {
                            if has_store.store().get_free_capacity(Some(*resource_type))
                                > (has_store.store().get_capacity(Some(*resource_type)) as f64
                                    * capacity_rate.unwrap_or(0 as f64))
                                    as i32
                            {
                                return true;
                            }
                        }

                        None => {
                            //no store.
                        }
                    }
                }

                None => {
                    // my_struct is not transferable
                }
            }
        }

        None => {
            match structure.as_transferable() {
                Some(_transf) => {
                    match structure.as_has_store() {
                        Some(has_store) => {
                            if has_store.store().get_free_capacity(Some(*resource_type)) > 0 {
                                return true;
                            }
                        }

                        None => {
                            //no store.
                        }
                    }
                }

                None => {
                    // my_struct is not transferable
                }
            }
        }
    }

    return false;
}

pub fn check_repairable(structure: &StructureObject) -> bool {
    match structure.as_owned() {
        Some(my_structure) => {
            if my_structure.my() == false {
                return false;
            }

            match structure.as_attackable() {
                Some(attackable) => {
                    if attackable.hits() < attackable.hits_max() {
                        if attackable.hits() > 0 {
                            return true;
                        }
                    }
                }

                None => {
                    // my_struct is not attackable.
                }
            }
        }

        None => {
            match structure.as_attackable() {
                Some(attackable) => {
                    if attackable.hits() < attackable.hits_max() {
                        if attackable.hits() > 0 {
                            return true;
                        }
                    }
                }

                None => {
                    // my_struct is not attackable.
                }
            }
        }
    }
    return false;
}

pub fn get_repairable_hp(structure: &StructureObject) -> Option<u32> {
    match structure.as_owned() {
        Some(my_structure) => {
            if my_structure.my() == false {
                return None;
            }

            match structure.as_attackable() {
                Some(attackable) => {
                    if attackable.hits() > 0 {
                        return Some(attackable.hits_max() - attackable.hits());
                    } else {
                        return None;
                    }
                }

                None => {
                    // my_struct is not attackable.
                }
            }
        }

        None => {
            match structure.as_attackable() {
                Some(attackable) => {
                    if attackable.hits() > 0 {
                        return Some(attackable.hits_max() - attackable.hits());
                    } else {
                        return None;
                    }
                }

                None => {
                    // my_struct is not attackable.
                }
            }
        }
    }
    return None;
}

fn live_tickcount_from_kind(
    structure: &StructureObject,
    attackable_hits: u32,
    this_terrain: Terrain,
) -> Option<u128> {
    match structure {
        StructureObject::StructureRoad(_road) => match this_terrain {
            Terrain::Plain => {
                return Some(
                    ROAD_DECAY_TIME as u128 * (attackable_hits as u128 / ROAD_DECAY_AMOUNT as u128),
                );
            }
            Terrain::Swamp => {
                return Some(
                    ROAD_DECAY_TIME as u128
                        * (attackable_hits as u128
                            / (ROAD_DECAY_AMOUNT as u128
                                * CONSTRUCTION_COST_ROAD_SWAMP_RATIO as u128)),
                );
            }
            Terrain::Wall => {
                return Some(
                    ROAD_DECAY_TIME as u128
                        * (attackable_hits as u128
                            / (ROAD_DECAY_AMOUNT as u128
                                * CONSTRUCTION_COST_ROAD_WALL_RATIO as u128)),
                );
            }
        },

        StructureObject::StructureContainer(_container) => {
            return Some(
                CONTAINER_DECAY_TIME_OWNED as u128 * (attackable_hits as u128 / CONTAINER_DECAY as u128),
            );
        }

        StructureObject::StructureRampart(_ramport) => {
            return Some(
                RAMPART_DECAY_TIME as u128 * (attackable_hits as u128 / RAMPART_DECAY_AMOUNT as u128),
            );
        }

        _ => {}
    }

    None
}

/// 部屋の地形 (wasm 側にコピー済み) を tick 単位でキャッシュする。
///
/// `get_live_tickcount` は構造物1個ごとに `room.get_terrain()` を呼んでいた。
/// 1500 構造物なら 1500 回の JS 呼び出し + オブジェクト生成になる。
/// `LocalRoomTerrain` は 2500 バイトを wasm 線形メモリへ写すので、部屋あたり
/// 1 回変換すれば以降の参照は JS 呼び出しゼロで済む。
pub fn room_terrain(room: &screeps::objects::Room) -> Rc<LocalRoomTerrain> {
    let name = room.name();

    TERRAIN_CACHE.with(|cache| {
        if let Some(cached) = cache.borrow().get(&name) {
            return Rc::clone(cached);
        }

        let terrain: Rc<LocalRoomTerrain> = Rc::new(room.get_terrain().into());
        cache.borrow_mut().insert(name, Rc::clone(&terrain));
        terrain
    })
}

pub fn get_live_tickcount(structure: &StructureObject) -> Option<u128> {
    let room_obj = structure
        .as_structure()
        .room()
        .expect("room is not visible to you");
    let terrain = room_terrain(&room_obj);

    match structure.as_owned() {
        Some(my_structure) => {
            if my_structure.my() == false {
                return None;
            }

            match structure.as_attackable() {
                Some(attackable) => {
                    let this_terrain = terrain.get_xy(structure.pos().xy());

                    return live_tickcount_from_kind(structure, attackable.hits(), this_terrain);
                }

                None => {
                    // my_struct is not attackable.
                }
            }
        }

        None => {
            match structure.as_attackable() {
                Some(attackable) => {
                    let this_terrain = terrain.get_xy(structure.pos().xy());

                    return live_tickcount_from_kind(structure, attackable.hits(), this_terrain);
                }

                None => {
                    // my_struct is not attackable.
                }
            }
        }
    }
    return None;
}

pub fn get_hp(structure: &StructureObject) -> Option<u32> {
    match structure.as_owned() {
        Some(my_structure) => {
            if my_structure.my() == false {
                return None;
            }

            match structure.as_attackable() {
                Some(attackable) => {
                    if (attackable.hits() > 0) && (attackable.hits() < attackable.hits_max()) {
                        return Some((attackable.hits()) as u32);
                    } else {
                        return None;
                    }
                }

                None => {
                    // my_struct is not attackable.
                }
            }
        }

        None => {
            match structure.as_attackable() {
                Some(attackable) => {
                    if (attackable.hits() > 0) && (attackable.hits() < attackable.hits_max()) {
                        return Some((attackable.hits()) as u32);
                    } else {
                        return None;
                    }
                }

                None => {
                    // my_struct is not attackable.
                }
            }
        }
    }
    return None;
}

pub fn check_repairable_hp(structure: &StructureObject, hp_th: u32) -> bool {
    match structure.as_owned() {
        Some(my_structure) => {
            if my_structure.my() == false {
                return false;
            }

            match structure.as_attackable() {
                Some(attackable) => {
                    if attackable.hits() < attackable.hits_max() {
                        if (attackable.hits() < hp_th) && (attackable.hits() > 0) {
                            return true;
                        }
                    }
                }

                None => {
                    // my_struct is not attackable.
                }
            }
        }

        None => {
            match structure.as_attackable() {
                Some(attackable) => {
                    if attackable.hits() < attackable.hits_max() {
                        if (attackable.hits() < hp_th) && (attackable.hits() > 0) {
                            return true;
                        }
                    }
                }

                None => {
                    // my_struct is not attackable.
                }
            }
        }
    }
    return false;
}

pub fn check_stored(
    structure: &StructureObject,
    resource_type: &ResourceType,
    keep_amount: u32,
) -> bool {
    match structure.as_has_store() {
        Some(storage) => {
            if storage.store().get_used_capacity(Some(*resource_type)) > keep_amount {
                return true;
            }
        }

        None => {}
    }
    return false;
}

/// 資源分類ごとの資源型リスト。
///
/// 旧実装は呼び出しのたびに `vec![]` で最大41要素をヒープ確保していた。
/// dropped resource のループ内からも呼ばれるので、creep 数 × 資源数だけ
/// アロケーションが走っていた。中身は不変なので static スライスにする。
static ENERGY_TYPES: &[ResourceType] = &[ResourceType::Energy];
static POWER_TYPES: &[ResourceType] = &[ResourceType::Power, ResourceType::Ops];
static MINERAL_TYPES: &[ResourceType] = &[ResourceType::Hydrogen,
                ResourceType::Oxygen,
                ResourceType::Utrium,
                ResourceType::Lemergium,
                ResourceType::Keanium,
                ResourceType::Zynthium,
                ResourceType::Catalyst,
                ResourceType::Ghodium,
                ResourceType::Hydroxide,
                ResourceType::ZynthiumKeanite,
                ResourceType::UtriumLemergite,
                ResourceType::UtriumHydride,
                ResourceType::UtriumOxide,
                ResourceType::KeaniumHydride,
                ResourceType::KeaniumOxide,
                ResourceType::LemergiumHydride,
                ResourceType::LemergiumOxide,
                ResourceType::ZynthiumHydride,
                ResourceType::ZynthiumOxide,
                ResourceType::GhodiumHydride,
                ResourceType::GhodiumOxide,
                ResourceType::UtriumAcid,
                ResourceType::UtriumAlkalide,
                ResourceType::KeaniumAcid,
                ResourceType::KeaniumAlkalide,
                ResourceType::LemergiumAcid,
                ResourceType::LemergiumAlkalide,
                ResourceType::ZynthiumAcid,
                ResourceType::ZynthiumAlkalide,
                ResourceType::GhodiumAcid,
                ResourceType::GhodiumAlkalide,
                ResourceType::CatalyzedUtriumAcid,
                ResourceType::CatalyzedUtriumAlkalide,
                ResourceType::CatalyzedKeaniumAcid,
                ResourceType::CatalyzedKeaniumAlkalide,
                ResourceType::CatalyzedLemergiumAcid,
                ResourceType::CatalyzedLemergiumAlkalide,
                ResourceType::CatalyzedZynthiumAcid,
                ResourceType::CatalyzedZynthiumAlkalide,
                ResourceType::CatalyzedGhodiumAcid,
                ResourceType::CatalyzedGhodiumAlkalide,];
static COMMODITY_TYPES: &[ResourceType] = &[ResourceType::Silicon,
                ResourceType::Metal,
                ResourceType::Biomass,
                ResourceType::Mist,
                ResourceType::UtriumBar,
                ResourceType::LemergiumBar,
                ResourceType::ZynthiumBar,
                ResourceType::KeaniumBar,
                ResourceType::GhodiumMelt,
                ResourceType::Oxidant,
                ResourceType::Reductant,
                ResourceType::Purifier,
                ResourceType::Battery,
                ResourceType::Composite,
                ResourceType::Crystal,
                ResourceType::Liquid,
                ResourceType::Wire,
                ResourceType::Switch,
                ResourceType::Transistor,
                ResourceType::Microchip,
                ResourceType::Circuit,
                ResourceType::Device,
                ResourceType::Cell,
                ResourceType::Phlegm,
                ResourceType::Tissue,
                ResourceType::Muscle,
                ResourceType::Organoid,
                ResourceType::Organism,
                ResourceType::Alloy,
                ResourceType::Tube,
                ResourceType::Fixtures,
                ResourceType::Frame,
                ResourceType::Hydraulics,
                ResourceType::Machine,
                ResourceType::Condensate,
                ResourceType::Concentrate,
                ResourceType::Extract,
                ResourceType::Spirit,
                ResourceType::Emanation,
                ResourceType::Essence,];

pub fn resource_types(resource_kind: &ResourceKind) -> &'static [ResourceType] {
    match resource_kind {
        ResourceKind::ENERGY => ENERGY_TYPES,
        ResourceKind::MINELALS => MINERAL_TYPES,
        ResourceKind::COMMODITIES => COMMODITY_TYPES,
        ResourceKind::POWER => POWER_TYPES,
    }
}

/// 旧 API 互換。呼び出し側が Vec を期待している箇所のため残す。
pub fn make_resoucetype_list(resource_kind: &ResourceKind) -> Vec<ResourceType> {
    resource_types(resource_kind).to_vec()
}

pub fn check_resouce_type_kind_matching(
    resource_type: &ResourceType,
    resource_kind: &ResourceKind,
) -> bool {
    resource_types(resource_kind).contains(resource_type)
}

pub fn find_nearest_transfarable_item(
    creep: &screeps::objects::Creep,
    resource_kind: &ResourceKind,
    is_except_storages: &bool,
    is_except_terminal: &bool,
    is_except_link: &bool,
) -> SearchResults {
    let item_list = all_structures();

    let mut find_item_list = Vec::<(StructureObject, u32)>::new();
    let resource_type_list = make_resoucetype_list(resource_kind);

    // creep が実際に持っている資源型だけに絞る。
    // 旧実装は構造物ごと・資源型ごとに creep.store() を呼び直しており、
    // 鉱物 (41種) × 構造物 500 個で 41,000 回の JS 境界越えが発生していた。
    // ループ不変なのでここで一度だけ確定させる。
    let store = creep.store();
    let held_types: Vec<ResourceType> = resource_type_list
        .iter()
        .copied()
        .filter(|rt| store.get_used_capacity(Some(*rt)) > 0)
        .collect();

    if held_types.is_empty() {
        return empty_search(creep);
    }

    for chk_item in item_list.iter() {
        if chk_item.structure_type() == StructureType::Lab
            && *resource_kind == ResourceKind::MINELALS
        {
            continue;
        }

        if *is_except_storages == true
            && (chk_item.structure_type() == StructureType::Container
                || chk_item.structure_type() == StructureType::Storage)
        {
            //前回storage系からresourceを調達している場合はもどさないようにする.

            continue;
        }

        if *is_except_terminal == true
            && (*resource_kind == ResourceKind::ENERGY
                && chk_item.structure_type() == StructureType::Terminal)
        {
            //前回Terminalからresourceを調達している場合はもどさないようにする.

            continue;
        }

        if *is_except_link == true && (chk_item.structure_type() == StructureType::Link) {
            //前回Linkからresourceを調達している場合はもどさないようにする.

            continue;
        }

        let mut dist = 1;
        if chk_item.structure_type() == StructureType::Container {
            dist = 0;
        }

        for resource_type in held_types.iter() {
            if check_transferable(chk_item, resource_type, None) {
                find_item_list.push((chk_item.clone(), dist));
                break;
            }
        }
    }

    if find_item_list.is_empty() {
        return empty_search(creep);
    }

    let option = default_search_options();

    return search_many(
        creep.pos(),
        search_goals(&find_item_list).into_iter(),
        Some(option),
    );
}

pub fn find_nearest_transfarable_terminal(
    creep: &screeps::objects::Creep,
    resource_kind: &ResourceKind,
) -> SearchResults {
    let item_list = all_structures();

    let mut find_item_list = Vec::<(StructureObject, u32)>::new();
    let resource_type_list = make_resoucetype_list(resource_kind);

    // creep が実際に持っている資源型だけに絞る。
    // 旧実装は構造物ごと・資源型ごとに creep.store() を呼び直しており、
    // 鉱物 (41種) × 構造物 500 個で 41,000 回の JS 境界越えが発生していた。
    // ループ不変なのでここで一度だけ確定させる。
    let store = creep.store();
    let held_types: Vec<ResourceType> = resource_type_list
        .iter()
        .copied()
        .filter(|rt| store.get_used_capacity(Some(*rt)) > 0)
        .collect();

    if held_types.is_empty() {
        return empty_search(creep);
    }

    for chk_item in item_list.iter() {
        if chk_item.structure_type() != StructureType::Terminal {
            //Terminal以外は除外.
            continue;
        }

        let mut dist = 1;
        if chk_item.structure_type() == StructureType::Container {
            dist = 0;
        }

        for resource_type in held_types.iter() {
            if check_transferable(chk_item, resource_type, None) {
                find_item_list.push((chk_item.clone(), dist));
                break;
            }
        }
    }

    if find_item_list.is_empty() {
        return empty_search(creep);
    }

    let option = default_search_options();

    return search_many(
        creep.pos(),
        search_goals(&find_item_list).into_iter(),
        Some(option),
    );
}

pub fn find_nearest_repairable_item_hp(
    creep: &screeps::objects::Creep,
    threshold: u32,
) -> SearchResults {
    let item_list = all_structures();

    let mut find_item_list = Vec::<(StructureObject, u32)>::new();

    for chk_item in item_list.iter() {
        if check_repairable(&chk_item) {
            if get_hp(&chk_item).unwrap_or(0) <= threshold {
                find_item_list.push((chk_item.clone(), 3));
            }
        }
    }

    if find_item_list.is_empty() {
        return empty_search(creep);
    }

    let option = default_search_options();

    return search_many(
        creep.pos(),
        search_goals(&find_item_list).into_iter(),
        Some(option),
    );
}

pub fn find_nearest_repairable_item_except_wall_dying(
    creep: &screeps::objects::Creep,
    threshold: u128,
) -> SearchResults {
    let item_list = all_structures();

    let mut find_item_list = Vec::<(StructureObject, u32)>::new();

    for chk_item in item_list.iter() {
        if chk_item.structure_type() != StructureType::Wall {
            if check_repairable(&chk_item) {
                if get_live_tickcount(&chk_item).unwrap_or(10000) as u128 <= threshold {
                    find_item_list.push((chk_item.clone(), 3));
                }
            }
        }
    }

    if find_item_list.is_empty() {
        return empty_search(creep);
    }

    let option = default_search_options();

    return search_many(
        creep.pos(),
        search_goals(&find_item_list).into_iter(),
        Some(option),
    );
}

pub fn find_nearest_transferable_structure(
    creep: &screeps::objects::Creep,
    structure_type: &StructureType,
    resource_type: &ResourceType,
    max_cost: Option<f64>,
    capacity_rate: Option<f64>,
) -> SearchResults {
    let item_list = all_structures();

    let mut find_item_list = Vec::<(StructureObject, u32)>::new();

    for chk_item in item_list.iter() {
        if chk_item.structure_type() == *structure_type {
            if check_transferable(&chk_item, resource_type, capacity_rate) {
                find_item_list.push((chk_item.clone(), 1));
            }
        }
    }

    if find_item_list.is_empty() {
        return empty_search(creep);
    }

    let option = default_search_options().max_cost(max_cost.unwrap_or(f64::INFINITY));

    return search_many(
        creep.pos(),
        search_goals(&find_item_list).into_iter(),
        Some(option),
    );
}

pub fn find_nearest_construction_site(
    creep: &screeps::objects::Creep,
    threshold: u32,
) -> SearchResults {
    let item_list = find_all_rooms(creep, || find::MY_CONSTRUCTION_SITES);

    let mut find_item_list = Vec::<(ConstructionSite, u32)>::new();

    for chk_item in item_list.iter() {
        if (chk_item.progress_total() - chk_item.progress()) <= threshold {
            find_item_list.push((chk_item.clone(), 3));
        }
    }

    if find_item_list.is_empty() {
        return empty_search(creep);
    }

    let option = default_search_options();

    return search_many(
        creep.pos(),
        search_goals(&find_item_list).into_iter(),
        Some(option),
    );
}

pub fn find_nearest_active_source(
    creep: &screeps::objects::Creep,
    resource_kind: &ResourceKind,
    is_2nd_check: bool,
) -> SearchResults {
    let mut find_item_list = Vec::<(Position, u32)>::new();
    let resource_type_list = make_resoucetype_list(&resource_kind);

    if is_2nd_check == false {
        // dropped resource.
        let item_list = find_all_rooms(creep, || find::DROPPED_RESOURCES);

        for chk_item in item_list.iter() {
            for resource in resource_type_list.iter() {
                if chk_item.resource_type() == *resource {
                    find_item_list.push((chk_item.pos(), 1));
                    break;
                }
            }
        }

        // TOMBSTONES.
        let item_list = find_all_rooms(creep, || find::TOMBSTONES);

        for chk_item in item_list.iter() {
            for resource in resource_type_list.iter() {
                if chk_item.store().get_used_capacity(Some(*resource)) > 0 {
                    find_item_list.push((chk_item.pos(), 1));
                    break;
                }
            }
        }

        // RUINs.
        let item_list = find_all_rooms(creep, || find::RUINS);

        for chk_item in item_list.iter() {
            for resource in resource_type_list.iter() {
                if chk_item.store().get_used_capacity(Some(*resource)) > 0 {
                    find_item_list.push((chk_item.pos(), 1));
                    break;
                }
            }
        }
    }

    if find_item_list.is_empty() {
        if *resource_kind == ResourceKind::ENERGY {
            // active source.
            let item_list = find_all_rooms(creep, || find::SOURCES_ACTIVE);

            for chk_item in item_list.iter() {
                find_item_list.push((chk_item.pos(), 1));
            }
        } else if *resource_kind == ResourceKind::MINELALS {
            // minerals.
            let item_list = find_all_rooms(creep, || find::MINERALS);

            for chk_item in item_list.iter() {
                // ミネラルのある部屋を見る。旧実装は creep が今いる部屋を見ていたため、
                // 他室のミネラル座標を自室で調べてしまい Extractor 判定が丸ごと誤って
                // いた (自室にたまたま Extractor があると、Extractor の無い他室の
                // ミネラルを採取先に選び、歩いて行って harvest に失敗し続ける)。
                if has_my_extractor_at(chk_item.pos()) {
                    find_item_list.push((chk_item.pos(), 1));
                }
            }
        } else if *resource_kind == ResourceKind::COMMODITIES {
            // comodities.
            let item_list = find_all_rooms(creep, || find::DEPOSITS);

            for chk_item in item_list.iter() {
                find_item_list.push((chk_item.pos(), 1));
            }
        } else {
            // power.
            let item_list = all_structures();

            for chk_item in item_list.iter() {
                if chk_item.structure_type() == StructureType::PowerBank {
                    find_item_list.push((chk_item.pos(), 1));
                }
            }
        }
    }

    if find_item_list.is_empty() {
        return empty_search(creep);
    }

    let option = default_search_options();

    return search_many(
        creep.pos(),
        find_item_list
            .into_iter()
            .map(|(pos, range)| SearchGoal::new(pos, range)),
        Some(option),
    );
}

pub fn find_nearest_stored_source(
    creep: &screeps::objects::Creep,
    resource_kind: &ResourceKind,
    is_2nd_check: bool,
) -> SearchResults {
    let mut find_item_list = Vec::<(Position, u32)>::new();
    let resource_type_list = make_resoucetype_list(&resource_kind);

    if is_2nd_check == false {
        // dropped resource.
        let item_list = find_all_rooms(creep, || find::DROPPED_RESOURCES);

        for chk_item in item_list.iter() {
            for resource in resource_type_list.iter() {
                if chk_item.resource_type() == *resource {
                    find_item_list.push((chk_item.pos(), 1));
                    break;
                }
            }
        }

        // TOMBSTONES.
        let item_list = find_all_rooms(creep, || find::TOMBSTONES);

        for chk_item in item_list.iter() {
            for resource in resource_type_list.iter() {
                if chk_item.store().get_used_capacity(Some(*resource)) > 0 {
                    find_item_list.push((chk_item.pos(), 1));
                    break;
                }
            }
        }

        // RUINs.
        let item_list = find_all_rooms(creep, || find::RUINS);

        for chk_item in item_list.iter() {
            for resource in resource_type_list.iter() {
                if chk_item.store().get_used_capacity(Some(*resource)) > 0 {
                    find_item_list.push((chk_item.pos(), 1));
                    break;
                }
            }
        }
    }

    if find_item_list.is_empty() {
        let item_list = all_structures();

        for chk_item in item_list.iter() {
            if chk_item.structure_type() == StructureType::Container
                || chk_item.structure_type() == StructureType::Storage
                || chk_item.structure_type() == StructureType::Link
                || ((chk_item.structure_type() == StructureType::Terminal)
                    && (*resource_kind == ResourceKind::ENERGY))
                || ((chk_item.structure_type() == StructureType::Lab)
                    && (*resource_kind == ResourceKind::MINELALS))
            {
                if check_my_structure(chk_item)
                    || (chk_item.structure_type() == StructureType::Container)
                {
                    for resource_type in resource_type_list.iter() {
                        let mut keep_amount = 0 as u32;
                        if chk_item.structure_type() == StructureType::Terminal {
                            keep_amount = TERMINAL_KEEP_ENERGY;
                        }

                        if check_stored(chk_item, resource_type, keep_amount) {
                            let mut dist = 1;
                            if chk_item.structure_type() == StructureType::Container {
                                dist = 0;
                            }

                            find_item_list.push((chk_item.pos(), dist));
                            break;
                        }
                    }
                }
            }
        }
    }

    if find_item_list.is_empty() {
        return empty_search(creep);
    }

    let option = default_search_options();

    return search_many(
        creep.pos(),
        find_item_list
            .into_iter()
            .map(|(pos, range)| SearchGoal::new(pos, range)),
        Some(option),
    );
}

pub fn find_nearest_exhausted_source(
    creep: &screeps::objects::Creep,
    harvest_kind: &ResourceKind,
) -> SearchResults {
    let mut find_item_list = Vec::<(Position, u32)>::new();

    match harvest_kind {
        ResourceKind::ENERGY => {
            let item_list = find_all_rooms(creep, || find::SOURCES);

            for chk_item in item_list.iter() {
                if (chk_item.energy() == 0) && (chk_item.ticks_to_regeneration().unwrap_or(0) < 50)
                {
                    find_item_list.push((chk_item.pos(), 1));
                }
            }
        }

        ResourceKind::MINELALS => {
            let item_list = find_all_rooms(creep, || find::MINERALS);

            for chk_item in item_list.iter() {
                // ミネラルのある部屋を見る。旧実装は creep が今いる部屋を見ていたため、
                // 他室のミネラル座標を自室で調べてしまい Extractor 判定が丸ごと誤って
                // いた (自室にたまたま Extractor があると、Extractor の無い他室の
                // ミネラルを採取先に選び、歩いて行って harvest に失敗し続ける)。
                if has_my_extractor_at(chk_item.pos()) {
                    find_item_list.push((chk_item.pos(), 1));
                }
            }
        }

        _ => {
            let item_list = find_all_rooms(creep, || find::SOURCES);

            for chk_item in item_list.iter() {
                find_item_list.push((chk_item.pos(), 1));
            }
        }
    }

    if find_item_list.is_empty() {
        return empty_search(creep);
    }

    let option = default_search_options();

    return search_many(
        creep.pos(),
        find_item_list
            .into_iter()
            .map(|(pos, range)| SearchGoal::new(pos, range)),
        Some(option),
    );
}

pub fn find_nearest_dropped_resource(
    creep: &screeps::objects::Creep,
    resource_kind: ResourceKind,
) -> SearchResults {
    let item_list = find_all_rooms(creep, || find::DROPPED_RESOURCES);

    let mut find_item_list = Vec::<(Resource, u32)>::new();
    let resource_type_list = make_resoucetype_list(&resource_kind);

    for chk_item in item_list.iter() {
        for resource_type in resource_type_list.iter() {
            if chk_item.resource_type() == *resource_type {
                find_item_list.push((chk_item.clone(), 1));
                break;
            }
        }
    }

    if find_item_list.is_empty() {
        return empty_search(creep);
    }

    let option = default_search_options();

    return search_many(
        creep.pos(),
        search_goals(&find_item_list).into_iter(),
        Some(option),
    );
}

pub fn find_flee_path_from_active_source(creep: &screeps::objects::Creep) -> SearchResults {
    let item_list = find_all_rooms(creep, || find::SOURCES_ACTIVE);

    let mut find_item_list = Vec::<(Source, u32)>::new();

    for chk_item in item_list.iter() {
        find_item_list.push((chk_item.clone(), 3));
    }

    if find_item_list.is_empty() {
        return empty_search(creep);
    }

    let option = default_search_options().flee(true);

    return search_many(
        creep.pos(),
        search_goals(&find_item_list).into_iter(),
        Some(option),
    );
}

pub fn find_nearest_enemy(creep: &screeps::objects::Creep, range: u32) -> SearchResults {
    let item_list = creep
        .room()
        .expect("room is not visible to you")
        .find(find::HOSTILE_CREEPS, None);

    // not nessesary to find another room hostile_creeps.

    let mut find_item_list = Vec::<(screeps::objects::Creep, u32)>::new();

    for chk_item in item_list.iter() {
        find_item_list.push((chk_item.clone(), range));
    }

    if find_item_list.is_empty() {
        return empty_search(creep);
    }

    let option = default_search_options();

    return search_many(
        creep.pos(),
        search_goals(&find_item_list).into_iter(),
        Some(option),
    );
}

pub fn find_nearest_room_controler(creep: &screeps::objects::Creep) -> SearchResults {
    let item_list = all_structures();

    let mut find_item_list = Vec::<(StructureObject, u32)>::new();

    for chk_item in item_list.iter() {
        if chk_item.structure_type() == StructureType::Controller {
            if check_my_structure(chk_item) == true {
                find_item_list.push((chk_item.clone(), 3));
            }
        }
    }

    if find_item_list.is_empty() {
        return empty_search(creep);
    }

    let option = default_search_options();

    return search_many(
        creep.pos(),
        search_goals(&find_item_list).into_iter(),
        Some(option),
    );
}

pub fn find_path(
    creep: &screeps::objects::Creep,
    target_pos: &Position,
    range: u32,
) -> SearchResults {
    let option = default_search_options();

    return search(creep.pos(), *target_pos, range, Some(option));
}
