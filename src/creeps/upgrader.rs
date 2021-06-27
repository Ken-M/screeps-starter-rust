use crate::util::*;
use log::*;

use screeps::{
    find, pathfinder::SearchResults, prelude::*, Creep, Part, ResourceType, ReturnCode,
    RoomObjectProperties,
};

pub fn run_upgrader(creep: &Creep) {
    let name = creep.name();
    info!("running upgrader {}", creep.name());

    debug!("check controller {}", name);

    if let Some(c) = creep
        .room()
        .expect("room is not visible to you")
        .controller()
    {
        let r = creep.upgrade_controller(&c);

        if r == ReturnCode::NotInRange {
            let res = find_path(&creep, &c.pos(), 1);

            if res.load_local_path().len() > 0 {
                let res = creep.move_by_path_search_result(&res);
                if res != ReturnCode::Ok {
                    info!("couldn't move to upgrade: {:?}", res);
                }
            }
        } else if r != ReturnCode::Ok {
            warn!("couldn't upgrade: {:?}", r);
        }
    } else {
        warn!("creep room has no controller!");
    }
}
