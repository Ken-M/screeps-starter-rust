use std::usize;

use log::*;
use screeps::constants::find::*;
use screeps::Structure;

use screeps::{
    find, prelude::*, Attackable, IntoExpectedType, Part, ResourceType, ReturnCode,
    RoomObjectProperties, StructureType,
};

const MAX_NUM_OF_CREEPS: u32 = 20;

pub fn do_spawn() {
    if screeps::game::creeps::values().len() >= MAX_NUM_OF_CREEPS as usize {
        return;
    }

    let _num_upgrader: i32 = screeps::memory::root()
        .i32("num_upgrader")
        .unwrap_or(Some(0))
        .unwrap_or(0);
    let _num_builder: i32 = screeps::memory::root()
        .i32("num_builder")
        .unwrap_or(Some(0))
        .unwrap_or(0);
    let num_harvester: i32 = screeps::memory::root()
        .i32("num_harvester")
        .unwrap_or(Some(0))
        .unwrap_or(0);
    let num_harvester_spawn: i32 = screeps::memory::root()
        .i32("num_harvester_spawn")
        .unwrap_or(Some(0))
        .unwrap_or(0);
    let _num_harvester_mineral: i32 = screeps::memory::root()
        .i32("num_harvester_mineral")
        .unwrap_or(Some(0))
        .unwrap_or(0);
    let _num_repairer: i32 = screeps::memory::root()
        .i32("num_repairer")
        .unwrap_or(Some(0))
        .unwrap_or(0);

    let opt_num_attackable_short: i32 = screeps::memory::root()
        .i32("opt_num_attackable_short")
        .unwrap_or(Some(0))
        .unwrap_or(0);
    let opt_num_attackable_long: i32 = screeps::memory::root()
        .i32("opt_num_attackable_long")
        .unwrap_or(Some(0))
        .unwrap_or(0);

    let num_total_creep = screeps::game::creeps::values().len() as i32;

    for spawn in screeps::game::spawns::values() {
        info!("running spawn {}", spawn.name());

        // check got attacked.
        if (spawn.hits() < spawn.hits_max()) || ((num_total_creep as u32) < MAX_NUM_OF_CREEPS / 2) {
            info!("got attacked!!");

            let all_structures = spawn
                .room()
                .expect("room is not visible to you")
                .find(STRUCTURES);

            for structure in all_structures {
                match structure {
                    Structure::Controller(controller) => {
                        controller.activate_safe_mode();
                    }
                    _ => {
                        //nothint to do.
                    }
                }
            }
        }

        //check energy can be used.
        let all_structures = spawn
            .room()
            .expect("room is not visible to you")
            .find(STRUCTURES);

        let mut sum_energy = spawn.store_of(ResourceType::Energy);
        let mut num_extention = 0 ;

        for structure in all_structures {
            match structure {
                Structure::Extension(extention) => {
                    if extention.my() == true {
                        sum_energy += extention.store_of(ResourceType::Energy);
                        num_extention += 1 ;
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

        let body_cost: u32 = body_unit.iter().map(|p| p.cost()).sum();
        let body_short_atk_cost: u32 = body_short_atk_unit.iter().map(|p| p.cost()).sum();
        let body_long_atk_cost: u32 = body_long_atk_unit.iter().map(|p| p.cost()).sum();

        let body_cost_vec = vec![body_cost, body_short_atk_cost, body_long_atk_cost];
        let _min_cost = body_cost_vec.iter().min().unwrap();

        let mut body = Vec::new();

        debug!("spawn calc sum_energy:{:?}", sum_energy);


        if ((num_harvester + num_harvester_spawn) >= 6) && (num_extention >= 6){
            if sum_energy < body_cost * 2 {
                continue ;
            }
        }

        // とりあえず基本セットをつける.
        if sum_energy >= body_cost {
            body.extend(body_unit.iter().cloned());
            sum_energy -= body_cost;
        } else {
            // 基本セット分だけEnergyがたまってなければまた次回.
            continue;
        }

        // 長距離攻撃がたりなければ装備.
        if opt_num_attackable_long < std::cmp::max(1, num_total_creep / 5) {
            if sum_energy >= body_long_atk_cost {
                let mut count = 0;

                while (sum_energy >= body_long_atk_cost)
                    && ((body.len() + body_long_atk_unit.len())
                        < screeps::constants::MAX_CREEP_SIZE as usize)
                {
                    count += 1;
                    if count % 3 == 0 {
                        body.extend(body_unit.iter().cloned());
                        sum_energy -= body_cost;
                    } else {
                        body.extend(body_long_atk_unit.iter().cloned());
                        sum_energy -= body_long_atk_cost;
                    }
                }
            } else {
                if (opt_num_attackable_long + opt_num_attackable_short) < (num_total_creep / 5) {
                    continue;
                }
            }

        // 短距離攻撃が足りなければ装備.
        } else if opt_num_attackable_short < std::cmp::max(1, num_total_creep / 5) {
            if sum_energy >= body_short_atk_cost {
                let mut count = 0;

                while (sum_energy >= body_short_atk_cost)
                    && ((body.len() + body_short_atk_unit.len())
                        < screeps::constants::MAX_CREEP_SIZE as usize)
                {
                    count += 1;

                    if count % 3 == 0 {
                        body.extend(body_unit.iter().cloned());
                        sum_energy -= body_cost;
                    } else {
                        body.extend(body_short_atk_unit.iter().cloned());
                        sum_energy -= body_short_atk_cost;
                    }
                }
            } else {
                if (opt_num_attackable_long + opt_num_attackable_short) < (num_total_creep / 5) {
                    continue;
                }
            }
        }

        // あとは可能な限り基本セット.
        let mut set_num = sum_energy / body_cost;

        while (set_num > 0)
            && ((body.len() + body_unit.len()) < screeps::constants::MAX_CREEP_SIZE as usize)
        {
            body.extend(body_unit.iter().cloned());
            set_num -= 1;
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
                info!("couldn't spawn: {:?}", res);
            }
            if res == ReturnCode::Ok {
                info!("spawn: {:?}", body);
            }
        }
    }
}
