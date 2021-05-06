use std::collections::HashSet;

use log::*;
use screeps::{IntoExpectedType, Part, ResourceType, ReturnCode, RoomObjectProperties, StructureType, find, prelude::*};
use screeps::StructureExtension;
use screeps::Structure;
use stdweb::js;
use screeps::constants::find::*;
use lazy_static::* ;





pub fn do_spawn() {
    if screeps::game::creeps::values().len() >= 17 {
        return;
    }
   
    let num_upgrader:i32 = screeps::memory::root().i32("num_upgrader").unwrap_or(Some(0)).unwrap_or(0);
    let num_builder:i32 = screeps::memory::root().i32("num_builder").unwrap_or(Some(0)).unwrap_or(0);
    let num_harvester:i32 = screeps::memory::root().i32("num_harvester").unwrap_or(Some(0)).unwrap_or(0);    
    let num_harvester_spawn:i32 = screeps::memory::root().i32("num_harvester_spawn").unwrap_or(Some(0)).unwrap_or(0);   
    let num_repairer:i32 = screeps::memory::root().i32("num_repairer").unwrap_or(Some(0)).unwrap_or(0);   

    let opt_num_attackable_short:i32 = screeps::memory::root().i32("opt_num_attackable_short").unwrap_or(Some(0)).unwrap_or(0);   
    let opt_num_attackable_long:i32 = screeps::memory::root().i32("opt_num_attackable_long").unwrap_or(Some(0)).unwrap_or(0);   

    let num_total_creep = screeps::game::creeps::values().len() as i32;

    for spawn in screeps::game::spawns::values() {
        info!("running spawn {}", spawn.name());

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
        let body_short_atk_unit = [Part::Move, Part::Attack];
        let body_long_atk_unit = [Part::Move, Part::RangedAttack];

        let body_cost: u32 = body_unit.iter().map(|p| p.cost()).sum() ;
        let body_short_atk_cost: u32 = body_short_atk_unit.iter().map(|p| p.cost()).sum() ;
        let body_long_atk_cost: u32 = body_long_atk_unit.iter().map(|p| p.cost()).sum() ;

        let body_cost_vec = vec![body_cost, body_short_atk_cost, body_long_atk_cost];
        let min_cost = body_cost_vec.iter().min().unwrap() ;

        let mut body = Vec::new() ;

        debug!("spawn calc sum_energy:{:?}", sum_energy);

        // とりあえず基本セットをつける.
        if sum_energy >= body_cost {
            body.extend(body_unit.iter().cloned());
            sum_energy -= body_cost;
        } else {
            // 基本セット分だけEnergyがたまってなければまた次回.
            return ;
        }

        // 長距離攻撃がたりなければ装備.
        if opt_num_attackable_long < std::cmp::max(1, num_total_creep/5) {

            if sum_energy >= body_long_atk_cost {
                body.extend(body_long_atk_unit.iter().cloned());
                sum_energy -= body_long_atk_cost; 
            }

        // 短距離攻撃が足りなければ装備.           
        } else if opt_num_attackable_short < std::cmp::max(1, num_total_creep/5) {
            if sum_energy >= body_short_atk_cost {
                body.extend(body_short_atk_unit.iter().cloned());
                sum_energy -= body_short_atk_cost; 
            }
        }

        // あとは可能な限り基本セット.
        let mut set_num = sum_energy / body_cost ;

        while set_num > 0 {
            body.extend(body_unit.iter().cloned());
            set_num -= 1 ;
        }

        if body.len() > 0 {
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