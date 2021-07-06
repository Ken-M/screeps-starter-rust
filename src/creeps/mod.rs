mod builder;
mod harvester;
mod repairer;
mod upgrader;

use crate::util::*;
use log::*;
use screeps::constants::find::*;
use screeps::Structure;
use screeps::{
    find, look::CREEPS, pathfinder::SearchResults, prelude::*, resource, Creep, LookConstant, Part,
    Position, ResourceType, ReturnCode, RoomObjectProperties, StructureType,
};

#[derive(PartialEq, Debug)]
enum AttackerKind {
    SHORT,
    RANGED,
    NONE,
}

fn reset_source_target(
    creep: &Creep,
    is_harvester: bool,
    harvest_kind: &ResourceKind,
) -> (SearchResults, Position) {
    debug!("harvesting : reset_source_target");

    if is_harvester == true {
        // active sourceをチェック.
        let res = find_nearest_active_source(&creep, harvest_kind, false);
        debug!(
            "harvesting : find_nearest_active_source result:{:?}",
            res.load_local_path()
        );

        if res.load_local_path().len() > 0 {
            let last_pos = *(res.load_local_path().last().unwrap());
            let json_str = serde_json::to_string(&last_pos).unwrap();
            creep.memory().set("target_pos", json_str);
            creep.memory().set("target_pos_count", 20);
            creep.memory().set("will_harvest_from_storage", false);

            debug!(
                "harvesting : target_pos:{:?}",
                creep.memory().string("target_pos")
            );

            let ret_position = res.load_local_path().last().unwrap().clone();
            return (res, ret_position);
        }

        // storageをチェック.
        let res = find_nearest_stored_source(&creep, harvest_kind, true);

        if res.load_local_path().len() > 0 {
            let last_pos = *(res.load_local_path().last().unwrap());
            let json_str = serde_json::to_string(&last_pos).unwrap();
            creep.memory().set("target_pos", json_str);
            creep.memory().set("target_pos_count", 10);
            creep.memory().set("will_harvest_from_storage", true);

            debug!(
                "harvesting : target_pos:{:?}",
                creep.memory().string("target_pos")
            );

            let ret_position = res.load_local_path().last().unwrap().clone();
            return (res, ret_position);
        }
    } else {
        // storageをチェック.
        let res = find_nearest_stored_source(&creep, harvest_kind, false);

        if res.load_local_path().len() > 0 {
            let last_pos = *(res.load_local_path().last().unwrap());
            let json_str = serde_json::to_string(&last_pos).unwrap();
            creep.memory().set("target_pos", json_str);
            creep.memory().set("target_pos_count", 20);
            creep.memory().set("will_harvest_from_storage", true);

            debug!(
                "harvesting : target_pos:{:?}",
                creep.memory().string("target_pos")
            );

            let ret_position = res.load_local_path().last().unwrap().clone();
            return (res, ret_position);
        }

        // active sourceをチェック.
        let res = find_nearest_active_source(&creep, harvest_kind, true);
        debug!(
            "harvesting : find_nearest_active_source result:{:?}",
            res.load_local_path()
        );

        if res.load_local_path().len() > 0 {
            let last_pos = *(res.load_local_path().last().unwrap());
            let json_str = serde_json::to_string(&last_pos).unwrap();
            creep.memory().set("target_pos", json_str);
            creep.memory().set("target_pos_count", 10);
            creep.memory().set("will_harvest_from_storage", false);

            debug!(
                "harvesting : target_pos:{:?}",
                creep.memory().string("target_pos")
            );

            let ret_position = res.load_local_path().last().unwrap().clone();
            return (res, ret_position);
        }
    }

    //　やむなく枯渇sourceを選ぶ.
    let res = find_nearest_source(&creep, harvest_kind);

    if res.load_local_path().len() > 0 {
        let last_pos = *(res.load_local_path().last().unwrap());
        let json_str = serde_json::to_string(&last_pos).unwrap();
        creep.memory().set("target_pos", json_str);
        creep.memory().set("target_pos_count", 5);
        creep.memory().set("will_harvest_from_storage", true);

        debug!(
            "harvesting : target_pos:{:?}",
            creep.memory().string("target_pos")
        );

        let ret_position = res.load_local_path().last().unwrap().clone();
        return (res, ret_position);
    }

    //全部ダメならとりあえずその場待機.
    let res = find_path(&creep, &creep.pos(), 0);
    return (res, creep.pos().clone());
}

