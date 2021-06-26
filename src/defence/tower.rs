use log::*;
use screeps::{Attackable, Creep, Part, ResourceType, ReturnCode, RoomObjectProperties, StructureType, find, game, pathfinder::SearchResults, prelude::*};
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

                    if my_tower.store_of(ResourceType::Energy) > (my_tower.store_capacity(Some(ResourceType::Energy))*2/3) {
                        debug!("repair structure {}", my_tower.id());

                        let my_structures = my_tower
                        .room()
                        .expect("room is not visible to you")
                        .find(STRUCTURES);
                        
                        // Wall以外でまず確認.
                        for structure in my_structures.iter() {
                            if structure.structure_type() != StructureType::Wall {
                                if check_repairable(structure) {
                                    let r = my_tower.repair(structure) ;               
                                    if r == ReturnCode::Ok {
                                        info!("repair my structure!!");
                                        task_done = true ;
                                        break ;
                                    }
                                }
                            }
                        }

                        // Wall含め.
                        for structure in my_structures.iter() {
                            if structure.structure_type() == StructureType::Wall {
                                if check_repairable_hp(structure, 1000) {
                                    let r = my_tower.repair(structure) ;               
                                    if r == ReturnCode::Ok {
                                        info!("repair my structure!!");
                                        task_done = true ;
                                        break ;
                                    }
                                }
                            }
                        }

                        for structure in my_structures.iter() {
                            if structure.structure_type() == StructureType::Wall {
                                if check_repairable_hp(structure, 1000000) {
                                    let r = my_tower.repair(structure) ;               
                                    if r == ReturnCode::Ok {
                                        info!("repair my structure!!");
                                        task_done = true ;
                                        break ;
                                    }
                                }
                            }
                        }

                        for structure in my_structures.iter() {
                            if structure.structure_type() == StructureType::Wall {
                                if check_repairable(structure) {
                                    let r = my_tower.repair(structure) ;               
                                    if r == ReturnCode::Ok {
                                        info!("repair my structure!!");
                                        task_done = true ;
                                        break ;
                                    }
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
