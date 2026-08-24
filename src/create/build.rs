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
            if has_container_near(source.pos(), 1, &structures, &sites) {
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

    // --- 1.5. controller 脇の container ---
    // アップグレード係の補給拠点。hauler がここへ届け、worker は隣で
    // 引き出してそのまま upgrade できる (source⇄controller の往復歩行が
    // 消える。アップグレード throughput の本命)。
    // range 2 に置く: range 1 は creep の立ち位置として予約済みで、
    // range 2 なら隣接マスの全てが upgrade 射程 (3) に収まる。
    if budget > 0 && count_of(StructureType::Container) < allowed(StructureType::Container, rcl) {
        if let Some(controller) = room.controller() {
            if controller.my() && !has_container_near(controller.pos(), 2, &structures, &sites)
            {
                if let Some(xy) =
                    best_ring_tile(room, controller.pos(), 2, &blocked, &reserved, &placed_now)
                {
                    if try_place(room, xy, StructureType::Container) {
                        placed_now.insert((xy.x.u8(), xy.y.u8()));
                        budget -= 1;
                    }
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

    // --- 4. 侵入経路の防衛 rampart ---
    // 出口から重要建造物へ敵が到達できる限り、侵入経路上の最も狭い「門」を
    // rampart で塞いでいく (封鎖検証つき)。
    if budget > 0 && allowed(StructureType::Rampart, rcl) > 0 {
        plan_defense_ramparts(room, &structures, &sites, &mut budget, &mut placed_now);
    }

    // --- 5. 幹線道路 ---
    // spawn から各 source と controller へ。creep の往復が最も多い経路。
    if budget > 0 {
        plan_roads(room, spawn_pos, &blocked, &mut budget, &mut placed_now);
    }
}

/// 塞いでよい「門」の幅の上限 (通行可能マスの列の長さ)。
/// これより開けた場所しか残っていない部屋は封鎖を諦め、重要建造物の
/// 直上 rampart と tower / defender に任せる。開豁地に線を引き始めると
/// 枚数が際限なく増えるための安全弁。
const MAX_CUT_WIDTH: usize = 4;

/// 防衛 rampart (重要建造物の直上を除く) の総数の上限。
/// 封鎖に必要な枚数が地形的にこれを超える場合は打ち切る安全弁。
/// rampart は建設コスト1だが、decay 分の修理維持費が枚数に比例する
/// (修理目標 30k の RCL3 で1枚あたり約 0.03 energy/tick)。
const MAX_DEFENSE_RAMPARTS: usize = 60;

/// 部屋の一辺と総マス数。ビットマップの線形添字用。
const ROOM_SIDE: usize = 50;
const ROOM_TILES: usize = ROOM_SIDE * ROOM_SIDE;

fn tile_idx(x: u8, y: u8) -> usize {
    y as usize * ROOM_SIDE + x as usize
}

fn tile_xy(i: usize) -> (u8, u8) {
    ((i % ROOM_SIDE) as u8, (i / ROOM_SIDE) as u8)
}

/// 侵入経路の防衛 rampart を計画する。
///
/// 考え方:
/// - rampart は自軍 creep だけが通過でき、敵は破壊しないと通れない。
///   出口→拠点の経路上の狭い場所に張れば、最小の枚数で堰き止められる。
/// - 旧実装は「各辺の中央の出口への経路1本」の最窄点1箇所に 3x3 の栓を
///   置くだけで、塞がったかどうかを検証していなかった。実測 (E23N15) では
///   栓のすぐ東が素通しで、敵の経路が spawn の隣まで通っていた。
/// - 本実装は封鎖検証つき。敵視点の BFS (壁・rampart・建造物は通行不可) で
///   「出口から重要建造物の隣接マスへ到達できるか」を確かめ、到達できる限り
///   その経路上で最も狭い門に rampart を張る、を繰り返す。到達不能になれば
///   封鎖完了で、以後この関数は何も置かない (これが定常状態)。
fn plan_defense_ramparts(
    room: &Room,
    structures: &[StructureObject],
    sites: &[screeps::objects::ConstructionSite],
    budget: &mut usize,
    placed_now: &mut HashSet<(u8, u8)>,
) {
    let terrain = room_terrain(room);

    // 敵が通れないマスのビットマップを組む。
    // 1) 地形の壁。ただし道路が乗っていれば敵も通れる。
    let mut blocked = vec![false; ROOM_TILES];
    for x in 0..ROOM_SIDE as u8 {
        for y in 0..ROOM_SIDE as u8 {
            let xy = RoomXY::checked_new(x, y).expect("in range");
            if terrain.get_xy(xy) == screeps::Terrain::Wall {
                blocked[tile_idx(x, y)] = true;
            }
        }
    }
    for s in structures.iter() {
        if s.structure_type() == StructureType::Road {
            let p = s.pos();
            blocked[tile_idx(p.x().u8(), p.y().u8())] = false;
        }
    }

    // 2) 建造物。container / road 以外は敵も (壊すまで) 通れない。
    //    あわせて封鎖の防衛目標 = 重要建造物のマスを集める。
    let mut critical_tiles: Vec<(u8, u8)> = Vec::new();
    for s in structures.iter() {
        let p = s.pos();
        let (x, y) = (p.x().u8(), p.y().u8());
        match s.structure_type() {
            StructureType::Road | StructureType::Container => {}
            ty => {
                blocked[tile_idx(x, y)] = true;
                let critical = matches!(
                    ty,
                    StructureType::Spawn
                        | StructureType::Tower
                        | StructureType::Storage
                        | StructureType::Terminal
                );
                if critical && is_mine(s) {
                    critical_tiles.push((x, y));
                }
            }
        }
    }

    // 3) rampart の建設サイトも「いずれ塞がるもの」として数える。
    //    そうしないと完成までの数十 tick、毎回同じ場所を置き直そうとする。
    //    ついでに防衛 rampart (重要建造物の直上を除く) の枚数を数える。
    let mut defense_ramparts = 0usize;
    for s in structures.iter() {
        if s.structure_type() == StructureType::Rampart {
            let p = s.pos();
            if !critical_tiles.contains(&(p.x().u8(), p.y().u8())) {
                defense_ramparts += 1;
            }
        }
    }
    for c in sites.iter() {
        if c.structure_type() == StructureType::Rampart {
            let p = c.pos();
            let (x, y) = (p.x().u8(), p.y().u8());
            blocked[tile_idx(x, y)] = true;
            if !critical_tiles.contains(&(x, y)) {
                defense_ramparts += 1;
            }
        }
    }

    if critical_tiles.is_empty() {
        return;
    }

    // 防衛目標: 重要建造物の隣接マス。ここへ敵が立てなければ、近接攻撃は
    // 届かない (遠隔は tower / defender の受け持ち)。
    let mut target = vec![false; ROOM_TILES];
    for &(cx, cy) in critical_tiles.iter() {
        for dx in -1..=1i8 {
            for dy in -1..=1i8 {
                let (nx, ny) = (cx as i8 + dx, cy as i8 + dy);
                if (0..ROOM_SIDE as i8).contains(&nx) && (0..ROOM_SIDE as i8).contains(&ny) {
                    target[tile_idx(nx as u8, ny as u8)] = true;
                }
            }
        }
    }

    // 封鎖できるまで「侵入経路を見つけては最も狭い門を塞ぐ」を繰り返す。
    // 1反復で必ず1枚以上置く (置けなければ打ち切る) ので停止する。
    while *budget > 0 {
        let Some(path) = find_intrusion_path(&blocked, &target) else {
            // 出口から重要建造物の隣へ到達できない = 封鎖完了。
            return;
        };

        let Some((width, gate)) = pick_choke(&path, &blocked) else {
            return;
        };
        if width > MAX_CUT_WIDTH {
            debug!(
                "{}: intrusion path stays open (narrowest gate is {} wide); leaving to towers",
                room.name(),
                width
            );
            return;
        }

        let mut placed = 0;
        for &i in gate.iter() {
            if *budget == 0 {
                break;
            }
            if defense_ramparts >= MAX_DEFENSE_RAMPARTS {
                debug!(
                    "{}: defense rampart cap ({}) reached; sealing aborted",
                    room.name(),
                    MAX_DEFENSE_RAMPARTS
                );
                return;
            }
            let (x, y) = tile_xy(i);
            if placed_now.contains(&(x, y)) || has_rampart_at(x, y, structures, sites) {
                continue;
            }
            let xy = RoomXY::checked_new(x, y).expect("in range");
            if try_place(room, xy, StructureType::Rampart) {
                placed_now.insert((x, y));
                blocked[i] = true;
                *budget -= 1;
                defense_ramparts += 1;
                placed += 1;
            }
        }

        if placed == 0 {
            // 門が見つかったのに1枚も置けない (エンジン側の制約など)。
            // 同じ門を見つけ続けて空回りするのを避ける。
            return;
        }
    }
}

/// 敵視点の侵入経路を探す。
///
/// 部屋の縁の通行可能マス (= 出口) すべてを起点に 8 方向 BFS し、
/// `target` のいずれかに到達する最短経路を「出口→target」の順で返す。
/// 到達できなければ None (= 封鎖されている)。
fn find_intrusion_path(blocked: &[bool], target: &[bool]) -> Option<Vec<usize>> {
    // prev[i] == usize::MAX で未訪問。起点は自分自身を指す。
    let mut prev = vec![usize::MAX; ROOM_TILES];
    let mut queue = std::collections::VecDeque::new();

    for i in 0..ROOM_TILES {
        let (x, y) = tile_xy(i);
        let on_border =
            x == 0 || y == 0 || x == ROOM_SIDE as u8 - 1 || y == ROOM_SIDE as u8 - 1;
        if on_border && !blocked[i] {
            prev[i] = i;
            queue.push_back(i);
        }
    }

    fn reconstruct(prev: &[usize], mut cur: usize) -> Vec<usize> {
        let mut path = vec![cur];
        while prev[cur] != cur {
            cur = prev[cur];
            path.push(cur);
        }
        path.reverse();
        path
    }

    // 縁の起点がすでに target になることは通常ない (重要建造物は縁から
    // 離して置かれる) が、念のため。
    if let Some(&hit) = queue.iter().find(|&&i| target[i]) {
        return Some(reconstruct(&prev, hit));
    }

    while let Some(cur) = queue.pop_front() {
        let (x, y) = tile_xy(cur);
        for dx in -1..=1i8 {
            for dy in -1..=1i8 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let (nx, ny) = (x as i8 + dx, y as i8 + dy);
                if !(0..ROOM_SIDE as i8).contains(&nx) || !(0..ROOM_SIDE as i8).contains(&ny) {
                    continue;
                }
                let n = tile_idx(nx as u8, ny as u8);
                if blocked[n] || prev[n] != usize::MAX {
                    continue;
                }
                prev[n] = cur;
                if target[n] {
                    return Some(reconstruct(&prev, n));
                }
                queue.push_back(n);
            }
        }
    }

    None
}

/// 経路上で最も狭い「門」を選ぶ。返り値は (幅, 塞ぐべきマスの列)。
/// 部屋の縁 EDGE_MARGIN 以内は対象外 (縁に張ると部屋の外から攻撃され、
/// 修理も出口を塞いで難しい)。
fn pick_choke(path: &[usize], blocked: &[bool]) -> Option<(usize, Vec<usize>)> {
    let mut best: Option<(usize, Vec<usize>)> = None;

    for k in 1..path.len().saturating_sub(1) {
        let (x, y) = tile_xy(path[k]);
        if x < EDGE_MARGIN
            || y < EDGE_MARGIN
            || x >= ROOM_SIDE as u8 - EDGE_MARGIN
            || y >= ROOM_SIDE as u8 - EDGE_MARGIN
        {
            continue;
        }
        let Some((width, gate)) = gate_at(path, k, blocked) else {
            continue;
        };
        if best.as_ref().is_none_or(|(b, _)| width < *b) {
            best = Some((width, gate));
        }
    }

    best
}

/// path[k] を通る敵の進行方向に直交する「門」= 連続した通行可能マスの列。
/// 返り値は (幅, マスの列)。幅が MAX_CUT_WIDTH を超えた時点、または部屋の
/// 縁まで開いていた場合は「門ではない」として幅だけ大きく返す。
fn gate_at(path: &[usize], k: usize, blocked: &[bool]) -> Option<(usize, Vec<usize>)> {
    let (px, py) = tile_xy(path[k - 1]);
    let (nx, ny) = tile_xy(path[k + 1]);
    let sx = (nx as i8 - px as i8).signum();
    let sy = (ny as i8 - py as i8).signum();
    if sx == 0 && sy == 0 {
        return None;
    }
    // 直交方向。進行方向が斜めなら門も斜めの列になる。斜めの列は
    // 角のすり抜けで漏れ得るが、封鎖検証の反復が漏れを検出して追加で塞ぐ。
    let (qx, qy) = (-sy, sx);

    let (cx, cy) = tile_xy(path[k]);
    let mut gate = vec![path[k]];

    for dir in [1i8, -1] {
        let mut step = 1i8;
        loop {
            let (gx, gy) = (cx as i8 + qx * step * dir, cy as i8 + qy * step * dir);
            if !(0..ROOM_SIDE as i8).contains(&gx) || !(0..ROOM_SIDE as i8).contains(&gy) {
                // 部屋の縁まで開いている。ここでは塞ぎ切れない。
                return Some((ROOM_TILES, gate));
            }
            let i = tile_idx(gx as u8, gy as u8);
            if blocked[i] {
                break;
            }
            gate.push(i);
            if gate.len() > MAX_CUT_WIDTH {
                // 広すぎる門は途中で数えるのをやめる (上限判定に足りる幅だけ返す)。
                return Some((gate.len(), gate));
            }
            step += 1;
        }
    }

    Some((gate.len(), gate))
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

fn has_container_near(
    pos: Position,
    range: u32,
    structures: &[StructureObject],
    sites: &[screeps::objects::ConstructionSite],
) -> bool {
    let near = |p: Position| p.get_range_to(pos) <= range;

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
/// 対象から range のリング上で、spawn に最も近い設置可能マスを選ぶ。
/// controller 脇 container の配置に使う (幹線道路側に寄る)。
fn best_ring_tile(
    room: &Room,
    pos: Position,
    range: i8,
    blocked: &HashSet<(u8, u8)>,
    reserved: &HashSet<(u8, u8)>,
    placed_now: &HashSet<(u8, u8)>,
) -> Option<RoomXY> {
    let spawn_pos = room.find(find::MY_SPAWNS, None).first().map(|s| s.pos())?;
    let (cx, cy) = (pos.x().u8() as i8, pos.y().u8() as i8);

    let mut best: Option<(u32, RoomXY)> = None;
    for dx in -range..=range {
        for dy in -range..=range {
            // チェビシェフ距離がちょうど range のリングだけ。
            if dx.abs().max(dy.abs()) != range {
                continue;
            }
            let (x, y) = (cx + dx, cy + dy);
            if !(0..50).contains(&x) || !(0..50).contains(&y) {
                continue;
            }
            let (x, y) = (x as u8, y as u8);
            if blocked.contains(&(x, y))
                || reserved.contains(&(x, y))
                || placed_now.contains(&(x, y))
            {
                continue;
            }
            let xy = RoomXY::checked_new(x, y).expect("in range");
            let dist = spawn_pos.get_range_to(Position::new(
                screeps::local::RoomCoordinate::new(x).expect("in range"),
                screeps::local::RoomCoordinate::new(y).expect("in range"),
                spawn_pos.room_name(),
            ));
            if best.is_none_or(|(b, _)| dist < b) {
                best = Some((dist, xy));
            }
        }
    }
    best.map(|(_, xy)| xy)
}

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
    // controller 脇の補給 container へも幹線を引き切る。hauler の主要な
    // 配送先で交通量が多いのに、controller までの道は袋小路の入り口で
    // 止まっていた (実測: 補給 container の周囲に道路ゼロ)。舗装すれば
    // 道路特化 body の配送が倍速になる上、道路マスは upgrader の席に
    // ならないので搬入レーンが恒久的に確保される。
    for s in room_structures(room).iter() {
        if is_controller_stock(s) {
            goals.push(s.pos());
        }
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
            // 幹線 (spawn⇄source⇄controller) は平地も含めて舗装する。
            // 旧実装は「沼のみ」でこの部屋では候補ゼロ = 道路網0本だった。
            // 道路は MOVE 半減 body (haulerのCARRY2:MOVE1等) の前提であり、
            // miner (MOVE1) の移動も倍速になる。建設コストは平地300と安く、
            // decay 修理は repairer の瀕死優先が拾う。
            let xy = RoomXY::checked_new(x, y).expect("in range");

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

#[cfg(test)]
mod tests {
    use super::*;

    fn open_map() -> Vec<bool> {
        vec![false; ROOM_TILES]
    }

    /// (25,25) を中心に、北壁の gap_width マスだけ開いた箱を作る。
    fn boxed_map(gap_width: usize) -> (Vec<bool>, Vec<bool>) {
        let mut blocked = open_map();
        for i in 20..=30u8 {
            blocked[tile_idx(i, 20)] = true; // 北壁
            blocked[tile_idx(i, 30)] = true; // 南壁
            blocked[tile_idx(20, i)] = true; // 西壁
            blocked[tile_idx(30, i)] = true; // 東壁
        }
        for w in 0..gap_width {
            blocked[tile_idx(25 + w as u8, 20)] = false; // 北壁の門
        }

        let mut target = vec![false; ROOM_TILES];
        target[tile_idx(25, 25)] = true;

        (blocked, target)
    }

    #[test]
    fn 開けた部屋では侵入経路が縁からtargetへ通る() {
        let blocked = open_map();
        let mut target = vec![false; ROOM_TILES];
        target[tile_idx(25, 25)] = true;

        let path = find_intrusion_path(&blocked, &target).expect("経路があるはず");
        let (x, y) = tile_xy(path[0]);
        assert!(x == 0 || y == 0 || x == 49 || y == 49, "起点は部屋の縁");
        assert_eq!(*path.last().unwrap(), tile_idx(25, 25));
    }

    #[test]
    fn 完全に囲まれたtargetへは侵入経路が無い() {
        let (blocked, target) = boxed_map(0);
        assert!(find_intrusion_path(&blocked, &target).is_none());
    }

    #[test]
    fn 一枚扉の門は幅1として検出され塞げば封鎖される() {
        let (mut blocked, target) = boxed_map(1);

        let path = find_intrusion_path(&blocked, &target).expect("門があるので通れる");
        assert!(path.contains(&tile_idx(25, 20)), "経路は門を通る");

        let (width, gate) = pick_choke(&path, &blocked).expect("門が見つかる");
        assert_eq!(width, 1, "最窄点は一枚扉");
        assert_eq!(gate, vec![tile_idx(25, 20)]);

        for i in gate {
            blocked[i] = true;
        }
        assert!(
            find_intrusion_path(&blocked, &target).is_none(),
            "門を塞げば封鎖完了"
        );
    }

    #[test]
    fn 反復すれば幅2の門も封鎖に到達する() {
        let (mut blocked, target) = boxed_map(2);

        // plan_defense_ramparts のループ相当を rampart 設置なしで再現。
        let mut placed = 0;
        while let Some(path) = find_intrusion_path(&blocked, &target) {
            let (width, gate) = pick_choke(&path, &blocked).expect("門が見つかる");
            assert!(width <= MAX_CUT_WIDTH, "幅2の箱で開豁地判定は出ない");
            for i in gate {
                if !blocked[i] {
                    blocked[i] = true;
                    placed += 1;
                }
            }
            assert!(placed < 20, "発散していないか");
        }
        assert!(placed >= 2, "少なくとも門の幅ぶんは置く");
    }

    #[test]
    fn 開豁地は門として扱わない() {
        // 壁が一切ない部屋: どの地点も幅が MAX_CUT_WIDTH を超える。
        let blocked = open_map();
        let mut target = vec![false; ROOM_TILES];
        target[tile_idx(25, 25)] = true;

        let path = find_intrusion_path(&blocked, &target).unwrap();
        let (width, _) = pick_choke(&path, &blocked).expect("候補自体は返る");
        assert!(width > MAX_CUT_WIDTH, "開豁地に線を引き始めない");
    }
}