fn attacker_routine(creep: &Creep, kind: &AttackerKind) -> bool {
    debug!("check enemies {}", creep.name());
    let enemies = creep
        .room()
        .expect("room is not visible to you")
        .find(HOSTILE_CREEPS);

    if enemies.len() == 0 {
        return false;
    }

    for enemy in enemies {
        debug!("try attack enemy {}", creep.name());

        match kind {
            AttackerKind::SHORT => {
                let r = creep.attack(&enemy);

                if r == ReturnCode::Ok {
                    info!("attack to enemy!!");
                    return true;
                }
            }

            AttackerKind::RANGED => {
                let r = creep.ranged_attack(&enemy);

                if r == ReturnCode::Ok {
                    info!("attack to enemy!!");
                    return true;
                }
            }

            _ => {}
        }
    }

    let mut range: u32 = 1;
    match kind {
        AttackerKind::SHORT => {
            range = 1;
        }

        AttackerKind::RANGED => {
            range = 2;
        }

        _ => {}
    }

    let res = find_nearest_enemy(&creep, range);
    debug!("go to:{:?}", res.load_local_path());

    if res.load_local_path().len() > 0 {
        let res = creep.move_by_path_search_result(&res);
        if res == ReturnCode::Ok {
            info!("move to enemy: {:?}", res);
            return true;
        }
    }

    return false;
}

fn get_role_and_attacker_kind(creep: &Creep) -> (String, AttackerKind) {
    let mut attacker_kind: AttackerKind = AttackerKind::NONE;
    let role = creep.memory().string("role");
    let mut role_string = String::from("none");

    // attacker kind check.
    let body_list = creep.body();
    for body_part in body_list {
        if body_part.part == Part::Attack {
            attacker_kind = AttackerKind::SHORT;
            break;
        } else if body_part.part == Part::RangedAttack {
            attacker_kind = AttackerKind::RANGED;
            break;
        }
    }

    if let Ok(object) = role {
        if let Some(object) = object {
            role_string = object;
        } else {
            role_string = String::from("none");
        }
    }

    return (role_string, attacker_kind);
}

