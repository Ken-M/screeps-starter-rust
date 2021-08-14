use crate::util::*;
use log::*;
use screeps::constants::find::*;
use screeps::{
    find, pathfinder::SearchResults, prelude::*, Creep, Part, ResourceType, ReturnCode,
    RoomObjectProperties, StructureType, Transferable,
};
use std::cmp::*;

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
    let is_harvested_from_terminal = creep.memory().bool("harvested_from_terminal");
    let is_harvested_from_link = creep.memory().bool("harvested_from_link");

    // not far extention .
    let structures = &creep
        .room()
        .expect("room is not visible to you")
        .find(STRUCTURES);

    for structure in structures.iter() {
        if structure.structure_type() != StructureType::Extension {
            continue;
        }

        if check_transferable(structure, &ResourceType::Energy) {
            match structure.as_transferable() {
                Some(transf) => {
                    let r = creep.transfer_all(transf, ResourceType::Energy);

                    if r == ReturnCode::Ok {
                        info!("transferd to my_structure!!");
                        return;
                    }
                }

                None => {
                    // my_struct is not transferable.
                }
            }
        }
    }

    let res = find_nearest_transferable_structure(
        &creep,
        &StructureType::Extension,
        &ResourceType::Energy,
        Some(20 as f64),
    );
    debug!("go to extention:{:?}", res.load_local_path());

    if res.incomplete == false {
        let res = creep.move_by_path_search_result(&res);
        if res == ReturnCode::Ok {
            return;
        }

        info!("couldn't move to transfer: {:?}", res);
    }

    // others.
    for structure in structures.iter() {
        if is_harvested_from_storage == true
            && (structure.structure_type() == StructureType::Container
                || structure.structure_type() == StructureType::Storage)
        {
            //前回storage系からresourceを調達している場合はもどさないようにする.

            continue;
        }

        if is_harvested_from_terminal == true
            && (structure.structure_type() == StructureType::Terminal)
        {
            continue;
        }

        if is_harvested_from_link == true && (structure.structure_type() == StructureType::Link) {
            continue;
        }

        if check_transferable(structure, &ResourceType::Energy) {
            if structure.structure_type() == StructureType::Container {
                match structure.as_transferable() {
                    Some(_transf) => {
                        match structure.as_has_store() {
                            Some(has_store) => {
                                if structure.pos() == creep.pos() {
                                    let trans_amount: u32 = min(
                                        has_store.store_free_capacity(Some(ResourceType::Energy))
                                            as u32,
                                        creep.store_of(ResourceType::Energy),
                                    );
                                    let r = creep.drop(ResourceType::Energy, Some(trans_amount));

                                    if r == ReturnCode::Ok {
                                        info!("dropeed to container!!");
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
            } else {
                match structure.as_transferable() {
                    Some(transf) => {
                        let r = creep.transfer_all(transf, ResourceType::Energy);

                        if r == ReturnCode::Ok {
                            info!("transferd to my_structure!!");
                            return;
                        }
                    }

                    None => {
                        // my_struct is not transferable.
                    }
                }
            }
        }
    }

    let res = find_nearest_transfarable_item(
        &creep,
        &ResourceKind::ENERGY,
        &is_harvested_from_storage,
        &is_harvested_from_terminal,
        &is_harvested_from_link,
    );
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

    let res = find_nearest_transferable_structure(
        &creep,
        &StructureType::Spawn,
        &ResourceType::Energy,
        None,
    );
    debug!("go to:{:?}", res.load_local_path());

    if res.load_local_path().len() > 0 {
        let res = creep.move_by_path_search_result(&res);
        if res == ReturnCode::Ok {
            return;
        }
    }

    // extention.
    let my_structures = &creep
        .room()
        .expect("room is not visible to you")
        .find(STRUCTURES);

    for my_structure in my_structures.iter() {
        if my_structure.structure_type() == StructureType::Extension {
            debug!("try transfer to extention {}", name);
            match my_structure.as_owned() {
                Some(my_extention) => {
                    if my_extention.my() == true {
                        match my_structure.as_transferable() {
                            Some(transf) => {
                                match my_structure.as_has_store() {
                                    Some(has_store) => {
                                        if has_store.store_free_capacity(Some(ResourceType::Energy))
                                            > 0
                                        {
                                            let r =
                                                creep.transfer_all(transf, ResourceType::Energy);

                                            if r == ReturnCode::Ok {
                                                info!("transferd to extention!!");
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

    let res = find_nearest_transferable_structure(
        &creep,
        &StructureType::Extension,
        &ResourceType::Energy,
        None,
    );
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

    let res = find_nearest_transferable_structure(
        &creep,
        &StructureType::Tower,
        &ResourceType::Energy,
        None,
    );
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
    info!("running harvester mineral {}", creep.name());

    if creep.store_used_capacity(None) <= 0 {
        // nothing to do.
        return;
    }

    let is_harvested_from_storage = creep.memory().bool("harvested_from_storage");
    let is_harvested_from_terminal = creep.memory().bool("harvested_from_terminal");

    let structures = &creep
        .room()
        .expect("room is not visible to you")
        .find(STRUCTURES);

    let resrouce_type_list = make_resoucetype_list(&ResourceKind::MINELALS);

    for structure in structures.iter() {
        if is_harvested_from_storage == true
            && (structure.structure_type() == StructureType::Container
                || structure.structure_type() == StructureType::Storage)
        {
            //前回storage系からresourceを調達している場合はもどさないようにする.

            continue;
        }

        if is_harvested_from_terminal == true
            && (structure.structure_type() == StructureType::Terminal)
        {
            //前回Terminalからresourceを調達している場合はもどさないようにする.

            continue;
        }

        for resource_type in resrouce_type_list.iter() {
            if &creep.store_of(*resource_type) > &(0 as u32) {
                if check_transferable(structure, &resource_type) {
                    if structure.structure_type() == StructureType::Container {
                        match structure.as_transferable() {
                            Some(_transf) => {
                                match structure.as_has_store() {
                                    Some(has_store) => {
                                        if structure.pos() == creep.pos() {
                                            let trans_amount: u32 = min(
                                                has_store.store_free_capacity(Some(*resource_type))
                                                    as u32,
                                                creep.store_of(*resource_type),
                                            );
                                            let r = creep.drop(*resource_type, Some(trans_amount));

                                            if r == ReturnCode::Ok {
                                                info!("dropeed to container!!");
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
                    } else {
                        match structure.as_transferable() {
                            Some(transf) => {
                                let r = creep.transfer_all(transf, *resource_type);

                                if r == ReturnCode::Ok {
                                    info!("transferd to my_structure!!");
                                    return;
                                }
                            }

                            None => {
                                // my_struct is not transferable.
                            }
                        }
                    }
                }
            }
        }
    }

    let res = find_nearest_transfarable_item(
        &creep,
        &ResourceKind::MINELALS,
        &is_harvested_from_storage,
        &is_harvested_from_terminal,
        &false,
    );
    debug!("go to:{:?}", res.load_local_path());

    if res.load_local_path().len() > 0 {
        let res = creep.move_by_path_search_result(&res);
        if res != ReturnCode::Ok {
            info!("couldn't move to transfer: {:?}", res);
        }

        return;
    }
}


pub fn run_carrier_mineral(creep: &Creep) {
    let _name = creep.name();
    info!("running carrier mineral {}", creep.name());

    if creep.store_used_capacity(None) <= 0 {
        // nothing to do.
        return;
    }

    let structures = &creep
        .room()
        .expect("room is not visible to you")
        .find(STRUCTURES);

    let resrouce_type_list = make_resoucetype_list(&ResourceKind::MINELALS);

    for structure in structures.iter() {
        if structure.structure_type() != StructureType::Terminal
        {
            //Terminal以外にはtransferしない.
            continue;
        }

        for resource_type in resrouce_type_list.iter() {
            if &creep.store_of(*resource_type) > &(0 as u32) {
                if check_transferable(structure, &resource_type) {
                    match structure.as_transferable() {
                        Some(transf) => {
                            let r = creep.transfer_all(transf, *resource_type);

                            if r == ReturnCode::Ok {
                                info!("transferd to my_structure!!");
                                return;
                            }
                        }

                        None => {
                            // my_struct is not transferable.
                        }
                    }
                }
            }

        }
    }

    let res = find_nearest_transfarable_terminal(
        &creep,
        &ResourceKind::MINELALS
    );
    debug!("go to:{:?}", res.load_local_path());

    if res.load_local_path().len() > 0 {
        let res = creep.move_by_path_search_result(&res);
        if res != ReturnCode::Ok {
            info!("couldn't move to transfer: {:?}", res);
        }

        return;
    }
}
