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

    let room_name = &creep
    .room()
    .expect("room is not visible to you")
    .name() ;


    // 残り時間が短いものを優先.
    // Wall以外でまず確認.
    for structure in structures.iter() {
        if check_repairable(structure) {
            if get_live_tickcount(structure).unwrap_or(10000) as u128 <= 500 {

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

    // Wall以外でまず確認.
    if is_skip_repair == false {
        for structure in structures.iter() {
            if structure.structure_type() != StructureType::Wall {
                if check_repairable(structure) {
                    if get_hp_rate(structure).unwrap_or(0) as u128 <= (get_hp_average_exceptwall(room_name) + 1) {

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
    }

    // Wall含め 5k.
    if is_skip_repair == false {
        for structure in structures.iter() {
            if structure.structure_type() == StructureType::Wall {
                if check_repairable_hp(structure, 5000) {
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

    // Wall含め 10k.
    if is_skip_repair == false {
        for structure in structures.iter() {
            if structure.structure_type() == StructureType::Wall {
                if check_repairable_hp(structure, 10000) {
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

    // のこり.
    if is_skip_repair == false {
        for structure in structures.iter() {
            if structure.structure_type() == StructureType::Wall {
                let repair_hp = get_repairable_hp(structure);

                match repair_hp {
                    Some(hp) => {
                        if hp >= (get_repairable_hp_average_wall(room_name)-1) as u32 {
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
                    None => {}
                }
            }
        }
    }

    //----------------------------------------
    // Wall以外でまず確認.
    let res = find_nearest_repairable_item_except_wall_dying(&creep);

    if res.load_local_path().len() > 0 {
        let res = creep.move_by_path_search_result(&res);
        if res != ReturnCode::Ok {
            info!("couldn't move to repair: {:?}", res);
        }
        return;
    }

    let res = find_nearest_repairable_item_except_wall_hp(&creep, (get_hp_average_exceptwall(room_name) + 1) as u32);

    if res.load_local_path().len() > 0 {
        let res = creep.move_by_path_search_result(&res);
        if res != ReturnCode::Ok {
            info!("couldn't move to repair: {:?}", res);
        }
        return;
    }

    // Wall含め 1k.
    let res = find_nearest_repairable_item_onlywall_hp(&creep, 5000);

    if res.load_local_path().len() > 0 {
        let res = creep.move_by_path_search_result(&res);
        if res != ReturnCode::Ok {
            info!("couldn't move to repair: {:?}", res);
        }

        return;
    }

    // Wall含め 1m.
    let res = find_nearest_repairable_item_onlywall_hp(&creep, 10000);

    if res.load_local_path().len() > 0 {
        let res = creep.move_by_path_search_result(&res);
        if res != ReturnCode::Ok {
            info!("couldn't move to repair: {:?}", res);
        }

        return;
    }

    // Wall含め.
    let res = find_nearest_repairable_item_onlywall_repair_hp(&creep, (get_repairable_hp_average_wall(room_name)-1) as u32);

    if res.load_local_path().len() > 0 {
        let res = creep.move_by_path_search_result(&res);
        if res != ReturnCode::Ok {
            info!("couldn't move to repair: {:?}", res);
        }

        return;
    }


    run_upgrader(creep);
}
