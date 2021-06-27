use crate::util::*;
use log::*;
use screeps::constants::find::*;
use screeps::{
    find, pathfinder::SearchResults, prelude::*, Creep, Part, ResourceType, ReturnCode,
    RoomObjectProperties, StructureType, Transferable,
};

use crate::creeps::builder::*;

pub fn run_harvester(creep: &Creep) {
    let name = creep.name();
    info!("running harvester {}", creep.name());

    debug!("check spawns {}", name);
    let my_spawns = &creep
        .room()
        .expect("room is not visible to you")
        .find(MY_SPAWNS);

    for my_spawn in my_spawns.iter() {
        debug!("try transfer to spawns {}", name);
        let r = creep.transfer_all(my_spawn, ResourceType::Energy);

        if r == ReturnCode::Ok {
            info!("transferd to spawn!!");
            return;
        }
    }

    let is_harvested_from_storage = creep.memory().bool("harvested_from_storage");

    let structures = &creep
        .room()
        .expect("room is not visible to you")
        .find(STRUCTURES);

    for structure in structures.iter() {
        if is_harvested_from_storage == true
            && (structure.structure_type() == StructureType::Container
                || structure.structure_type() == StructureType::Storage
                || structure.structure_type() == StructureType::Terminal)
        {
            //前回storage系からresourceを調達している場合はもどさないようにする.

            continue;
        }

        match structure.as_owned() {
            Some(my_structure) => {
                if my_structure.my() == false {
                    continue;
                }

                match structure.as_transferable() {
                    Some(transf) => {
                        match structure.as_has_store() {
                            Some(has_store) => {
                                if has_store.store_free_capacity(Some(ResourceType::Energy)) > 0 {
                                    let r = creep.transfer_all(transf, ResourceType::Energy);

                                    if r == ReturnCode::Ok {
                                        info!("transferd to my_structure!!");
                                        return;
                                    }
                                }
                            }

                            None => {
                                //no store.
                            }
                        }
                    }

                    None => {
                        // my_struct is not transferable.
                    }
                }
            }

            None => {
                //not mine.
            }
        }
    }

    let res =
        find_nearest_transfarable_item(&creep, &ResourceKind::ENERGY, &is_harvested_from_storage);
    debug!("go to:{:?}", res.load_local_path());

    if res.load_local_path().len() > 0 {
        let res = creep.move_by_path_search_result(&res);
        if res == ReturnCode::Ok {
            return;
        }

        info!("couldn't move to transfer: {:?}", res);
    }

    run_builder(creep);
}

pub fn run_harvester_spawn(creep: &Creep) {
    let name = creep.name();
    info!("running harvester_spawn {}", creep.name());

    debug!("check spawns {}", name);

    let my_spawns = &creep
        .room()
        .expect("room is not visible to you")
        .find(MY_SPAWNS);

    for my_spawn in my_spawns.iter() {
        debug!("try transfer to spawns {}", name);
        let r = creep.transfer_all(my_spawn, ResourceType::Energy);

        if r == ReturnCode::Ok {
            info!("transferd to spawn!!");
            return;
        }
    }

    let res =
        find_nearest_transferable_structure(&creep, &StructureType::Spawn, &ResourceType::Energy);
    debug!("go to:{:?}", res.load_local_path());

    if res.load_local_path().len() > 0 {
        let res = creep.move_by_path_search_result(&res);
        if res == ReturnCode::Ok {
            return;
        }
    }

    debug!("check towers {}", name);

    let my_towers = &creep
        .room()
        .expect("room is not visible to you")
        .find(STRUCTURES);

    for my_tower in my_towers.iter() {
        if my_tower.structure_type() == StructureType::Tower {
            debug!("try transfer to tower {}", name);
            match my_tower.as_owned() {
                Some(my_structure) => {
                    if my_structure.my() == true {
                        match my_tower.as_transferable() {
                            Some(transf) => {
                                match my_tower.as_has_store() {
                                    Some(has_store) => {
                                        if has_store.store_free_capacity(Some(ResourceType::Energy))
                                            > 0
                                        {
                                            let r =
                                                creep.transfer_all(transf, ResourceType::Energy);

                                            if r == ReturnCode::Ok {
                                                info!("transferd to tower!!");
                                                return;
                                            }
                                        }
                                    }

                                    None => {
                                        //no store.
                                    }
                                }
                            }

                            None => {
                                // my_struct is not transferable
                            }
                        }
                    }
                }

                None => {
                    //not my structure.
                }
            }
        }
    }

    let res =
        find_nearest_transferable_structure(&creep, &StructureType::Tower, &ResourceType::Energy);
    debug!("go to:{:?}", res.load_local_path());

    if res.load_local_path().len() > 0 {
        let res = creep.move_by_path_search_result(&res);
        if res == ReturnCode::Ok {
            return;
        }
    }

    // act as normal harvester.
    run_harvester(creep);
}

pub fn run_harvester_mineral(creep: &Creep) {
    let _name = creep.name();
    info!("running harvester mineral{}", creep.name());

    let is_harvested_from_storage = creep.memory().bool("harvested_from_storage");

    let structures = &creep
        .room()
        .expect("room is not visible to you")
        .find(STRUCTURES);

    for structure in structures.iter() {
        if is_harvested_from_storage == true
            && (structure.structure_type() == StructureType::Container
                || structure.structure_type() == StructureType::Storage)
        {
            //前回storage系からresourceを調達している場合はもどさないようにする.

            continue;
        }

        match structure.as_owned() {
            Some(my_structure) => {
                if my_structure.my() == false {
                    continue;
                }

                match structure.as_transferable() {
                    Some(transf) => {
                        match structure.as_has_store() {
                            Some(has_store) => {
                                let resrouce_type_list =
                                    make_resoucetype_list(&ResourceKind::MINELALS);

                                for resource_type in resrouce_type_list {
                                    if has_store.store_free_capacity(Some(resource_type)) > 0 {
                                        let r = creep.transfer_all(transf, resource_type);

                                        if r == ReturnCode::Ok {
                                            info!("transferd to my_structure!!");
                                            return;
                                        }
                                    }
                                }
                            }

                            None => {
                                //no store.
                            }
                        }
                    }

                    None => {
                        // my_struct is not transferable.
                    }
                }
            }

            None => {
                //not mine.
            }
        }
    }

    let res =
        find_nearest_transfarable_item(&creep, &ResourceKind::MINELALS, &is_harvested_from_storage);
    debug!("go to:{:?}", res.load_local_path());

    if res.load_local_path().len() > 0 {
        let res = creep.move_by_path_search_result(&res);
        if res != ReturnCode::Ok {
            info!("couldn't move to transfer: {:?}", res);
        }

        return;
    }

    run_builder(creep);
}
