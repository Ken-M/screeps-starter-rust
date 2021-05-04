use log::*;
use screeps::{Creep, Part, ResourceType, ReturnCode, RoomObjectProperties, find, pathfinder::SearchResults, prelude::*};
use screeps::constants::find::*;
use crate::util::*;


pub fn run_upgrader(creep:Creep){

    let name = creep.name();

    debug!("check controller {}", name);

    if let Some(c) = creep
        .room()
        .expect("room is not visible to you")
        .controller()
    {
        let r = creep.upgrade_controller(&c);
        
        if r == ReturnCode::NotInRange {
            let res = creep.move_to(&c);

            if res != ReturnCode::Ok {
                warn!("couldn't move to upgrade: {:?}", res);
            }
        } else if r != ReturnCode::Ok {
            warn!("couldn't upgrade: {:?}", r);
        }
    } else {
        warn!("creep room has no controller!");
    }
}