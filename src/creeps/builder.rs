use log::*;
use screeps::{Creep, Part, ResourceType, ReturnCode, RoomObjectProperties, find, pathfinder::SearchResults, prelude::*};
use screeps::constants::find::*;
use crate::util::*;


pub fn run_builder(creep:&Creep){
    let name = creep.name();
    info!("running builder {}", creep.name());

    debug!("check construction sites {}", name);
    let constructin_sites = &creep
    .room()
    .expect("room is not visible to you")
    .find(MY_CONSTRUCTION_SITES);

    for construction_site in constructin_sites.iter() {

        let r = creep.build(construction_site);
        if r == ReturnCode::Ok {
            info!("build to my_construction_sites!!");
            return ;
        }
    }

    let res = find_nearest_construction_site(&creep);
    if res.load_local_path().len() > 0 {
        let last_pos = *(res.load_local_path().last().unwrap());
        let res = creep.move_to(&last_pos); 
        if res != ReturnCode::Ok {
            warn!("couldn't move to build: {:?}", res);
        }
    }
}