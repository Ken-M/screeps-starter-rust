use crate::util::*;
use log::*;

use screeps::action_error_codes::UpgradeControllerErrorCode;
use screeps::prelude::*;
use screeps::{Creep, ResourceType};

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

    run_upgrader(creep);
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
