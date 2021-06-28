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

    // Wall以外でまず確認.
    for structure in structures.iter() {
        if structure.structure_type() != StructureType::Wall {
            if check_repairable(structure) {
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

    let mut total_repair_hp: u128 = 0;
    let mut count: u128 = 0;

    for structure in structures.iter() {
        if structure.structure_type() == StructureType::Wall {
            let repair_hp = get_repairable_hp(structure);

            match repair_hp {
                Some(hp) => {
                    count += 1 as u128;
                    total_repair_hp += hp as u128
                }
                None => {}
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
        if count > 0 {
            let average = ((total_repair_hp / count) - 1) as u32;
            info!("repair_hp:{:?}", average);

            for structure in structures.iter() {
                if structure.structure_type() == StructureType::Wall {
                    let repair_hp = get_repairable_hp(structure);

                    match repair_hp {
                        Some(hp) => {
                            if hp >= average {
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
    }

    //----------------------------------------
    // Wall以外でまず確認.
    debug!("1");
    let res = find_nearest_repairable_item_except_wall(&creep);

    if res.load_local_path().len() > 0 {
        let res = creep.move_by_path_search_result(&res);
        if res != ReturnCode::Ok {
            info!("couldn't move to repair: {:?}", res);
        }
        return;
    }

    // Wall含め 1k.
    debug!("2");
    let res = find_nearest_repairable_item_onlywall_hp(&creep, 5000);

    if res.load_local_path().len() > 0 {
        let res = creep.move_by_path_search_result(&res);
        if res != ReturnCode::Ok {
            info!("couldn't move to repair: {:?}", res);
        }

        return;
    }

    // Wall含め 1m.
    debug!("3");
    let res = find_nearest_repairable_item_onlywall_hp(&creep, 10000);

    if res.load_local_path().len() > 0 {
        let res = creep.move_by_path_search_result(&res);
        if res != ReturnCode::Ok {
            info!("couldn't move to repair: {:?}", res);
        }

        return;
    }

    // Wall含め.
    debug!("4");
    if count > 0 {
        let average = ((total_repair_hp / count) - 1) as u32;
        let res = find_nearest_repairable_item_onlywall_repair_hp(&creep, average);

        if res.load_local_path().len() > 0 {
            let res = creep.move_by_path_search_result(&res);
            if res != ReturnCode::Ok {
                info!("couldn't move to repair: {:?}", res);
            }

            return;
        }
    }

    run_upgrader(creep);
}
