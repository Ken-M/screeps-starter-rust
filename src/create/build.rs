//! 建設サイトの自動配置。
//!
//! 何を建てるかは RCL ごとの上限と現状の差分で決め、どこに置くかは建物の種類ごとの
//! 規則で決める。バンカーのような固定ブロックは使わず、地形と既存の建物に合わせて
//! 少しずつ埋めていく (有機的配置)。手動で置いた建物とも共存する。
//!
//! 1 tick に何件も置くと builder が捌けないので、同時進行のサイト数と計画の頻度の
//! 両方で絞る。

use crate::util::*;
use log::*;
use screeps::enums::StructureObject;
use screeps::local::{Position, RoomXY};
use screeps::objects::Room;
use screeps::prelude::*;
use screeps::{find, game, pathfinder, StructureType};
use std::collections::HashSet;

/// 計画を見直す間隔 (tick)。毎tick走らせる必要はない。
const PLANNER_INTERVAL: u32 = 20;

/// 1部屋あたり同時に抱えてよい建設サイト数。
/// 多すぎると builder が分散して1つも完成しない。
/// source が2つある部屋で container を両方置けるだけの余裕は要る。
const MAX_ACTIVE_SITES: usize = 6;

/// 1回の計画で新規に置く上限。
const MAX_NEW_SITES_PER_RUN: usize = 3;

/// spawn からこの範囲までを拠点とみなして extension 等を配置する。
const BASE_RADIUS: i8 = 8;

/// 部屋の縁からこのマス数以内には置かない (出口を塞がないため)。
const EDGE_MARGIN: u8 = 3;

pub fn run_planner() {
    if !game::time().is_multiple_of(PLANNER_INTERVAL) {
        return;
    }

    for room in game::rooms().values() {
        plan_room(&room);
    }
}

fn plan_room(room: &Room) {
    let Some(controller) = room.controller() else {
        return;
    };
    if !controller.my() {
        return;
    }
    let rcl = controller.level() as u32;

    let sites = room.find(find::MY_CONSTRUCTION_SITES, None);
    if sites.len() >= MAX_ACTIVE_SITES {
        debug!("{}: {} sites in progress; hold", room.name(), sites.len());
        return;
    }

    let structures = room_structures(room);

    // 現状の建物数 (完成済み + 建設中) を種類ごとに数える。
    let count_of = |ty: StructureType| -> u32 {
        let built = structures
            .iter()
            .filter(|s| s.structure_type() == ty && is_mine(s))
            .count() as u32;
        let planned = sites.iter().filter(|c| c.structure_type() == ty).count() as u32;
        built + planned
    };

    // 置けないマスの集合を作る。
    let blocked = blocked_tiles(room, &structures, &sites);
    // creep が立つ必要があるので建物を置いてはいけないマス。
    let reserved = reserved_tiles(room);

    let mut budget = std::cmp::min(MAX_NEW_SITES_PER_RUN, MAX_ACTIVE_SITES - sites.len());
    let mut placed_now: HashSet<(u8, u8)> = HashSet::new();

    // --- 1. source 脇の container ---
    // 採取係がその場で降ろせるようにする。採取効率に最も効く。
    if budget > 0 && count_of(StructureType::Container) < allowed(StructureType::Container, rcl) {
        for source in room.find(find::SOURCES, None) {
            if budget == 0 {
                break;
            }
            if has_container_adjacent(source.pos(), &structures, &sites) {
                continue;
            }
            if let Some(xy) = best_adjacent_tile(room, source.pos(), &blocked, &placed_now) {
                if try_place(room, xy, StructureType::Container) {
                    placed_now.insert((xy.x.u8(), xy.y.u8()));
                    budget -= 1;
                }
            }
        }
    }

    // --- 2. 拠点まわりの建物 ---
    // 優先度順。上限に達していない最初の種類を1つずつ置く。
    let base_plan: [(StructureType, u32); 5] = [
        (StructureType::Extension, allowed(StructureType::Extension, rcl)),
        (StructureType::Tower, allowed(StructureType::Tower, rcl)),
        (StructureType::Storage, allowed(StructureType::Storage, rcl)),
        (StructureType::Terminal, allowed(StructureType::Terminal, rcl)),
        (StructureType::Link, allowed(StructureType::Link, rcl)),
    ];

    let Some(spawn_pos) = room.find(find::MY_SPAWNS, None).first().map(|s| s.pos()) else {
        return;
    };

    for (ty, limit) in base_plan.iter() {
        if budget == 0 {
            break;
        }
        if count_of(*ty) >= *limit {
            continue;
        }
        if let Some(xy) = pick_base_tile(spawn_pos, &blocked, &reserved, &placed_now) {
            if try_place(room, xy, *ty) {
                placed_now.insert((xy.x.u8(), xy.y.u8()));
                budget -= 1;
            }
        }
    }

    // --- 3. 重要建造物の rampart ---
    // spawn / tower / storage / terminal のマスに直接張る。被弾はまず
    // rampart が受けるので、本体の HP が減らず safe mode の温存にもつながる
    // (check_safe_mode は重要建造物の HP 減少で発動を判定している)。
    // 対象の並びは check_safe_mode の重要建造物と揃えること。
    if budget > 0 && allowed(StructureType::Rampart, rcl) > 0 {
        for s in structures.iter() {
            if budget == 0 {
                break;
            }
            let critical = matches!(
                s.structure_type(),
                StructureType::Spawn
                    | StructureType::Tower
                    | StructureType::Storage
                    | StructureType::Terminal
            );
            if !critical || !is_mine(s) {
                continue;
            }
            let p = s.pos();
            let (x, y) = (p.x().u8(), p.y().u8());
            if has_rampart_at(x, y, &structures, &sites) || placed_now.contains(&(x, y)) {
                continue;
            }
            let xy = RoomXY::checked_new(x, y).expect("in range");
            if try_place(room, xy, StructureType::Rampart) {
                placed_now.insert((x, y));
                budget -= 1;
            }
        }
    }

    // --- 4. 幹線道路 ---
    // spawn から各 source と controller へ。creep の往復が最も多い経路。
    if budget > 0 {
        plan_roads(room, spawn_pos, &blocked, &mut budget, &mut placed_now);
    }
}

