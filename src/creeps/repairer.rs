use log::*;
use screeps::{Attackable, Creep, Part, ResourceType, ReturnCode, RoomObjectProperties, StructureType, find, pathfinder::SearchResults, prelude::*};
use screeps::constants::find::*;
use crate::util::*;

use crate::creeps::harvester::*;

pub fn run_repairer(creep:&Creep){

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
                return ;
            }
        }
    }

    let structures = &creep
    .room()
    .expect("room is not visible to you")
    .find(STRUCTURES);

    // Wall以外でまず確認.
    for structure in structures.iter() {
        if structure.structure_type() != StructureType::Wall {
            if check_repairable(structure) {
                let r = creep.repair(structure);

                if r == ReturnCode::Ok {
                    info!("repair my_structure!!");
                    return ;
                }
            }
        }
    }

    // Wall含め.
    for structure in structures.iter() {
        if structure.structure_type() == StructureType::Wall {
            if check_repairable_hp(structure, 1000) {
                let r = creep.repair(structure);

                if r == ReturnCode::Ok {
                    info!("repair my_structure!!");
                    return ;
                }
            }
        }
    }

    for structure in structures.iter() {
        if structure.structure_type() == StructureType::Wall {
            if check_repairable_hp(structure, 1000000) {
                let r = creep.repair(structure);

                if r == ReturnCode::Ok {
                    info!("repair my_structure!!");
                    return ;
                }
            }
        }
    }

    for structure in structures.iter() {
        if structure.structure_type() == StructureType::Wall {
            if check_repairable(structure) {
                let r = creep.repair(structure);

                if r == ReturnCode::Ok {
                    info!("repair my_structure!!");
                    return ;
                }
            }
        }
    }

    //----------------------------------------
    // Wall以外でまず確認.
    debug!("1");
    let res = find_nearest_repairable_item_except_wall(&creep) ;

    if res.load_local_path().len() > 0 {
        let res = creep.move_by_path_search_result(&res); 
        if res != ReturnCode::Ok {
            info!("couldn't move to repair: {:?}", res);
        }
        return ;
    }   

    // Wall含め 1k.
    debug!("2");
    let res = find_nearest_repairable_item_onlywall_hp1k(&creep);

    if res.load_local_path().len() > 0 {
        let res = creep.move_by_path_search_result(&res); 
        if res != ReturnCode::Ok {
            info!("couldn't move to repair: {:?}", res);
        }

        return ;
    }

    // Wall含め 1m.
    debug!("3");
    let res = find_nearest_repairable_item_onlywall_hp1m(&creep);

    if res.load_local_path().len() > 0 {
        let res = creep.move_by_path_search_result(&res); 
        if res != ReturnCode::Ok {
            info!("couldn't move to repair: {:?}", res);
        }

        return ;
    }

    // Wall含め.
    debug!("4");
    let res = find_nearest_repairable_item_onlywall(&creep);

    if res.load_local_path().len() > 0 {
        let res = creep.move_by_path_search_result(&res); 
        if res != ReturnCode::Ok {
            info!("couldn't move to repair: {:?}", res);
        }

        return ;
    }

    run_harvester(creep) ;
}
