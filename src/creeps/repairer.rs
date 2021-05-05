use log::*;
use screeps::{Attackable, Creep, Part, ResourceType, ReturnCode, RoomObjectProperties, find, pathfinder::SearchResults, prelude::*};
use screeps::constants::find::*;
use crate::util::*;


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

    for structure in structures.iter() {

        if check_repairable(structure) {
            let r = creep.repair(structure);

            if r == ReturnCode::Ok {
                info!("repair my_structure!!");
                return ;
            }
        }
    }

    let res = find_nearest_repairable_item(&creep);

    if res.load_local_path().len() > 0 {
        let last_pos = *(res.load_local_path().last().unwrap());
        let res = creep.move_to(&last_pos); 
        if res != ReturnCode::Ok {
            warn!("couldn't move to repair: {:?}", res);
        }
    }
}
