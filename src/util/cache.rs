//! tick 単位のキャッシュ。
//!
//! JS 境界を越える取得 (find / getTerrain / メモリ) は高くつくので、
//! 「同じ問いは 1 tick に 1 回だけ尋ね、答えを Rust 側で使い回す」。
//! すべて `clear_init_flag()` が tick 先頭で破棄する。

use super::predicates::check_repairable;
use screeps::enums::StructureObject;
use screeps::local::RoomName;
use screeps::prelude::*;
use screeps::{find, game, LocalRoomTerrain, StructureType};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

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
    /// tick 内で共有する部屋ごとの RCL。詳細は `room_rcl()`。
    static RCL_CACHE: RefCell<HashMap<RoomName, u8>> = RefCell::new(HashMap::new());
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

pub fn find_all_rooms<T>(
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
pub fn all_structures() -> Rc<Vec<StructureObject>> {
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
}

pub fn work_summary() -> Rc<WorkSummary> {
    WORK_SUMMARY_CACHE.with(|cache| {
        if let Some(cached) = cache.borrow().as_ref() {
            return Rc::clone(cached);
        }

        let mut has_construction = false;
        let mut has_repair_target = false;

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


                if has_repair_target {
                    break;
                }
            }

            if has_construction && has_repair_target {
                break;
            }
        }

        let summary = Rc::new(WorkSummary {
            has_construction,
            has_repair_target,
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

pub fn clear_init_flag() {
    crate::creeps::clear_colony_cache();
    super::traffic::clear_traffic();
    STRUCTURE_CACHE.with(|cache| *cache.borrow_mut() = None);
    ROOM_STRUCTURE_CACHE.with(|cache| cache.borrow_mut().clear());
    HOSTILE_CACHE.with(|cache| cache.borrow_mut().clear());
    TERRAIN_CACHE.with(|cache| cache.borrow_mut().clear());
    WORK_SUMMARY_CACHE.with(|cache| *cache.borrow_mut() = None);
    RCL_CACHE.with(|cache| cache.borrow_mut().clear());

    // MAP_CACHE (静的コスト層) は TTL で管理するのでここでは消さない。
    super::stats::clear_stats_caches();
}


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

/// 部屋の RCL (tick 単位でキャッシュ)。
///
/// Rampart / Wall の修理目標 (`barrier_target_hp`) の判定は構造物ループの
/// 内側から呼ばれる。rooms().get → controller → level は JS 呼び出しの
/// 連鎖なので、部屋あたり 1 tick 1回に抑える。
/// controller の無い部屋・見えない部屋は 0。
pub fn room_rcl(room_name: RoomName) -> u8 {
    RCL_CACHE.with(|cache| {
        if let Some(cached) = cache.borrow().get(&room_name) {
            return *cached;
        }

        let level = game::rooms()
            .get(room_name)
            .and_then(|room| room.controller())
            .map(|controller| controller.level())
            .unwrap_or(0);
        cache.borrow_mut().insert(room_name, level);
        level
    })
}
