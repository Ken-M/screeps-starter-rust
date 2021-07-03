use crate::util::*;
use log::*;
use screeps::constants::find::*;
use screeps::{
    find, pathfinder::SearchResults, prelude::*, Creep, Part, ResourceType, ReturnCode,
    RoomObjectProperties,
};

use crate::creeps::repairer::*;

pub fn run_builder(creep: &Creep) {
    let name = creep.name();
    info!("running builder {}", creep.name());

    debug!("check construction sites {}", name);
    let construction_sites = &creep
        .room()
        .expect("room is not visible to you")
        .find(MY_CONSTRUCTION_SITES);

    let room_name = creep
        .room()
        .expect("room is not visible to you")
        .name() ;


    for construction_site in construction_sites.iter() {
        if (construction_site.progress_total() - construction_site.progress()) <= (get_construction_progress_average(&room_name) + 1) as u32 {
            let r = creep.build(construction_site);
            if r == ReturnCode::Ok {
                info!("build to my_construction_sites!!");
                return;
            }
        }
    }

    let res = find_nearest_construction_site(&creep, (get_construction_progress_average(&room_name) + 1) as u32);
    debug!("go to:{:?}", res.load_local_path());

    if res.load_local_path().len() > 0 {
        let res = creep.move_by_path_search_result(&res);
        if res != ReturnCode::Ok {
            info!("couldn't move to build: {:?}", res);
        }

        return;
    }


    // if nothing to do, act like repairer.
    run_repairer(creep);
}
