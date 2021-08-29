use crate::constants::*;
use std::u128;

use crate::util::*;
use log::*;
use screeps::constants::find::*;
use screeps::{
    find, pathfinder::SearchResults, prelude::*, Attackable, Creep, Part, ResourceType, ReturnCode,
    RoomObjectProperties, StructureType,
};

use crate::creeps::upgrader::*;

pub fn run_repairer(creep: &Creep) {
    let name = creep.name();
    info!("running repairer {}", creep.name());

    debug!("check spawns {}", name);
    let my_spawns = &creep
        .room()
        .expect("room is not visible to you")
        .find(MY_SPAWNS);

    for my_spawn in my_spawns.iter() {
        if my_spawn.hits() < my_spawn.hits_max() {
            debug!("try repair spawns {}", name);
            let r = creep.repair(my_spawn);

            if r == ReturnCode::Ok {
                info!("repair spawn!!");
                return;
            }
        }
    }

    let structures = &creep
        .room()
        .expect("room is not visible to you")
        .find(STRUCTURES);

    let mut is_skip_repair = false;

    let room_name = &creep.room().expect("room is not visible to you").name();

    // 残り時間が短いものを優先.
    for structure in structures.iter() {
        if check_repairable(structure) {
            if get_live_tickcount(structure).unwrap_or(10000) as u128 <= REPAIRER_DYING_THRESHOLD {
                let r = creep.repair(structure);

                if r == ReturnCode::Ok {
                    info!(
                        "repair my_structure!!:{:?},{:?},{:?}",
                        structure.structure_type(),
                        structure.pos().x(),
                        structure.pos().y()
                    );
                    return;
                }

                if r == ReturnCode::NotInRange {
                    is_skip_repair = true;
                }
            }
        }
    }

    // 残りhpが少ない物を優先.
    if is_skip_repair == false {
        let stats = get_hp_average(room_name);
        let threshold = stats.1 + (stats.0 - stats.1) / 1000;

        for structure in structures.iter() {
            if check_repairable(structure) {
                if get_hp(structure).unwrap_or(0) as u128 <= (threshold + 1) {
                    let r = creep.repair(structure);

                    if r == ReturnCode::Ok {
                        info!(
                            "repair my_structure!!:{:?},{:?},{:?}",
                            structure.structure_type(),
                            structure.pos().x(),
                            structure.pos().y()
                        );
                        return;
                    }

                    if r == ReturnCode::NotInRange {
                        is_skip_repair = true;
                    }
                }
            }
        }
    }

    //----------------------------------------
    // 残り時間が少ない物を優先.
    let res = find_nearest_repairable_item_except_wall_dying(&creep, REPAIRER_DYING_THRESHOLD);

    if res.load_local_path().len() > 0 {
        let res = creep.move_by_path_search_result(&res);
        if res != ReturnCode::Ok {
            info!("couldn't move to repair: {:?}", res);
        }
        return;
    }

    // 残りhpが少ない物を優先.
    let stats = get_hp_average(room_name);
    let threshold = stats.1 + (stats.0 - stats.1) / 1000;

    let res = find_nearest_repairable_item_hp(&creep, (threshold + 1) as u32);

    if res.load_local_path().len() > 0 {
        let res = creep.move_by_path_search_result(&res);
        if res != ReturnCode::Ok {
            info!("couldn't move to repair: {:?}", res);
        }
        return;
    }

    run_upgrader(creep);
}
