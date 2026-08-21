use crate::constants::*;
use crate::util::*;
use log::*;
use screeps::enums::StructureObject;
use screeps::prelude::*;
use screeps::{find, game, ResourceType, StructureType};

pub fn run_tower() {
    for game_structure in game::structures().values() {
        let mut is_done = false;

        if check_my_structure(&game_structure) == true {
            match game_structure {
                StructureObject::StructureTower(my_tower) => {
                    debug!("check enemies {}", my_tower.id());
                    let enemies = my_tower
                        .room()
                        .expect("room is not visible to you")
                        .find(find::HOSTILE_CREEPS, None);

                    let room_name = my_tower.room().expect("room is not visible to you").name();

                    for enemy in enemies {
                        debug!("try attack enemy {}", my_tower.id());
                        let r = my_tower.attack(&enemy);

                        if r.is_ok() {
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
                        .find(find::MY_CREEPS, None);

                    for my_creep in my_creeps {
                        if my_creep.hits() < my_creep.hits_max() {
                            debug!("heal my creep {}", my_tower.id());
                            let r = my_tower.heal(&my_creep);

                            if r.is_ok() {
                                info!("heal my creep!!");
                                is_done = true;
                                break;
                            }
                        }
                    }
                    if is_done {
                        continue;
                    }

                    if my_tower.store().get_used_capacity(Some(ResourceType::Energy))
                        > (my_tower.store().get_capacity(Some(ResourceType::Energy)) * 2 / 3)
                    {
                        debug!("repair structure {}", my_tower.id());

                        let my_structures = my_tower
                            .room()
                            .expect("room is not visible to you")
                            .find(find::STRUCTURES, None);

                        // 残り時間が短いものを優先.
                        for structure in my_structures.iter() {
                            if structure.structure_type() != StructureType::Wall {
                                if check_repairable(structure) {
                                    if get_live_tickcount(structure).unwrap_or(10000)
                                        <= REPAIRER_DYING_THRESHOLD
                                    {
                                        if let Some(repairable) = structure.as_repairable() {
                                            let r = my_tower.repair(repairable);
                                            if r.is_ok() {
                                                info!("repair my structure!!");
                                                is_done = true;
                                                break;
                                            }
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
                        let threshold = stats.1 + (stats.0 - stats.1) / 1000;

                        for structure in my_structures.iter() {
                            if check_repairable(structure) {
                                if get_hp(structure).unwrap_or(0) <= (threshold + 1) as u32 {
                                    if let Some(repairable) = structure.as_repairable() {
                                        let r = my_tower.repair(repairable);
                                        if r.is_ok() {
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
