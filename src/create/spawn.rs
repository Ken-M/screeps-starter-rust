use std::collections::HashSet;

use log::*;
use screeps::{IntoExpectedType, Part, ResourceType, ReturnCode, RoomObjectProperties, StructureType, find, prelude::*};
use screeps::StructureExtension;
use screeps::Structure;
use stdweb::js;
use screeps::constants::find::*;


pub fn do_spawn() {
    if screeps::game::creeps::values().len() >= 10 {
        return;
    }
   
    let num_upgrader:i32 = screeps::memory::root().i32("num_upgrader").unwrap_or(Some(0)).unwrap_or(0);
    let num_builder:i32 = screeps::memory::root().i32("num_builder").unwrap_or(Some(0)).unwrap_or(0);
    let num_harvester:i32 = screeps::memory::root().i32("num_harvester").unwrap_or(Some(0)).unwrap_or(0);    

    for spawn in screeps::game::spawns::values() {
        debug!("running spawn {}", spawn.name());

        //check energy can be used.
        let all_structures = spawn
        .room()
        .expect("room is not visible to you")
        .find(STRUCTURES);

        let mut sum_energy = spawn.store_of(ResourceType::Energy);

        for structure in all_structures {

            match structure {
                Structure::Extension(extention) => {
                    if extention.my() == true {
                        sum_energy += extention.store_of(ResourceType::Energy);
                    }
                }
                _ => {
                    // other structure
                }
            }
        }

        let body_unit = [Part::Move, Part::Move, Part::Carry, Part::Work];
        let body_cost: u32 = body_unit.iter().map(|p| p.cost()).sum() ;

        let mut set_num = sum_energy / body_cost ;
        let mut body = Vec::new() ;

        debug!("spawn calc sum_energy:{:?}, body_cost:{:?}, set_num:{:?}", sum_energy, body_cost, set_num);

        while set_num > 0 {
            body.extend(body_unit.iter().cloned());
            set_num -= 1 ;
        }

        if sum_energy >= body.iter().map(|p| p.cost()).sum() {
            // create a unique name, spawn.
            let name_base = screeps::game::time();
            let mut additional = 0;
            let res = loop {
                let name = format!("{}-{}", name_base, additional);
                debug!("try spawn {:?}", body);
                let res = spawn.spawn_creep(&body, &name);

                if res == ReturnCode::NameExists {
                    additional += 1;
                } else {
                    break res;
                }
            };

            if res != ReturnCode::Ok {
                warn!("couldn't spawn: {:?}", res);
            }
            if res == ReturnCode::Ok {
                info!("spawn: {:?}", body);
            }
        }
    }
}