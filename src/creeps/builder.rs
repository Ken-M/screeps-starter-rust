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

    if construction_sites.len() > 0 {
        let mut sum_of_progress = 0;
        for construction_site in construction_sites.iter() {
            sum_of_progress += construction_site.progress_total() - construction_site.progress();
        }
        let average = (sum_of_progress / construction_sites.len() as u32) + 1;

        for construction_site in construction_sites.iter() {
            if (construction_site.progress_total() - construction_site.progress()) <= average {
                let r = creep.build(construction_site);
                if r == ReturnCode::Ok {
                    info!("build to my_construction_sites!!");
                    return;
                }
            }
        }

        let res = find_nearest_construction_site(&creep, average);
        debug!("go to:{:?}", res.load_local_path());

        if res.load_local_path().len() > 0 {
            let res = creep.move_by_path_search_result(&res);
            if res != ReturnCode::Ok {
                info!("couldn't move to build: {:?}", res);
            }

            return;
        }
    }

    // if nothing to do, act like repairer.
    run_repairer(creep);
}
