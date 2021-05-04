use log::*;
use screeps::{Creep, Part, ResourceType, ReturnCode, RoomObjectProperties, find, pathfinder::SearchResults, prelude::*};
use screeps::constants::find::*;
use crate::util::*;


pub fn run_harvester(creep:Creep){

    let name = creep.name();
    debug!("check spawns {}", name);

    let my_spawns = &creep
    .room()
    .expect("room is not visible to you")
    .find(MY_SPAWNS);

    for my_spawn in my_spawns.iter() {
        debug!("try transfer to spawns {}", name);
        let r = creep.transfer_all(my_spawn, ResourceType::Energy);

        if r == ReturnCode::Ok {
            debug!("transferd to spawn!!");
            return ;
        }
    }

    let structures = &creep
    .room()
    .expect("room is not visible to you")
    .find(STRUCTURES);

    for structure in structures.iter() {

        match structure.as_owned() {            
            Some(my_structure) => {

                if my_structure.my() == false {
                    continue ;
                }

                match structure.as_transferable() {
                    Some(transf) => {
            
                        match structure.as_has_store() {
                            Some(has_store) => {
            
                                if has_store.store_free_capacity(Some(ResourceType::Energy)) > 0  {
                                    let r = creep.transfer_all(transf, ResourceType::Energy);

                                    if r == ReturnCode::Ok {
                                        debug!("transferd to my_structure!!");
                                        return ;
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

    let res = find_nearest_transfarable_item(&creep);
    debug!("go to:{:?}", res.load_local_path());

    if res.load_local_path().len() > 0 {
        let last_pos = *(res.load_local_path().last().unwrap());
        let res = creep.move_to(&last_pos); 
        if res != ReturnCode::Ok {
            warn!("couldn't move to transfer: {:?}", res);
        }
    }
}