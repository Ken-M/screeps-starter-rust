use log::*;
use screeps::{Attackable, Creep, Part, ResourceType, ReturnCode, RoomObjectProperties, find, game, pathfinder::SearchResults, prelude::*};
use screeps::constants::find::*;
use crate::util::*;
use screeps::Structure;


pub fn run_tower(){

    for game_structure in screeps::game::structures::values() {

        let mut task_done = false ;

        if check_my_structure(&game_structure) == true {

            match game_structure {
                Structure::Tower(my_tower) => {

                    debug!("check enemies {}", my_tower.id());
                    let enemies = my_tower
                    .room()
                    .expect("room is not visible to you")
                    .find(HOSTILE_CREEPS);
        
                    for enemy in enemies {
                        debug!("try attack enemy {}", my_tower.id());
                        let r = my_tower.attack(&enemy) ;
            
                        if r == ReturnCode::Ok {
                            info!("attack to enemy!!");
                            task_done = true ;
                            break ;
                        }
                    } 

                    if task_done == true {
                        return ;
                    }


                    debug!("heal creeps {}", my_tower.id());
                    let my_creeps = my_tower
                    .room()
                    .expect("room is not visible to you")
                    .find(MY_CREEPS);
        
                    for my_creep in my_creeps {

                        if my_creep.hits() < my_creep.hits_max() {
                            debug!("heal my creep {}", my_tower.id());
                            let r = my_tower.heal(&my_creep) ;
                
                            if r == ReturnCode::Ok {
                                info!("heal my creep!!");
                                task_done = true ;
                                break ;
                            }
                        }
                    } 

                    if task_done == true {
                        return ;
                    }

                    if my_tower.store_of(ResourceType::Energy) > (my_tower.store_capacity(Some(ResourceType::Energy))/2) {
                        debug!("repair structure {}", my_tower.id());
                        let my_structures = my_tower
                        .room()
                        .expect("room is not visible to you")
                        .find(STRUCTURES);
            
                        for my_structure in my_structures {

                            match my_structure.as_attackable() {

                                Some(attackable) => {                    
                                    if attackable.hits() < attackable.hits_max() {
                                        let r = my_tower.repair(&my_structure) ;               
                                        if r == ReturnCode::Ok {
                                            info!("repair my structure!!");
                                            task_done = true ;
                                            break ;
                                        }
                                    } 
                                }

                                None => {
                                    // my_struct is not transferable.
                                }
                            }
                        }
                    } 

                    if task_done == true {
                        return ;
                    }
                }

                _ => {
                }
            }
        }
    } 

}
