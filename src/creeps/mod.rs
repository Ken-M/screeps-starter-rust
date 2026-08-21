mod builder;
mod harvester;
mod repairer;
mod upgrader;

use crate::constants::*;
use crate::mem::{self, MemoryExt};
use crate::util::*;
use log::*;
use screeps::action_error_codes::{HarvestErrorCode, CreepMoveToErrorCode};
use screeps::enums::StructureObject;
use screeps::local::Position;
use screeps::pathfinder::SearchResults;
use screeps::prelude::*;
use screeps::{find, game, look, Creep, Part};

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
            res.path()
        );

        if res.path().len() > 0 && res.incomplete() == false {
            let last_pos = *(res.path().last().unwrap());
            let json_str = serde_json::to_string(&last_pos).unwrap();
            creep.memory().set("target_pos", json_str);
            creep.memory().set("target_pos_count", 20);
            creep.memory().set("will_harvest_from_storage", false);
            creep.memory().del("nothing_to_harvest");

            debug!(
                "harvesting : target_pos:{:?}",
                creep.memory().string("target_pos")
            );

            return (res, last_pos);
        }

        // storageをチェック.
        if *harvest_kind == ResourceKind::ENERGY {
            let res = find_nearest_stored_source(&creep, harvest_kind, true);

            if res.path().len() > 0 && res.incomplete() == false {
                let last_pos = *(res.path().last().unwrap());
                let json_str = serde_json::to_string(&last_pos).unwrap();
                creep.memory().set("target_pos", json_str);
                creep.memory().set("target_pos_count", 10);
                creep.memory().set("will_harvest_from_storage", true);
                creep.memory().del("nothing_to_harvest");

                debug!(
                    "harvesting : target_pos:{:?}",
                    creep.memory().string("target_pos")
                );

                return (res, last_pos);
            }
        }
    } else {
        // storageをチェック.
        let res = find_nearest_stored_source(&creep, harvest_kind, false);

        if res.path().len() > 0 && res.incomplete() == false {
            let last_pos = *(res.path().last().unwrap());
            let json_str = serde_json::to_string(&last_pos).unwrap();
            creep.memory().set("target_pos", json_str);
            creep.memory().set("target_pos_count", 20);
            creep.memory().set("will_harvest_from_storage", true);
            creep.memory().del("nothing_to_harvest");

            debug!(
                "harvesting : target_pos:{:?}",
                creep.memory().string("target_pos")
            );

            return (res, last_pos);
        }

        // active sourceをチェック.
        let res = find_nearest_active_source(&creep, harvest_kind, true);
        debug!(
            "harvesting : find_nearest_active_source result:{:?}",
            res.path()
        );

        if res.path().len() > 0 && res.incomplete() == false {
            let last_pos = *(res.path().last().unwrap());
            let json_str = serde_json::to_string(&last_pos).unwrap();
            creep.memory().set("target_pos", json_str);
            creep.memory().set("target_pos_count", 10);
            creep.memory().set("will_harvest_from_storage", false);
            creep.memory().del("nothing_to_harvest");

            debug!(
                "harvesting : target_pos:{:?}",
                creep.memory().string("target_pos")
            );

            return (res, last_pos);
        }
    }

    //　やむなく枯渇sourceを選ぶ.
    let res = find_nearest_exhausted_source(&creep, harvest_kind);

    if res.path().len() > 0 {
        let last_pos = *(res.path().last().unwrap());
        let json_str = serde_json::to_string(&last_pos).unwrap();
        creep.memory().set("target_pos", json_str);
        creep.memory().set("target_pos_count", 5);
        creep.memory().set("will_harvest_from_storage", true);
        creep.memory().del("nothing_to_harvest");

        debug!(
            "harvesting : target_pos:{:?}",
            creep.memory().string("target_pos")
        );

        return (res, last_pos);
    }

    //全部ダメならとりあえずその場待機.
    creep.memory().set("nothing_to_harvest", true);
    let res = find_path(&creep, &creep.pos(), 0);
    return (res, creep.pos().clone());
}