pub fn creep_loop() {
    let mut num_builder: i32 = 0;
    let mut num_harvester: i32 = 0;
    let mut num_upgrader: i32 = 0;
    let mut num_harvester_spawn: i32 = 0;
    let mut num_harvester_mineral: i32 = 0;
    let mut num_repairer: i32 = 0;

    let mut opt_num_attackable_short: i32 = 0;
    let mut opt_num_attackable_long: i32 = 0;

    let mut cap_worker_carry: u128 = 0;

    for creep in screeps::game::creeps::values() {
        let name = creep.name();
        debug!("checking creep {}", name);

        let mut attacker_kind: AttackerKind = AttackerKind::NONE;
        let mut role_string = String::from("none");

        let role_and_attacker_kind = get_role_and_attacker_kind(&creep);

        role_string = role_and_attacker_kind.0;
        attacker_kind = role_and_attacker_kind.1;

        debug!("role:{:?}:atk:{:?}", role_string, attacker_kind);

        match attacker_kind {
            AttackerKind::SHORT => {
                opt_num_attackable_short += 1;
            }

            AttackerKind::RANGED => {
                opt_num_attackable_long += 1;
            }

            AttackerKind::NONE => {
                //nothing.
            }
        }

        match role_string.as_str() {
            "harvester" => {
                num_harvester += 1;
                cap_worker_carry += creep.store_capacity(None) as u128;
            }

            "harvester_spawn" => {
                num_harvester_spawn += 1;
                cap_worker_carry += creep.store_capacity(None) as u128;
            }

            "harvester_mineral" => {
                num_harvester_mineral += 1;
            }

            "builder" => {
                num_builder += 1;
            }

            "upgrader" => {
                num_upgrader += 1;
            }

            "repairer" => {
                num_repairer += 1;
            }

            "none" => {
                // do nothing.
            }

            &_ => {
                error!("no role info");
            }
        }
    }

    for creep in screeps::game::creeps::values() {
        let name = creep.name();
        info!(
            "running creep {}, cpu:{}",
            name,
            screeps::game::cpu::get_used()
        );

        let mut attacker_kind: AttackerKind = AttackerKind::NONE;
        let mut role_string = String::from("none");

        let role_and_attacker_kind = get_role_and_attacker_kind(&creep);
        let mut harvest_kind: ResourceKind = ResourceKind::ENERGY;

        let mut is_harvester = false;

        role_string = role_and_attacker_kind.0;
        attacker_kind = role_and_attacker_kind.1;

        match role_string.as_str() {
            "none" => {
                if num_harvester_spawn == 0 {
                    creep.memory().set("role", "harvester_spawn");
                    num_harvester_spawn += 1;
                    role_string = String::from("harvester_spawn");
                    cap_worker_carry += creep.store_capacity(None) as u128;
                } else if num_upgrader < (screeps::game::creeps::values().len() as i32 / 6) + 1 {
                    creep.memory().set("role", "upgrader");
                    num_upgrader += 1;
                    role_string = String::from("upgrader");
                } else if num_builder < (screeps::game::creeps::values().len() as i32 / 6) {
                    creep.memory().set("role", "builder");
                    num_builder += 1;
                    role_string = String::from("builder");
                } else if num_repairer < (screeps::game::creeps::values().len() as i32 / 6) {
                    creep.memory().set("role", "repairer");
                    num_repairer += 1;
                    role_string = String::from("repairer");
                } else if (num_harvester_mineral <= 1)
                    && (screeps::game::creeps::values().len() as i32 > 20)
                {
                    creep.memory().set("role", "harvester_mineral");
                    num_harvester_mineral += 1;
                    harvest_kind = ResourceKind::MINELALS;
                    role_string = String::from("harvester_mineral");
                    is_harvester = true;
                } else {
                    creep.memory().set("role", "harvester");
                    num_harvester += 1;
                    role_string = String::from("harvester");
                    is_harvester = true;
                    cap_worker_carry += creep.store_capacity(None) as u128;
                }
            }

            "harvester" => {
                is_harvester = true;
            }

            "harvester_mineral" => {
                is_harvester = true;
            }

            &_ => {
                // do nothing.
            }
        }

        info!("role:{:?}:atk:{:?}", role_string, attacker_kind);

        if creep.spawning() {
            continue;
        }

        //// atacker check.
        if attacker_kind != AttackerKind::NONE {
            let result = attacker_routine(&creep, &attacker_kind);

            if result == true {
                continue;
            }
        }

        //// harvest resrouce kind.
        if role_string == String::from("harvester_mineral") {
            harvest_kind = ResourceKind::MINELALS;
        }

        if creep.memory().bool("harvesting") {
            if creep.store_free_capacity(None) == 0 {
                creep.memory().set("harvesting", false);
                creep.memory().del("target_pos");
                creep.memory().del("will_harvest_from_storage");
            }
        } else {
            if creep.store_used_capacity(None) == 0 {
                creep.memory().set("harvesting", true);
                creep.memory().del("target_pos");
                creep.memory().del("harvested_from_storage");
            }
        }

        if creep.memory().bool("harvesting") {
            debug!("harvesting {}", name);

            let check_string = creep.memory().string("target_pos");
            debug!("harvesting string{:?}", check_string);

            let mut defined_target_pos = creep.pos();
            let mut path_search_result;

            match check_string {
                Ok(v) => {
                    match v {
                        Some(v) => {
                            let defined_target_obj: Result<Position, serde_json::Error> =
                                serde_json::from_str(v.as_str());

                            match defined_target_obj {
                                Ok(object) => {
                                    defined_target_pos = object;
                                    debug!("harvesting decided:{}", defined_target_pos);
                                    path_search_result = find_path(&creep, &defined_target_pos, 0);
                                    debug!(
                                        "harvesting decided path:{:?}",
                                        path_search_result.load_local_path()
                                    );

                                    let look_result =
                                        creep.room().expect("I can't see").look_for_at_xy(
                                            CREEPS,
                                            defined_target_pos.x(),
                                            defined_target_pos.y(),
                                        );

                                    for one_result in look_result {
                                        if one_result != creep {
                                            debug!("re-check source :{}", defined_target_pos);
                                            creep.memory().del("target_pos");

                                            let reset_result = reset_source_target(
                                                &creep,
                                                is_harvester,
                                                &harvest_kind,
                                            );
                                            path_search_result = reset_result.0;
                                            defined_target_pos = reset_result.1;

                                            break;
                                        }
                                    }
                                }

                                Err(_err) => {
                                    //ロードに成功して値もあったけどDeSerializeできなかった.
                                    let reset_result =
                                        reset_source_target(&creep, is_harvester, &harvest_kind);
                                    path_search_result = reset_result.0;
                                    defined_target_pos = reset_result.1;
                                }
                            }
                        }

                        None => {
                            //ロードに成功したけど値がない.
                            let reset_result =
                                reset_source_target(&creep, is_harvester, &harvest_kind);
                            path_search_result = reset_result.0;
                            defined_target_pos = reset_result.1;
                        }
                    }
                }

                //ロードに失敗(key自体がない).
                Err(_err) => {
                    let reset_result = reset_source_target(&creep, is_harvester, &harvest_kind);
                    path_search_result = reset_result.0;
                    defined_target_pos = reset_result.1;
                }
            }

            let mut is_harvested = false;

            // check dropped source.
            let resources = &creep
                .room()
                .expect("room is not visible to you")
                .find(find::DROPPED_RESOURCES);

            for resource in resources.iter() {
                if creep.pos().is_near_to(resource)
                    && check_resouce_type_kind_matching(&resource.resource_type(), &harvest_kind)
                {
                    let r = creep.pickup(resource);
                    if r != ReturnCode::Ok {
                        warn!("couldn't pick-up dropped resrouces: {:?}", r);
                        continue;
                    }
                    is_harvested = true;
                    break;
                }
            }

            // check ruins.
            if is_harvested == false {
                let ruins = &creep
                    .room()
                    .expect("room is not visible to you")
                    .find(find::RUINS);

                for ruin in ruins.iter() {
                    if creep.pos().is_near_to(ruin) {
                        let resource_type_list = make_resoucetype_list(&harvest_kind);
                        for resource_type in resource_type_list {
                            if ruin.store_of(resource_type) > 0 {
                                let r = creep.withdraw_all(ruin, resource_type);
                                if r != ReturnCode::Ok {
                                    warn!("couldn't withdraw from RUINs: {:?}", r);
                                    continue;
                                }
                                is_harvested = true;
                                break;
                            }
                        }
                    }

                    if is_harvested == true {
                        break;
                    }
                }
            }

            // check tombstones.
            if is_harvested == false {
                let tombstones = &creep
                    .room()
                    .expect("room is not visible to you")
                    .find(find::TOMBSTONES);

                for tombstone in tombstones.iter() {
                    if creep.pos().is_near_to(tombstone) {
                        let resource_type_list = make_resoucetype_list(&harvest_kind);
                        for resource_type in resource_type_list {
                            if tombstone.store_of(resource_type) > 0 {
                                let r = creep.withdraw_all(tombstone, resource_type);
                                if r != ReturnCode::Ok {
                                    warn!("couldn't withdraw from TOMBSTONES: {:?}", r);
                                    continue;
                                }
                                is_harvested = true;
                                break;
                            }
                        }
                    }

                    if is_harvested == true {
                        break;
                    }
                }
            }

            //  check sources active.
            if is_harvested == false && harvest_kind == ResourceKind::ENERGY {
                let sources = &creep
                    .room()
                    .expect("room is not visible to you")
                    .find(find::SOURCES_ACTIVE);

                for source in sources.iter() {
                    if creep.pos().is_near_to(source) {
                        let r = creep.harvest(source);
                        if r != ReturnCode::Ok {
                            warn!("couldn't harvest from ActiveSource: {:?}", r);
                            continue;
                        }
                        is_harvested = true;
                        break;
                    }
                }
            }

            if is_harvested == false && harvest_kind == ResourceKind::MINELALS {
                let sources = &creep
                    .room()
                    .expect("room is not visible to you")
                    .find(find::MINERALS);

                for source in sources.iter() {
                    if creep.pos().is_near_to(source) {
                        let r = creep.harvest(source);
                        if r != ReturnCode::Ok && r != ReturnCode::Tired {
                            info!("couldn't harvest from Minerals: {:?}", r);
                            continue;
                        }
                        is_harvested = true;
                        break;
                    }
                }
            }

            //  storage.
            if is_harvested == false && creep.memory().bool("will_harvest_from_storage") == true {
                let structures = &creep
                    .room()
                    .expect("room is not visible to you")
                    .find(find::STRUCTURES);

                for structure in structures.iter() {
                    if creep.pos().is_near_to(structure) {
                        let resource_type_list = make_resoucetype_list(&harvest_kind);
                        for resource_type in resource_type_list {
                            if check_stored(structure, &resource_type) {
                                match structure {
                                    Structure::Container(container) => {
                                        let r = creep.withdraw_all(container, resource_type);
                                        if r != ReturnCode::Ok {
                                            warn!("couldn't withdraw from container: {:?}", r);
                                            continue;
                                        }
                                        creep.memory().set("harvested_from_storage", true);
                                        is_harvested = true;
                                        break;
                                    }

                                    Structure::Storage(storage) => {
                                        let r = creep.withdraw_all(storage, resource_type);
                                        if r != ReturnCode::Ok {
                                            warn!("couldn't withdraw from storage: {:?}", r);
                                            continue;
                                        }
                                        creep.memory().set("harvested_from_storage", true);
                                        is_harvested = true;
                                        break;
                                    }

                                    Structure::Terminal(terminal) => {
                                        if harvest_kind == ResourceKind::ENERGY {
                                            let r = creep.withdraw_all(terminal, resource_type);
                                            if r != ReturnCode::Ok {
                                                warn!("couldn't withdraw from terminal: {:?}", r);
                                                continue;
                                            }
                                            creep.memory().set("harvested_from_storage", true);
                                            is_harvested = true;
                                            break;
                                        }
                                    }

                                    _ => {
                                        //do nothing
                                    }
                                }
                            }
                        }

                        if is_harvested == true {
                            break;
                        }
                    }
                }
            }

            if is_harvested == false {
                if creep.pos() == defined_target_pos.pos() {
                    debug!("already arrived, but can't harvest!!!");
                    creep.memory().del("target_pos");
                } else {
                    let res = creep.move_by_path_search_result(&path_search_result);

                    if res != ReturnCode::Ok {
                        info!("couldn't move to source: {:?}", res);
                        if res == ReturnCode::NoPath {
                            creep.memory().del("target_pos");
                        }
                    }
                }

                let mut target_pos_count = creep
                    .memory()
                    .i32("target_pos_count")
                    .unwrap_or(Some(10))
                    .unwrap_or(10);
                target_pos_count -= 1;
                if target_pos_count <= 0 {
                    creep.memory().del("target_pos");
                    creep.memory().del("target_pos_count");
                } else {
                    creep.memory().set("target_pos_count", target_pos_count);
                }
            }
        } else {
            debug!("TASK role:{:?}", role_string);

            let sources = &creep
                .room()
                .expect("room is not visible to you")
                .find(find::SOURCES_ACTIVE);

            let mut is_finished = false;

            let flee_count = creep
                .memory()
                .i32("fleeing_count")
                .unwrap_or(Some(0))
                .unwrap_or(0);

            if flee_count <= 0 {
                for source in sources.iter() {
                    if creep.pos().is_near_to(source) {
                        info!("fleeing from source!!");

                        let result = find_flee_path_from_active_source(&creep);
                        debug!(
                            "fleeing from source!!:{},{},{:?}",
                            result.ops,
                            result.cost,
                            result.load_local_path()
                        );

                        let res = creep.move_by_path_search_result(&result);
                        debug!("fleeing from source!!:{:?}", res);

                        if res == ReturnCode::Ok {
                            creep.memory().set("fleeing_count", 5);
                            is_finished = true;
                        }

                        break;
                    }
                }
            } else {
                creep.memory().set("fleeing_count", flee_count - 1);
            }

            if is_finished {
                continue;
            }

            match role_string.as_str() {
                "harvester" => {
                    harvester::run_harvester(&creep);
                }

                "harvester_spawn" => {
                    harvester::run_harvester_spawn(&creep);
                }

                "harvester_mineral" => {
                    harvester::run_harvester_mineral(&creep);
                }

                "builder" => {
                    builder::run_builder(&creep);
                }

                "upgrader" => {
                    upgrader::run_upgrader(&creep);
                }

                "repairer" => {
                    repairer::run_repairer(&creep);
                }

                "attacker" => {}

                "none" => {
                    error!("no role info");
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
    screeps::memory::root().set("num_harvester_spawn", num_harvester_spawn);
    screeps::memory::root().set("num_harvester_mineral", num_harvester_mineral);
    screeps::memory::root().set("num_repairer", num_repairer);

    screeps::memory::root().set("opt_num_attackable_short", opt_num_attackable_short);
    screeps::memory::root().set("opt_num_attackable_long", opt_num_attackable_long);

    screeps::memory::root().set("total_num", screeps::game::creeps::values().len() as i32);
    screeps::memory::root().set("cap_worker_carry", cap_worker_carry as i32);
}
