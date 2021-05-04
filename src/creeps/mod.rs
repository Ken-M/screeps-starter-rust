mod builder;
mod harvester;
mod upgrader;

use log::*;
use screeps::{LookConstant, Part, Position, ResourceType, ReturnCode, RoomObjectProperties, find, look::CREEPS, pathfinder::SearchResults, prelude::*};
use screeps::constants::find::*;
use crate::util::*;

use stdweb::serde ;


pub fn creep_loop() {

    let mut num_builder:i32 = 0 ;
    let mut num_harvester:i32 = 0 ;
    let mut num_upgrader:i32 = 0 ;

    for creep in screeps::game::creeps::values() {
        let name = creep.name();
        debug!("running creep {}", name);

        let role = creep.memory().string("role");
        let mut role_string =  String::from("none");
        debug!("role:{:?}", role); 

        if let Ok(object) = role {
            if let Some(object) = object {
                role_string = object;
            } else {
                role_string = String::from("none");
            }
        }

        match role_string.as_str() {
            "harvester" => {
                num_harvester += 1;
            }

            "builder" => {
                num_builder += 1;
            }

            "upgrader" => {
                num_upgrader += 1;
            }

            "none" => {
                if num_upgrader <= (screeps::game::creeps::values().len() as i32 / 4) {
                    creep.memory().set("role", "upgrader");
                    num_upgrader += 1 ;
                    role_string = String::from("upgrader") ;
                } else if num_builder <= (screeps::game::creeps::values().len() as i32 / 4) {
                    creep.memory().set("role", "builder");
                    num_builder += 1;
                    role_string = String::from("builder") ;                
                } else {
                    creep.memory().set("role", "harvester");
                    num_harvester += 1;     
                    role_string = String::from("harvester") ;                       
                }
            }

            &_ => {
                error!("no role info");
            }
        }

        if creep.spawning() {
            continue;
        }

        if creep.memory().bool("harvesting") {
            if creep.store_free_capacity(Some(ResourceType::Energy)) == 0 {
                creep.memory().set("harvesting", false);
                creep.memory().del("target_pos");
            }
        } else {
            if creep.store_used_capacity(None) == 0 {
                creep.memory().set("harvesting", true);
                creep.memory().del("target_pos");
            }
        }

        if creep.memory().bool("harvesting") {
            debug!("harvesting {}", name);

            let check_string = creep.memory().string("target_pos");      
            debug!("harvesting string{:?}", check_string); 

            let mut defined_target_pos = creep.pos() ;
            
            match check_string {
                Ok(v) => {
                    match v {
                        Some(v) => {
                            let defined_target_obj : Result<Position, serde_json::Error> = serde_json::from_str(v.as_str());

                            match defined_target_obj {
                                Ok(object) => {
                                    defined_target_pos = object ;
                                    debug!("harvesting decided:{}", defined_target_pos);

                                    let look_result = creep
                                    .room()
                                    .expect("I can't see")
                                    .look_for_at_xy(CREEPS, defined_target_pos.x(), defined_target_pos.y());

                                    if look_result.len() > 0 {
                                        debug!("re-check source :{}", defined_target_pos);
                                        creep.memory().del("target_pos");
                                        
                                        let res = find_nearest_active_source(&creep);

                                        if res.load_local_path().len() > 0 {
                    
                                            let last_pos = *(res.load_local_path().last().unwrap());
                    
                                            let json_str = serde_json::to_string(&last_pos).unwrap();
                                            creep.memory().set("target_pos", json_str);
                    
                                            defined_target_pos = last_pos.clone() ;
                                            debug!("harvesting : target_pos:{:?}", creep.memory().string("target_pos"));
                                        } else {
                                            defined_target_pos = creep.pos() ;
                                        }                                       
                                    }
                                }

                                Err(err) => {
                                    let res = find_nearest_active_source(&creep);

                                    if res.load_local_path().len() > 0 {
                
                                        let last_pos = *(res.load_local_path().last().unwrap());
                
                                        let json_str = serde_json::to_string(&last_pos).unwrap();
                                        creep.memory().set("target_pos", json_str);
                
                                        defined_target_pos = last_pos.clone() ;
                                        debug!("harvesting : target_pos:{:?}", creep.memory().string("target_pos"));
                                    } else {
                                        defined_target_pos = creep.pos() ;
                                    }
                                }
                            }
                        }
        
                        None => {
                            let res = find_nearest_active_source(&creep);

                            if res.load_local_path().len() > 0 {
        
                                let last_pos = *(res.load_local_path().last().unwrap());
        
                                let json_str = serde_json::to_string(&last_pos).unwrap();
                                creep.memory().set("target_pos", json_str);
        
                                defined_target_pos = last_pos.clone() ;
                                debug!("harvesting : target_pos:{:?}", creep.memory().string("target_pos"));
                            } else {
                                defined_target_pos = creep.pos() ;
                            } 
                        }
                    }
                }

                Err(err) => {
                    let res = find_nearest_active_source(&creep);

                    if res.load_local_path().len() > 0 {

                        let last_pos = *(res.load_local_path().last().unwrap());

                        let json_str = serde_json::to_string(&last_pos).unwrap();
                        creep.memory().set("target_pos", json_str);

                        defined_target_pos = last_pos.clone() ;
                        debug!("harvesting : target_pos:{:?}", creep.memory().string("target_pos"));
                    } else {
                        defined_target_pos = creep.pos() ;
                    }                
                }
            }

            let sources = &creep
            .room()
            .expect("room is not visible to you")
            .find(find::SOURCES_ACTIVE);        

            let mut is_harvested = false;
        
            for source in sources.iter() {
                if creep.pos().is_near_to(source) {
                    let r = creep.harvest(source);
                    if r != ReturnCode::Ok {
                        warn!("couldn't harvest: {:?}", r);
                        continue;
                    }
                    is_harvested = true;
                    break;
                } 
            }

            if is_harvested == false {

                let res = creep.move_to(&defined_target_pos);           

                if res != ReturnCode::Ok {
                    warn!("couldn't move to source: {:?}", res);
                    if res == ReturnCode::NoPath {
                        creep.memory().del("target_pos");
                    }
                }
            }

        } else {
            debug!("TASK role:{:?}", role_string);

            match role_string.as_str() {
                "harvester" => {
                    harvester::run_harvester(creep) ;
                }

                "builder" => {
                    builder::run_builder(creep) ;
                }

                "upgrader" => {
                    upgrader::run_upgrader(creep) ;
                }

                "none" => {
                    if num_upgrader <= (screeps::game::creeps::values().len() as i32 / 4) {
                        creep.memory().set("role", "upgrader");
                        upgrader::run_upgrader(creep) ;
                    } else if num_builder <= (screeps::game::creeps::values().len() as i32 / 4) {
                        creep.memory().set("role", "builder");
                        builder::run_builder(creep) ;  
            
                    } else {
                        creep.memory().set("role", "harvester");
                        harvester::run_harvester(creep) ;                     
                    }
                }

                &_ => {
                    error!("no role info");
                }
            }
        }
    }

    // check number of each type creeps.
    screeps::memory::root().set("num_upgrader", num_upgrader);
    screeps::memory::root().set("num_builder", num_builder);   
    screeps::memory::root().set("num_harvester", num_harvester);

}


