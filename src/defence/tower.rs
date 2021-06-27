use crate::util::*;
use log::*;
use screeps::constants::find::*;
use screeps::Structure;
use screeps::{
    find, game, pathfinder::SearchResults, prelude::*, Attackable, Creep, Part, ResourceType,
    ReturnCode, RoomObjectProperties, StructureType,
};

pub fn run_tower() {
    for game_structure in screeps::game::structures::values() {
        let mut task_done = false;

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
                        let r = my_tower.attack(&enemy);

                        if r == ReturnCode::Ok {
                            info!("attack to enemy!!");
                            task_done = true;
                            break;
                        }
                    }

                    if task_done == true {
                        return;
                    }

                    debug!("heal creeps {}", my_tower.id());
                    let my_creeps = my_tower
                        .room()
                        .expect("room is not visible to you")
                        .find(MY_CREEPS);

                    for my_creep in my_creeps {
                        if my_creep.hits() < my_creep.hits_max() {
                            debug!("heal my creep {}", my_tower.id());
                            let r = my_tower.heal(&my_creep);

                            if r == ReturnCode::Ok {
                                info!("heal my creep!!");
                                task_done = true;
                                break;
                            }
                        }
                    }

                    if task_done == true {
                        return;
                    }

                    if my_tower.store_of(ResourceType::Energy)
                        > (my_tower.store_capacity(Some(ResourceType::Energy)) * 2 / 3)
                    {
                        debug!("repair structure {}", my_tower.id());

                        let my_structures = my_tower
                            .room()
                            .expect("room is not visible to you")
                            .find(STRUCTURES);

                        // Wall以外でまず確認.
                        for structure in my_structures.iter() {
                            if structure.structure_type() != StructureType::Wall {
                                if check_repairable(structure) {
                                    let r = my_tower.repair(structure);
                                    if r == ReturnCode::Ok {
                                        info!("repair my structure!!");
                                        task_done = true;
                                        break;
                                    }
                                }
                            }
                        }

                        let mut total_repair_hp: u128 = 0;
                        let mut count: u128 = 0;

                        for structure in my_structures.iter() {
                            if structure.structure_type() == StructureType::Wall {
                                let repair_hp = get_repairable_hp(structure);

                                match repair_hp {
                                    Some(hp) => {
                                        count += 1;
                                        total_repair_hp += hp as u128;
                                    }
                                    None => {}
                                }
                            }
                        }

                        // Wall含め.
                        for structure in my_structures.iter() {
                            if structure.structure_type() == StructureType::Wall {
                                if check_repairable_hp(structure, 5000) {
                                    let r = my_tower.repair(structure);
                                    if r == ReturnCode::Ok {
                                        info!("repair my structure!!");
                                        task_done = true;
                                        break;
                                    }
                                }
                            }
                        }

                        for structure in my_structures.iter() {
                            if structure.structure_type() == StructureType::Wall {
                                if check_repairable_hp(structure, 10000) {
                                    let r = my_tower.repair(structure);
                                    if r == ReturnCode::Ok {
                                        info!("repair my structure!!");
                                        task_done = true;
                                        break;
                                    }
                                }
                            }
                        }

                        if count > 0 {
                            let average = (total_repair_hp / count) - 1;
                            for structure in my_structures.iter() {
                                if structure.structure_type() == StructureType::Wall {
                                    let repair_hp = get_repairable_hp(structure);

                                    match repair_hp {
                                        Some(hp) => {
                                            if hp >= average as u32 {
                                                let r = my_tower.repair(structure);

                                                if r == ReturnCode::Ok {
                                                    info!("repair my structure!!");
                                                    task_done = true;
                                                    break;
                                                }
                                            }
                                        }
                                        None => {}
                                    }
                                }
                            }
                        }
                    }

                    if task_done == true {
                        return;
                    }
                }

                _ => {}
            }
        }
    }
}
