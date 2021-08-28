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
        let mut is_done = false;

        if check_my_structure(&game_structure) == true {
            match game_structure {
                Structure::Tower(my_tower) => {
                    debug!("check enemies {}", my_tower.id());
                    let enemies = my_tower
                        .room()
                        .expect("room is not visible to you")
                        .find(HOSTILE_CREEPS);

                    let room_name = my_tower.room().expect("room is not visible to you").name();

                    for enemy in enemies {
                        debug!("try attack enemy {}", my_tower.id());
                        let r = my_tower.attack(&enemy);

                        if r == ReturnCode::Ok {
                            info!("attack to enemy!!");
                            is_done = true;
                            break;
                        }
                    }

                    if is_done {
                        continue;
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
                                is_done = true;
                                break;
                            }
                        }
                    }
                    if is_done {
                        continue;
                    }

                    if my_tower.store_of(ResourceType::Energy)
                        > (my_tower.store_capacity(Some(ResourceType::Energy)) * 2 / 3)
                    {
                        debug!("repair structure {}", my_tower.id());

                        let my_structures = my_tower
                            .room()
                            .expect("room is not visible to you")
                            .find(STRUCTURES);

                        // 残り時間が短いものを優先.
                        for structure in my_structures.iter() {
                            if structure.structure_type() != StructureType::Wall {
                                if check_repairable(structure) {
                                    if get_live_tickcount(structure).unwrap_or(10000) <= 1000 {
                                        let r = my_tower.repair(structure);
                                        if r == ReturnCode::Ok {
                                            info!("repair my structure!!");
                                            is_done = true;
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                        if is_done {
                            continue;
                        }

                        // HPが低い物を確認.
                        let stats = get_hp_average(&room_name);
                        let threshold = (stats.0 + stats.1) / 2;

                        for structure in my_structures.iter() {
                            if structure.structure_type() != StructureType::Wall {
                                if check_repairable(structure) {
                                    if get_hp(structure).unwrap_or(0) <= (threshold + 1) as u32 {
                                        let r = my_tower.repair(structure);
                                        if r == ReturnCode::Ok {
                                            info!("repair my structure!!");
                                            is_done = true;
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                        if is_done {
                            continue;
                        }
                    }
                }

                _ => {}
            }
        }
    }
}