fn attacker_routine(creep: &Creep, kind: &AttackerKind) -> bool {
    debug!("check enemies {}", creep.name());
    let enemies = creep
        .room()
        .expect("room is not visible to you")
        .find(find::HOSTILE_CREEPS, None);

    if enemies.len() == 0 {
        return false;
    }

    for enemy in enemies {
        debug!("try attack enemy {}", creep.name());

        match kind {
            AttackerKind::SHORT => {
                let r = creep.attack(&enemy);

                if r.is_ok() {
                    info!("attack to enemy!!");
                    return true;
                }
            }

            AttackerKind::RANGED => {
                let r = creep.ranged_attack(&enemy);

                if r.is_ok() {
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
    debug!("go to:{:?}", res.path());

    if res.path().len() > 0 {
        let last_pos = *(res.path().last().unwrap());
        let json_str = serde_json::to_string(&last_pos).unwrap();
        creep.memory().set("target_pos", json_str);
        creep.memory().set("target_pos_count", 5);
        creep.memory().set("harvesting", true);

        let move_result = move_by_search_result(&creep, &res);
        if move_result.is_ok() {
            info!("move to enemy: {:?}", move_result);
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
        if body_part.part() == Part::Attack {
            attacker_kind = AttackerKind::SHORT;
            break;
        } else if body_part.part() == Part::RangedAttack {
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
    let mut num_carrier_mineral: i32 = 0;
    let mut num_repairer: i32 = 0;

    let mut opt_num_attackable_short: i32 = 0;
    let mut opt_num_attackable_long: i32 = 0;

    let mut cap_worker_carry: u128 = 0;

    // creep 総数は tick 内で不変なので 1 回だけ数える (JS 境界越えの節約).
    let total_creeps = game::creeps().values().count();

    for creep in game::creeps().values() {
        let name = creep.name();
        debug!("checking creep {}", name);

        let role_and_attacker_kind = get_role_and_attacker_kind(&creep);

        let role_string = role_and_attacker_kind.0;
        let attacker_kind = role_and_attacker_kind.1;

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
                cap_worker_carry += creep.store().get_capacity(None) as u128;
            }

            "harvester_spawn" => {
                num_harvester_spawn += 1;
                cap_worker_carry += creep.store().get_capacity(None) as u128;
            }

            "harvester_mineral" => {
                num_harvester_mineral += 1;
            }

            "carrier_mineral" => {
                num_carrier_mineral += 1;
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

    // if no harvester, clear role.
    if (num_harvester + num_harvester_spawn <= 2)
        && (total_creeps > (num_harvester + num_harvester_spawn) as usize)
    {
        for creep in game::creeps().values() {
            creep.memory().del("role");
        }
    }

    for creep in game::creeps().values() {
        let name = creep.name();
        info!("running creep {}, cpu:{}", name, game::cpu::get_used());

        let role_and_attacker_kind = get_role_and_attacker_kind(&creep);
        let mut harvest_kind: ResourceKind = ResourceKind::ENERGY;

        let mut is_harvester = false;

        let mut role_string = role_and_attacker_kind.0;
        let attacker_kind = role_and_attacker_kind.1;

        match role_string.as_str() {
            "none" => {
                if num_harvester_spawn < 3 {
                    creep.memory().set("role", "harvester_spawn");
                    num_harvester_spawn += 1;
                    role_string = String::from("harvester_spawn");
                    cap_worker_carry += creep.store().get_capacity(None) as u128;
                } else if num_upgrader < (total_creeps as i32 / 10) + 1 {
                    creep.memory().set("role", "upgrader");
                    num_upgrader += 1;
                    role_string = String::from("upgrader");
                } else if num_builder < (total_creeps as i32 / 6) {
                    creep.memory().set("role", "builder");
                    num_builder += 1;
                    role_string = String::from("builder");
                } else if num_repairer < (total_creeps as i32 / 6) {
                    creep.memory().set("role", "repairer");
                    num_repairer += 1;
                    role_string = String::from("repairer");
                } else if (num_harvester_mineral <= 0) && (total_creeps as i32 > 13) {
                    creep.memory().set("role", "harvester_mineral");
                    num_harvester_mineral += 1;
                    harvest_kind = ResourceKind::MINELALS;
                    role_string = String::from("harvester_mineral");
                    is_harvester = true;
                } else if cap_worker_carry < 1000 {
                    creep.memory().set("role", "harvester");
                    num_harvester += 1;
                    role_string = String::from("harvester");
                    is_harvester = true;
                    cap_worker_carry += creep.store().get_capacity(None) as u128;
                } else if let Some(_my_terminal) = creep.room().expect("I can't see").terminal() {
                    if num_carrier_mineral <= 0 {
                        creep.memory().set("role", "carrier_mineral");
                        num_carrier_mineral += 1;
                        harvest_kind = ResourceKind::MINELALS;
                        role_string = String::from("carrier_mineral");
                        is_harvester = false;
                    } else {
                        creep.memory().set("role", "repairer");
                        num_repairer += 1;
                        role_string = String::from("repairer");
                    }
                } else {
                    creep.memory().set("role", "repairer");
                    num_repairer += 1;
                    role_string = String::from("repairer");
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
        if role_string == String::from("harvester_mineral")
            || role_string == String::from("carrier_mineral")
        {
            harvest_kind = ResourceKind::MINELALS;
        }

        if creep.memory().bool("harvesting") {
            if (creep.store().get_free_capacity(None) == 0)
                || ((creep.memory().bool("nothing_to_harvest"))
                    && (creep.store().get_used_capacity(None) > 0))
            {
                creep.memory().set("harvesting", false);
                creep.memory().del("target_pos");
                creep.memory().del("will_harvest_from_storage");
                creep.memory().del("nothing_to_harvest");
            }
        } else {
            if creep.store().get_used_capacity(None) == 0 {
                creep.memory().set("harvesting", true);
                creep.memory().del("target_pos");
                creep.memory().del("harvested_from_storage");
                creep.memory().del("harvested_from_terminal");
                creep.memory().del("harvested_from_link");
                creep.memory().del("nothing_to_harvest");
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
                                        path_search_result.path()
                                    );

                                    let look_result =
                                        creep.room().expect("I can't see").look_for_at_xy(
                                            look::CREEPS,
                                            defined_target_pos.x().u8(),
                                            defined_target_pos.y().u8(),
                                        );

                                    for one_result in look_result {
                                        if one_result.name() != creep.name() {
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
            let resource_type_list = make_resoucetype_list(&harvest_kind);

            // check dropped source.
            let resources = &creep
                .room()
                .expect("room is not visible to you")
                .find(find::DROPPED_RESOURCES, None);

            for resource in resources.iter() {
                if creep.pos().is_near_to(resource.pos())
                    && check_resouce_type_kind_matching(&resource.resource_type(), &harvest_kind)
                {
                    if let Err(r) = creep.pickup(resource) {
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
                    .find(find::RUINS, None);

                for ruin in ruins.iter() {
                    if creep.pos().is_near_to(ruin.pos()) {
                        for resource_type in resource_type_list.iter() {
                            if ruin.store().get_used_capacity(Some(*resource_type)) > 0 {
                                if let Err(r) = creep.withdraw(ruin, *resource_type, None) {
                                    warn!("couldn't withdraw from RUINs: {:?}", r);
                                    break;
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
                    .find(find::TOMBSTONES, None);

                for tombstone in tombstones.iter() {
                    if creep.pos().is_near_to(tombstone.pos()) {
                        for resource_type in resource_type_list.iter() {
                            if tombstone.store().get_used_capacity(Some(*resource_type)) > 0 {
                                if let Err(r) = creep.withdraw(tombstone, *resource_type, None) {
                                    warn!("couldn't withdraw from TOMBSTONES: {:?}", r);
                                    break;
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
                    .find(find::SOURCES_ACTIVE, None);

                for source in sources.iter() {
                    if creep.pos().is_near_to(source.pos()) {
                        if let Err(r) = creep.harvest(source) {
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
                    .find(find::MINERALS, None);

                for source in sources.iter() {
                    if creep.pos().is_near_to(source.pos()) {
                        match creep.harvest(source) {
                            Ok(()) | Err(HarvestErrorCode::Tired) => {
                                // Tired は採掘クールダウン中なのでその場維持 (旧実装と同じ扱い).
                            }
                            Err(r) => {
                                info!("couldn't harvest from Minerals: {:?}", r);
                                continue;
                            }
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
                    .find(find::STRUCTURES, None);

                for structure in structures.iter() {
                    if creep.pos().is_near_to(structure.pos()) {
                        for resource_type in resource_type_list.iter() {
                            if check_stored(structure, &resource_type, 0) {
                                match structure {
                                    StructureObject::StructureContainer(container) => {
                                        if let Err(r) =
                                            creep.withdraw(container, *resource_type, None)
                                        {
                                            warn!("couldn't withdraw from container: {:?}", r);
                                            break;
                                        }
                                        creep.memory().set("harvested_from_storage", true);
                                        is_harvested = true;
                                        break;
                                    }

                                    StructureObject::StructureStorage(storage) => {
                                        if let Err(r) =
                                            creep.withdraw(storage, *resource_type, None)
                                        {
                                            warn!("couldn't withdraw from storage: {:?}", r);
                                            break;
                                        }
                                        creep.memory().set("harvested_from_storage", true);
                                        is_harvested = true;
                                        break;
                                    }

                                    StructureObject::StructureTerminal(terminal) => {
                                        if harvest_kind == ResourceKind::ENERGY {
                                            if terminal
                                                .store()
                                                .get_used_capacity(Some(*resource_type))
                                                > TERMINAL_KEEP_ENERGY
                                            {
                                                if let Err(r) = creep.withdraw(
                                                    terminal,
                                                    *resource_type,
                                                    Some(std::cmp::min(
                                                        terminal
                                                            .store()
                                                            .get_used_capacity(Some(
                                                                *resource_type,
                                                            ))
                                                            - TERMINAL_KEEP_ENERGY,
                                                        creep.store().get_free_capacity(None)
                                                            as u32,
                                                    )),
                                                ) {
                                                    warn!(
                                                        "couldn't withdraw from terminal: {:?}",
                                                        r
                                                    );
                                                    break;
                                                }
                                                creep.memory().set("harvested_from_terminal", true);
                                                is_harvested = true;
                                                break;
                                            }
                                        }
                                    }

                                    StructureObject::StructureLink(link) => {
                                        if let Err(r) = creep.withdraw(link, *resource_type, None) {
                                            warn!("couldn't withdraw from link: {:?}", r);
                                            break;
                                        }
                                        creep.memory().set("harvested_from_link", true);
                                        is_harvested = true;
                                        break;
                                    }

                                    StructureObject::StructureLab(lab) => {
                                        if harvest_kind == ResourceKind::MINELALS {
                                            if let Err(r) =
                                                creep.withdraw(lab, *resource_type, None)
                                            {
                                                warn!("couldn't withdraw from lab: {:?}", r);
                                                break;
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
                if creep.pos() == defined_target_pos {
                    debug!("already arrived, but can't harvest!!!");
                    creep.memory().del("target_pos");
                } else {
                    let res = move_by_search_result(&creep, &path_search_result);

                    if let Err(e) = res {
                        info!("couldn't move to source: {:?}", e);
                        if e == CreepMoveToErrorCode::NoPath {
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
                .find(find::SOURCES_ACTIVE, None);

            let mut is_finished = false;

            let flee_count = creep
                .memory()
                .i32("fleeing_count")
                .unwrap_or(Some(0))
                .unwrap_or(0);

            if flee_count <= 0 {
                for source in sources.iter() {
                    if creep.pos().is_near_to(source.pos()) {
                        info!("fleeing from source!!");

                        let result = find_flee_path_from_active_source(&creep);
                        debug!(
                            "fleeing from source!!:{},{},{:?}",
                            result.ops(),
                            result.cost(),
                            result.path()
                        );

                        let res = move_by_search_result(&creep, &result);
                        debug!("fleeing from source!!:{:?}", res);

                        if res.is_ok() {
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

                "carrier_mineral" => {
                    harvester::run_carrier_mineral(&creep);
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
    let root = mem::root();
    root.set("num_upgrader", num_upgrader);
    root.set("num_builder", num_builder);
    root.set("num_harvester", num_harvester);
    root.set("num_harvester_spawn", num_harvester_spawn);
    root.set("num_harvester_mineral", num_harvester_mineral);
    root.set("num_carrier_mineral", num_carrier_mineral);
    root.set("num_repairer", num_repairer);

    root.set("opt_num_attackable_short", opt_num_attackable_short);
    root.set("opt_num_attackable_long", opt_num_attackable_long);

    root.set("total_num", total_creeps as i32);
    root.set("cap_worker_carry", cap_worker_carry as i32);
}
