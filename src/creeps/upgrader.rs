use crate::mem::{keys, MemoryExt};
use crate::util::*;
use log::*;

use screeps::action_error_codes::UpgradeControllerErrorCode;
use screeps::local::RoomXY;
use screeps::prelude::*;
use screeps::{game, Creep, Position, ResourceType};

/// 専任アップグレード係 (WORK 全振り body)。
///
/// controller 脇の補給 container の隣が定位置。withdraw と upgrade は
/// 別枠のアクションで同 tick に併用できるため、CARRY が小さくても
/// 毎 tick 継ぎ足しながら WORK 全開で回り続けられる。
pub fn run_dedicated_upgrader(creep: &Creep) {
    let Some(room) = creep.room() else {
        return;
    };

    // 空荷なら調達から。補給 container を含む最寄りの貯蔵へ。
    if creep
        .store()
        .get_used_capacity(Some(ResourceType::Energy))
        == 0
    {
        super::hauler::collect_energy(creep, &room, true);
        return;
    }

    // 隣の補給 container から継ぎ足す (upgrade と同 tick に併用可)。
    top_up_from_stock(creep, &room);

    // 指定席方式 (miner の MINE_AT と同じ粘着)。席が無いと container の
    // 周りに無秩序に集まり、悪い位置取りの1体が他の進入経路を塞ぐ
    // (実測: 席未定の upgrader どうしが道を融通し合えず滞留)。
    if let Some(seat) = resolve_seat(creep, &room) {
        if creep.pos() == seat {
            try_upgrade(creep);
            return;
        }
        let res = find_path(creep, &seat, 0);
        if !res.path().is_empty() {
            let _ = move_by_search_result(creep, &res);
            // 席へ移動中でも射程内なら同 tick に upgrade できる (別枠アクション)。
            try_upgrade(creep);
            return;
        }
        // 席まで詰まっている。以下の通常動作で射程内に留まる。
    }

    run_upgrader(creep);
}

/// controller が自室にあり射程内なら upgrade する (移動はしない)。
fn try_upgrade(creep: &Creep) {
    if let Some(controller) = creep.room().and_then(|r| r.controller()) {
        if controller.my() {
            let _ = creep.upgrade_controller(&controller);
        }
    }
}

/// claim 済みの席が今も有効ならそれを、無ければ選び直して claim する。
fn resolve_seat(creep: &Creep, room: &screeps::objects::Room) -> Option<Position> {
    let cmem = creep.memory();

    if let Ok(Some(s)) = cmem.string(keys::UPGRADE_SEAT) {
        if let Some(pos) = parse_seat(&s, room) {
            if seat_valid(room, pos) {
                return Some(pos);
            }
        }
        cmem.del(keys::UPGRADE_SEAT);
    }

    let seat = pick_seat(creep, room)?;
    cmem.set(
        keys::UPGRADE_SEAT,
        format!("{},{}", seat.x().u8(), seat.y().u8()),
    );
    info!(
        "{} claims upgrade seat ({},{})",
        creep.name(),
        seat.x().u8(),
        seat.y().u8()
    );
    Some(seat)
}

fn parse_seat(s: &str, room: &screeps::objects::Room) -> Option<Position> {
    let (x, y) = s.split_once(',')?;
    let xy = RoomXY::checked_new(x.parse().ok()?, y.parse().ok()?).ok()?;
    Some(Position::new(xy.x, xy.y, room.name()))
}

/// 席の有効条件: 今も補給 container の上か隣であること。
fn seat_valid(room: &screeps::objects::Room, pos: Position) -> bool {
    room_structures(room)
        .iter()
        .any(|s| is_controller_stock(s) && pos.get_range_to(s.pos()) <= 1)
}

/// 空いている席を選ぶ。候補は補給 container の上と隣接8マス
/// (container は controller の range 2 以内なので、どの候補も
/// range 3 の upgrade 射程に収まる)。
/// 道路上の席は最後の手段: 道路は輸送の幹線で、座ると対向をブロックする。
fn pick_seat(creep: &Creep, room: &screeps::objects::Room) -> Option<Position> {
    // 他の生存 upgrader が claim 済みの席。Memory への書き込みは同 tick 内
    // でも見えるので、複数が同時に選んでも早い者勝ちで重複しない。
    let taken: Vec<String> = game::creeps()
        .values()
        .filter(|c| c.name() != creep.name())
        .filter_map(|c| c.memory().string(keys::UPGRADE_SEAT).ok().flatten())
        .collect();

    let mut best: Option<(bool, u32, Position)> = None;
    for structure in room_structures(room).iter() {
        if !is_controller_stock(structure) {
            continue;
        }
        let center = structure.pos();
        for dx in -1..=1i8 {
            for dy in -1..=1i8 {
                let x = center.x().u8() as i8 + dx;
                let y = center.y().u8() as i8 + dy;
                if !(0..50).contains(&x) || !(0..50).contains(&y) {
                    continue;
                }
                let Ok(xy) = RoomXY::checked_new(x as u8, y as u8) else {
                    continue;
                };
                if !is_walkable_tile(room, xy) {
                    continue;
                }
                if taken.contains(&format!("{},{}", x, y)) {
                    continue;
                }
                let pos = Position::new(xy.x, xy.y, room.name());
                let road = tile_has_road(room, xy);
                let dist = creep.pos().get_range_to(pos);
                if best
                    .as_ref()
                    .is_none_or(|(b_road, b_dist, _)| (road, dist) < (*b_road, *b_dist))
                {
                    best = Some((road, dist, pos));
                }
            }
        }
    }
    best.map(|(_, _, pos)| pos)
}

/// 隣接する controller 脇の補給 container から手持ちを補充する。
fn top_up_from_stock(creep: &Creep, room: &screeps::objects::Room) {
    if creep
        .store()
        .get_free_capacity(Some(ResourceType::Energy))
        == 0
    {
        return;
    }
    for structure in room_structures(room).iter() {
        if !is_controller_stock(structure) {
            continue;
        }
        if !creep.pos().is_near_to(structure.pos()) {
            continue;
        }
        if !check_stored(structure, &ResourceType::Energy, 0) {
            continue;
        }
        if let screeps::enums::StructureObject::StructureContainer(container) = structure {
            let _ = creep.withdraw(container, ResourceType::Energy, None);
            return;
        }
    }
}

pub fn run_upgrader(creep: &Creep) {
    let name = creep.name();
    debug!("running upgrader {}", creep.name());

    debug!("check controller {}", name);

    if let Some(c) = creep
        .room()
        .expect("room is not visible to you")
        .controller()
    {
        if c.my() == true {
            let r = creep.upgrade_controller(&c);

            match r {
                Err(UpgradeControllerErrorCode::NotInRange) => {
                    let res = find_path(&creep, &c.pos(), 3);

                    if res.path().len() > 0 {
                        let res = move_by_search_result(&creep, &res);
                        if let Err(e) = res {
                            debug!("couldn't move to upgrade: {:?}", e);
                        } else {
                            return;
                        }
                    }
                }

                Err(e) => {
                    warn!(
                        "couldn't upgrade: {:?},{:?}",
                        e,
                        creep.store().get_used_capacity(None)
                    );
                }

                Ok(()) => {
                    return;
                }
            }
        }
    }

    let res = find_nearest_room_controler(&creep);
    debug!("go to:{:?}", res.path());

    if res.path().len() > 0 {
        let res = move_by_search_result(&creep, &res);
        if let Err(e) = res {
            debug!("couldn't move to build: {:?}", e);
        }

        return;
    }
}
