use crate::constants::*;
use std::usize;

use crate::mem::{self, MemoryExt};
use log::*;
use screeps::action_error_codes::SpawnCreepErrorCode;
use screeps::enums::StructureObject;
use screeps::prelude::*;
use screeps::{find, game, Part, ResourceType};

const MAX_NUM_OF_CREEPS: u32 = 14;

pub fn do_spawn() {
    let num_total_creep = game::creeps().values().count() as i32;

    if num_total_creep >= MAX_NUM_OF_CREEPS as i32 {
        return;
    }

    let root = mem::root();

    let _num_upgrader: i32 = root.i32("num_upgrader").unwrap_or(Some(0)).unwrap_or(0);
    let _num_builder: i32 = root.i32("num_builder").unwrap_or(Some(0)).unwrap_or(0);
    let _num_harvester: i32 = root.i32("num_harvester").unwrap_or(Some(0)).unwrap_or(0);
    let _num_harvester_spawn: i32 = root
        .i32("num_harvester_spawn")
        .unwrap_or(Some(0))
        .unwrap_or(0);
    let _num_harvester_mineral: i32 = root
        .i32("num_harvester_mineral")
        .unwrap_or(Some(0))
        .unwrap_or(0);
    let _num_carrier_mineral: i32 = root
        .i32("num_carrier_mineral")
        .unwrap_or(Some(0))
        .unwrap_or(0);
    let _num_repairer: i32 = root.i32("num_repairer").unwrap_or(Some(0)).unwrap_or(0);

    let opt_num_attackable_short: i32 = root
        .i32("opt_num_attackable_short")
        .unwrap_or(Some(0))
        .unwrap_or(0);
    let opt_num_attackable_long: i32 = root
        .i32("opt_num_attackable_long")
        .unwrap_or(Some(0))
        .unwrap_or(0);

    let cap_worker_carry: i32 = root.i32("cap_worker_carry").unwrap_or(Some(0)).unwrap_or(0);

    for spawn in game::spawns().values() {
        info!("running spawn {}", spawn.name());

        // check got attacked.
        if (spawn.hits() < spawn.hits_max())
            || ((num_total_creep as u32) < MAX_NUM_OF_CREEPS / 3)
            || ((opt_num_attackable_short + opt_num_attackable_long) <= 0)
        {
            info!("got attacked!!");

            let my_controller = spawn
                .room()
                .expect("room is not visible to you")
                .controller();

            match my_controller {
                Some(controller) => {
                    let _ = controller.activate_safe_mode();
                }
                None => {
                    //nothint to do.
                }
            }
        }

        //check energy can be used.
        let all_structures = spawn
            .room()
            .expect("room is not visible to you")
            .find(find::STRUCTURES, None);

        let mut sum_energy = spawn.store().get_used_capacity(Some(ResourceType::Energy));
        let mut extention_cap = spawn.store().get_capacity(Some(ResourceType::Energy));

        for structure in all_structures {
            match structure {
                StructureObject::StructureExtension(extention) => {
                    if extention.my() == true {
                        sum_energy += extention
                            .store()
                            .get_used_capacity(Some(ResourceType::Energy));
                        extention_cap +=
                            extention.store().get_capacity(Some(ResourceType::Energy));
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
        let min_basic_body_set = std::cmp::min(
            ((cap_worker_carry as f64 * CAP_WORKER_CARRY_COEFF) / (body_cost as f64)) as u32,
            ((extention_cap as f64) / (body_cost as f64)) as u32,
        );

        info!("min basic body set:{:?}", min_basic_body_set);
        if ((cap_worker_carry as f64 * CAP_WORKER_CARRY_COEFF)
            >= (body_cost as f64 * min_basic_body_set as f64))
            && (extention_cap as f64 >= (body_cost as f64 * min_basic_body_set as f64))
        {
            if sum_energy < body_cost * min_basic_body_set {
                continue;
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
        if opt_num_attackable_long < std::cmp::max(1, num_total_creep / 3) {
            if sum_energy >= body_long_atk_cost {
                let mut count = 0;

                while (sum_energy >= body_long_atk_cost)
                    && ((body.len() + body_long_atk_unit.len())
                        < screeps::constants::MAX_CREEP_SIZE as usize)
                {
                    count += 1;
                    if count % 3 == 0 {
                        if sum_energy >= body_cost {
                            body.extend(body_unit.iter().cloned());
                            sum_energy -= body_cost;
                        }
                    } else {
                        body.extend(body_long_atk_unit.iter().cloned());
                        sum_energy -= body_long_atk_cost;
                    }
                }
            } else {
                if ((opt_num_attackable_long + opt_num_attackable_short) < (num_total_creep / 3))
                    && (extention_cap > (body_long_atk_cost + body_cost))
                {
                    continue;
                }
            }

        // 短距離攻撃が足りなければ装備.
        } else if opt_num_attackable_short < std::cmp::max(1, num_total_creep / 3) {
            if sum_energy >= body_short_atk_cost {
                let mut count = 0;

                while (sum_energy >= body_short_atk_cost)
                    && ((body.len() + body_short_atk_unit.len())
                        < screeps::constants::MAX_CREEP_SIZE as usize)
                {
                    count += 1;

                    if count % 3 == 0 {
                        if sum_energy >= body_cost {
                            body.extend(body_unit.iter().cloned());
                            sum_energy -= body_cost;
                        }
                    } else {
                        body.extend(body_short_atk_unit.iter().cloned());
                        sum_energy -= body_short_atk_cost;
                    }
                }
            } else {
                if ((opt_num_attackable_long + opt_num_attackable_short) < (num_total_creep / 3))
                    && (extention_cap > (body_short_atk_cost + body_cost))
                {
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
            let name_base = game::time();
            let mut additional = 0;
            let res = loop {
                let name = format!("{}-{}", name_base, additional);
                debug!("try spawn {:?}", body);
                let res = spawn.spawn_creep(&body, &name);

                if res == Err(SpawnCreepErrorCode::NameExists) {
                    additional += 1;
                } else {
                    break res;
                }
            };

            match res {
                Err(e) => {
                    info!("couldn't spawn: {:?}", e);
                }
                Ok(()) => {
                    info!("spawn: {:?}", body);
                }
            }
        }
    }
}
