//! 経路探索とコストマトリクス。
//!
//! コストマトリクスは静的層 (地形・建物、TTL 付きキャッシュ) と
//! 動的層 (creep の位置・敵の危険域、毎 tick) の2層で組む。

use super::cache::{room_structures, room_terrain};
use super::predicates::check_my_structure;
use crate::mem::MemoryExt;
use lazy_static::lazy_static;
use log::*;
use screeps::local::{RoomName, RoomXY};
use screeps::pathfinder::{
    search, MultiRoomCostResult, SearchGoal, SearchOptions, SearchResults,
};
use screeps::prelude::*;
use screeps::{game, find, CostMatrix, LocalCostMatrix, Part, StructureType, Terrain};
use std::collections::HashMap;
use std::sync::RwLock;

const MY_CREEP_COST: u8 = 8;

/// 座り仕事ロール (miner / 専任 upgrader) のいるマスのコスト。
/// 彼らは待っても退かないので、強めのコストで迂回を促す。
/// 255 (通行不可) にはしない: 唯一の通路に座られた場合でも経路自体は
/// 組めるようにし、到達不能扱いで max_ops を使い切るのを避ける。
const SEATED_CREEP_COST: u8 = 40;

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

lazy_static! {
    static ref MAP_CACHE: RwLock<Data> = RwLock::new(HashMap::new());
}

pub fn move_by_search_result(
    creep: &screeps::objects::Creep,
    res: &SearchResults,
) -> Result<(), screeps::action_error_codes::CreepMoveByPathErrorCode> {
    creep.move_by_path(res.opaque_path().as_ref())
}

pub fn empty_search(creep: &screeps::objects::Creep) -> SearchResults {
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

/// creep の現在室 + 見えている他室から `find` した結果を連結して返す.

pub fn default_search_options() -> SearchOptions<fn(RoomName) -> MultiRoomCostResult> {
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

pub fn search_goals<T: HasPosition>(list: &[(T, u32)]) -> Vec<SearchGoal> {
    list.iter()
        .map(|(item, range)| SearchGoal::new(item.pos(), *range))
        .collect()
}

// 完成コスト層 (静的+動的) の tick 内キャッシュ。
// pathfinder は探索のたびにこのコールバックを呼ぶので、素直に作ると
// 同 tick 内に creep の数だけ動的層を組み直す。部屋の状態 (creep の位置・
// 敵) は tick 内では変わらないため、1回組んで使い回す。
lazy_static! {
    static ref TICK_CACHE: RwLock<Data> = RwLock::new(HashMap::new());
}

fn calc_room_cost(room_name: RoomName) -> MultiRoomCostResult {
    let Some(room_obj) = game::rooms().get(room_name) else {
        // 視界の無い部屋は既定コストのまま。
        return MultiRoomCostResult::Default;
    };

    let now = game::time();
    {
        let cache = TICK_CACHE.read().unwrap();
        if let Some((built_at, matrix)) = cache.get(&room_name) {
            if *built_at == now {
                return MultiRoomCostResult::CostMatrix(CostMatrix::from(matrix.clone()));
            }
        }
    }

    let mut cost_matrix = static_layer(&room_obj, room_name);

    // ここから動的層。tick ごとに作り直す。
    apply_dynamic_layer(&room_obj, &mut cost_matrix);

    TICK_CACHE
        .write()
        .unwrap()
        .insert(room_name, (now, cost_matrix.clone()));

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

    // creep のいるマス。敵は本当の障害物。味方はロールで扱いを分ける:
    // - 座り仕事 (miner / 専任 upgrader) は退かないので強い迂回誘導
    // - 道路上の歩行 creep は素通り扱い。一本道での正面衝突は双方が直進
    //   すれば同 tick のすれ違い (swap) で解決するのに、コストを載せると
    //   道路から降りて迂回し、道路特化 body (MOVE 半減) は倍の tick を払う
    //   (実測: 対向のたびに片方が降りてタイムロス)
    // - それ以外 (平地の歩行 creep) は従来どおり軽い回避
    for creep in room_obj.find(find::CREEPS, None) {
        if creep.my() {
            let xy = creep.pos().xy();
            let cur = cost_matrix.get(xy);
            if cur >= 0xff {
                continue;
            }
            let role = creep
                .memory()
                .string(crate::mem::keys::ROLE)
                .ok()
                .flatten();
            let seated = role.as_deref().is_some_and(|r| {
                r == crate::creeps::ROLE_MINER || r == crate::creeps::ROLE_UPGRADER
            });
            if seated {
                cost_matrix.set(xy, cur.max(SEATED_CREEP_COST));
            } else if !is_road[xy_index(xy)] {
                cost_matrix.set(xy, cur.max(MY_CREEP_COST));
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


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 危険コストは254で飽和し通行不可の255に達しない() {
        assert_eq!(danger_cost(0), 10);
        assert_eq!(danger_cost(244), 254);
        assert_eq!(danger_cost(250), 254);
        assert_eq!(danger_cost(254), 254);
    }
}