/// そのマスに rampart (完成済み or 建設予定) があるか。
/// rampart は他の建造物と同じマスに重ねて置くため、blocked 判定は使えない。
fn has_rampart_at(
    x: u8,
    y: u8,
    structures: &[StructureObject],
    sites: &[screeps::objects::ConstructionSite],
) -> bool {
    let at = |p: Position| p.x().u8() == x && p.y().u8() == y;

    structures
        .iter()
        .any(|s| s.structure_type() == StructureType::Rampart && at(s.pos()))
        || sites
            .iter()
            .any(|c| c.structure_type() == StructureType::Rampart && at(c.pos()))
}

/// RCL に対するその種類の建設可能数。
fn allowed(ty: StructureType, rcl: u32) -> u32 {
    ty.controller_structures(rcl)
}

fn is_mine(s: &StructureObject) -> bool {
    // 中立の建物 (container / road) は所有者を持たないので、
    // as_owned が None なら自分のものとして数える。
    s.as_owned().map(|o| o.my()).unwrap_or(true)
}

/// すでに何かが建っている / 建設予定のマスと、地形が壁のマス。
fn blocked_tiles(
    room: &Room,
    structures: &[StructureObject],
    sites: &[screeps::objects::ConstructionSite],
) -> HashSet<(u8, u8)> {
    let mut blocked = HashSet::new();

    let terrain = room_terrain(room);
    for x in 0..50u8 {
        for y in 0..50u8 {
            let xy = RoomXY::checked_new(x, y).expect("in range");
            if terrain.get_xy(xy) == screeps::Terrain::Wall {
                blocked.insert((x, y));
            }
        }
    }

    for s in structures.iter() {
        // 道路の上には建てられるが、ここでは単純に避ける。
        let p = s.pos();
        blocked.insert((p.x().u8(), p.y().u8()));
    }
    for c in sites.iter() {
        let p = c.pos();
        blocked.insert((p.x().u8(), p.y().u8()));
    }

    blocked
}

/// creep が立つ必要があるので建物で塞いではいけないマス。
/// source / controller / mineral の隣接と、部屋の縁。
fn reserved_tiles(room: &Room) -> HashSet<(u8, u8)> {
    let mut reserved = HashSet::new();

    let mut reserve_ring = |pos: Position| {
        let (cx, cy) = (pos.x().u8() as i8, pos.y().u8() as i8);
        for dx in -1..=1i8 {
            for dy in -1..=1i8 {
                let x = cx + dx;
                let y = cy + dy;
                if (0..50).contains(&x) && (0..50).contains(&y) {
                    reserved.insert((x as u8, y as u8));
                }
            }
        }
    };

    for s in room.find(find::SOURCES, None) {
        reserve_ring(s.pos());
    }
    for m in room.find(find::MINERALS, None) {
        reserve_ring(m.pos());
    }
    if let Some(c) = room.controller() {
        reserve_ring(c.pos());
    }

    // 部屋の縁。出口を塞ぐと creep が閉じ込められる。
    for x in 0..50u8 {
        for y in 0..50u8 {
            if x < EDGE_MARGIN || y < EDGE_MARGIN || x >= 50 - EDGE_MARGIN || y >= 50 - EDGE_MARGIN
            {
                reserved.insert((x, y));
            }
        }
    }

    reserved
}

fn has_container_adjacent(
    pos: Position,
    structures: &[StructureObject],
    sites: &[screeps::objects::ConstructionSite],
) -> bool {
    let near = |p: Position| p.get_range_to(pos) <= 1;

    structures
        .iter()
        .any(|s| s.structure_type() == StructureType::Container && near(s.pos()))
        || sites
            .iter()
            .any(|c| c.structure_type() == StructureType::Container && near(c.pos()))
}

/// 対象の隣接マスのうち、spawn に最も近い空きマスを選ぶ。
/// 運搬 creep の歩数が最短になる。
fn best_adjacent_tile(
    room: &Room,
    pos: Position,
    blocked: &HashSet<(u8, u8)>,
    placed_now: &HashSet<(u8, u8)>,
) -> Option<RoomXY> {
    let spawn_pos = room.find(find::MY_SPAWNS, None).first().map(|s| s.pos())?;

    let (cx, cy) = (pos.x().u8() as i8, pos.y().u8() as i8);
    let mut best: Option<(u32, RoomXY)> = None;

    for dx in -1..=1i8 {
        for dy in -1..=1i8 {
            if dx == 0 && dy == 0 {
                continue;
            }
            let x = cx + dx;
            let y = cy + dy;
            if !(0..50).contains(&x) || !(0..50).contains(&y) {
                continue;
            }
            let (x, y) = (x as u8, y as u8);
            if blocked.contains(&(x, y)) || placed_now.contains(&(x, y)) {
                continue;
            }

            let xy = RoomXY::checked_new(x, y).ok()?;
            let candidate = Position::new(xy.x, xy.y, room.name());
            let dist = candidate.get_range_to(spawn_pos);

            if best.is_none_or(|(d, _)| dist < d) {
                best = Some((dist, xy));
            }
        }
    }

    best.map(|(_, xy)| xy)
}

/// 拠点内の配置先を選ぶ。
///
/// spawn を中心に外へ広がりながら、市松模様の位置だけを使う。
/// 市松にするのは、埋めなかったマスが通路として残り、creep が拠点内を
/// 斜め移動で抜けられるようにするため。全部埋めると自分の建物で詰まる。
fn pick_base_tile(
    spawn_pos: Position,
    blocked: &HashSet<(u8, u8)>,
    reserved: &HashSet<(u8, u8)>,
    placed_now: &HashSet<(u8, u8)>,
) -> Option<RoomXY> {
    let sx = spawn_pos.x().u8() as i8;
    let sy = spawn_pos.y().u8() as i8;
    // spawn と同じ市松の側を使う。
    let parity = (sx + sy) % 2;

    for radius in 1..=BASE_RADIUS {
        for dx in -radius..=radius {
            for dy in -radius..=radius {
                // その半径の輪の上だけを見る。
                if dx.abs() != radius && dy.abs() != radius {
                    continue;
                }

                let x = sx + dx;
                let y = sy + dy;
                if !(0..50).contains(&x) || !(0..50).contains(&y) {
                    continue;
                }
                if (x + y) % 2 != parity {
                    continue;
                }

                let (x, y) = (x as u8, y as u8);
                if blocked.contains(&(x, y))
                    || reserved.contains(&(x, y))
                    || placed_now.contains(&(x, y))
                {
                    continue;
                }

                if let Ok(xy) = RoomXY::checked_new(x, y) {
                    return Some(xy);
                }
            }
        }
    }

    None
}

/// spawn から source / controller への経路に道路を敷く。
fn plan_roads(
    room: &Room,
    spawn_pos: Position,
    blocked: &HashSet<(u8, u8)>,
    budget: &mut usize,
    placed_now: &mut HashSet<(u8, u8)>,
) {
    let mut goals: Vec<Position> = room.find(find::SOURCES, None).iter().map(|s| s.pos()).collect();
    if let Some(c) = room.controller() {
        goals.push(c.pos());
    }

    for goal in goals {
        if *budget == 0 {
            return;
        }

        let opts = pathfinder::SearchOptions::new(|_: screeps::RoomName| {
            pathfinder::MultiRoomCostResult::Default
        })
        .max_ops(2000)
        .max_rooms(1);

        let res = pathfinder::search(spawn_pos, goal, 1, Some(opts));
        if res.incomplete() {
            continue;
        }

        for step in res.path() {
            if *budget == 0 {
                return;
            }
            let (x, y) = (step.x().u8(), step.y().u8());
            if blocked.contains(&(x, y)) || placed_now.contains(&(x, y)) {
                continue;
            }
            // 平地に道路を敷いても効果は薄いが、沼地は移動コストが5倍なので効く。
            // 全面に敷くと維持費が嵩むため、沼地だけに絞る。
            let terrain = room_terrain(room);
            let xy = RoomXY::checked_new(x, y).expect("in range");
            if terrain.get_xy(xy) != screeps::Terrain::Swamp {
                continue;
            }

            if try_place(room, xy, StructureType::Road) {
                placed_now.insert((x, y));
                *budget -= 1;
            }
        }
    }
}

fn try_place(room: &Room, xy: RoomXY, ty: StructureType) -> bool {
    match room.create_construction_site(xy.x.u8(), xy.y.u8(), ty, None) {
        Ok(()) => {
            info!(
                "planned {:?} at ({},{}) in {}",
                ty,
                xy.x.u8(),
                xy.y.u8(),
                room.name()
            );
            true
        }
        Err(e) => {
            debug!(
                "couldn't place {:?} at ({},{}): {:?}",
                ty,
                xy.x.u8(),
                xy.y.u8(),
                e
            );
            false
        }
    }
}
