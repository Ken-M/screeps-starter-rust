mod builder;
mod harvester;
mod upgrader;
mod repairer;

use log::*;
use screeps::{Creep, LookConstant, Part, Position, ResourceType, ReturnCode, RoomObjectProperties, find, look::CREEPS, pathfinder::SearchResults, prelude::*};
use screeps::constants::find::*;
use crate::util::*;

use stdweb::serde ;

#[derive(PartialEq, Debug)]
enum AttackerKind {
    SHORT,
    RANGED,
    NONE
}


fn reset_source_target(creep: &Creep) -> (SearchResults, Position) {
    debug!("harvesting : reset_source_target");
    let res = find_nearest_active_source(&creep, ResourceKind::ENERGY);
    debug!("harvesting : find_nearest_active_source result:{:?}", res.load_local_path());

    if res.load_local_path().len() > 0 {

        let last_pos = *(res.load_local_path().last().unwrap());
        let json_str = serde_json::to_string(&last_pos).unwrap();
        creep.memory().set("target_pos", json_str);
        creep.memory().set("target_pos_count", 20);

        debug!("harvesting : target_pos:{:?}", creep.memory().string("target_pos"));

        let ret_position = res.load_local_path().last().unwrap().clone() ;
        return (res, ret_position) ;        
    } else {
        // storageをチェック.
        let res = find_nearest_stored_energy_source(&creep);

        if res.load_local_path().len() > 0 {

            let last_pos = *(res.load_local_path().last().unwrap());
            let json_str = serde_json::to_string(&last_pos).unwrap();
            creep.memory().set("target_pos", json_str);
            creep.memory().set("target_pos_count", 10);
    
            debug!("harvesting : target_pos:{:?}", creep.memory().string("target_pos"));
    
            let ret_position = res.load_local_path().last().unwrap().clone() ;
            return (res, ret_position) ;   

        }

        //　やむなく枯渇sourceを選ぶ.
        let res = find_nearest_source(&creep);

        if res.load_local_path().len() > 0 {

            let last_pos = *(res.load_local_path().last().unwrap());
            let json_str = serde_json::to_string(&last_pos).unwrap();
            creep.memory().set("target_pos", json_str);
            creep.memory().set("target_pos_count", 5);
    
            debug!("harvesting : target_pos:{:?}", creep.memory().string("target_pos"));
    
            let ret_position = res.load_local_path().last().unwrap().clone() ;
            return (res, ret_position) ;   
        }

        //全部ダメならとりあえずその場待機.
        let res = find_path(&creep, &creep.pos(), 0);
        return (res, creep.pos().clone()) ;  

    }          
}


fn attacker_routine(creep:&Creep, kind:&AttackerKind) -> bool {

    debug!("check enemies {}", creep.name());
    let enemies = creep
    .room()
    .expect("room is not visible to you")
    .find(HOSTILE_CREEPS);

    if enemies.len() == 0 {
        return false ;
    }

    for enemy in enemies {
        debug!("try attack enemy {}", creep.name());

        match kind {
            AttackerKind::SHORT => {
                let r = creep.attack(&enemy) ;

                if r == ReturnCode::Ok {
                    info!("attack to enemy!!");
                    return true ;
                }
            }

            AttackerKind::RANGED => {
                let r = creep.ranged_attack(&enemy) ;

                if r == ReturnCode::Ok {
                    info!("attack to enemy!!");
                    return true ;
                }  
            }

            _ => {

            }
        }
    }

    let mut range:u32 = 1;
    match kind {
        AttackerKind::SHORT => {
            range = 1 ;
        }

        AttackerKind::RANGED => {
            range = 2 ;
        }

        _ => {

        }
    }

    let res = find_nearest_enemy(&creep, range);
    debug!("go to:{:?}", res.load_local_path());

    if res.load_local_path().len() > 0 {
        let res = creep.move_by_path_search_result(&res); 
        if res == ReturnCode::Ok {
            info!("move to enemy: {:?}", res);
            return true ;
        }
    }   
    
    return false ;
}


fn get_role_and_attacker_kind(creep:&Creep) -> (String, AttackerKind) {
    let mut attacker_kind : AttackerKind = AttackerKind::NONE ;
    let role = creep.memory().string("role");
    let mut role_string =  String::from("none");

    // attacker kind check.
    let body_list = creep.body();
    for body_part in body_list{
        if body_part.part == Part::Attack {
            attacker_kind = AttackerKind::SHORT ;
            break ;
        } else if body_part.part == Part::RangedAttack {
            attacker_kind = AttackerKind::RANGED ;
            break ;                
        }
    }

    if let Ok(object) = role {
        if let Some(object) = object {
            role_string = object;
        } else {
            role_string = String::from("none");
        }
    }

    return (role_string, attacker_kind) ;
}


pub fn creep_loop() {

    let mut num_builder:i32 = 0 ;
    let mut num_harvester:i32 = 0 ;
    let mut num_upgrader:i32 = 0 ;
    let mut num_harvester_spawn:i32 = 0;
    let mut num_repairer:i32 = 0 ;

    let mut opt_num_attackable_short:i32 = 0;
    let mut opt_num_attackable_long:i32 = 0;



    for creep in screeps::game::creeps::values() {
        let name = creep.name();
        info!("checking creep {}", name);

        let mut attacker_kind : AttackerKind = AttackerKind::NONE ;
        let mut role_string =  String::from("none");

        let role_and_attacker_kind = get_role_and_attacker_kind(&creep) ;

        role_string = role_and_attacker_kind.0 ;
        attacker_kind = role_and_attacker_kind.1 ;

        info!("role:{:?}:atk:{:?}", role_string, attacker_kind); 

        match attacker_kind {
            AttackerKind::SHORT => {
                opt_num_attackable_short += 1 ;
            }

            AttackerKind::RANGED => {
                opt_num_attackable_long += 1 ;
            }

            AttackerKind::NONE => {
                //nothing.
            }
        }

        match role_string.as_str() {
            "harvester" => {
                num_harvester += 1;
            }

            "harvester_spawn" => {
                num_harvester_spawn += 1;
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
        info!("running creep {}", name);

        let mut attacker_kind : AttackerKind = AttackerKind::NONE ;
        let mut role_string =  String::from("none");

        let role_and_attacker_kind = get_role_and_attacker_kind(&creep) ;

        role_string = role_and_attacker_kind.0 ;
        attacker_kind = role_and_attacker_kind.1 ;

        match role_string.as_str() {

            "none" => {
                if num_harvester_spawn == 0 {
                    creep.memory().set("role", "harvester_spawn");
                    num_harvester_spawn += 1 ;
                    role_string = String::from("harvester_spawn") ;
                } else if num_upgrader < (screeps::game::creeps::values().len() as i32 / 10)+1 {
                    creep.memory().set("role", "upgrader");
                    num_upgrader += 1 ;
                    role_string = String::from("upgrader") ;
                } else if num_builder < (screeps::game::creeps::values().len() as i32 / 5) {
                    creep.memory().set("role", "builder");
                    num_builder += 1;
                    role_string = String::from("builder") ;        
                } else if num_repairer < (screeps::game::creeps::values().len() as i32 / 5) {
                    creep.memory().set("role", "repairer");
                    num_repairer += 1;
                    role_string = String::from("repairer") ;      
                } else {
                    creep.memory().set("role", "harvester");
                    num_harvester += 1;     
                    role_string = String::from("harvester") ;                       
                }
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
                continue ;
            }
        }

        if creep.memory().bool("harvesting") {
            if creep.store_free_capacity(Some(ResourceType::Energy)) == 0 {
                creep.memory().set("harvesting", false);
                creep.memory().del("target_pos");
            }
        } else {
            if creep.store_used_capacity(None) == 0 {
                creep.memory().set("harvesting", true);
                creep.memory().del("target_pos");
            }
        }

        if creep.memory().bool("harvesting") {
            debug!("harvesting {}", name);

            let check_string = creep.memory().string("target_pos"); 
            debug!("harvesting string{:?}", check_string); 

            let mut defined_target_pos = creep.pos() ;
            let mut path_search_result ;
            
            match check_string {
                Ok(v) => {
                    match v {
                        Some(v) => {
                            let defined_target_obj : Result<Position, serde_json::Error> = serde_json::from_str(v.as_str());

                            match defined_target_obj {
                                Ok(object) => {
                                    defined_target_pos = object ;
                                    debug!("harvesting decided:{}", defined_target_pos);
                                    path_search_result = find_path(&creep, &defined_target_pos, 0);
                                    debug!("harvesting decided path:{:?}", path_search_result.load_local_path());

                                    let look_result = creep
                                    .room()
                                    .expect("I can't see")
                                    .look_for_at_xy(CREEPS, defined_target_pos.x(), defined_target_pos.y());

                                    if look_result.len() > 0 {
                                        debug!("re-check source :{}", defined_target_pos);
                                        creep.memory().del("target_pos");

                                        let reset_result = reset_source_target(&creep) ;
                                        path_search_result = reset_result.0 ;
                                        defined_target_pos = reset_result.1 ;
                                    }
                                }

                                Err(err) => {
                                    //ロードに成功して値もあったけどDeSerializeできなかった.
                                    let reset_result = reset_source_target(&creep) ;
                                    path_search_result = reset_result.0 ;
                                    defined_target_pos = reset_result.1 ;
                                }
                            }
                        }
        
                        None => {
                            //ロードに成功したけど値がない.
                            let reset_result = reset_source_target(&creep) ;
                            path_search_result = reset_result.0 ;
                            defined_target_pos = reset_result.1 ;                      
                        }
                    }
                }

                //ロードに失敗(key自体がない).
                Err(err) => {
                    let reset_result = reset_source_target(&creep) ;
                    path_search_result = reset_result.0 ;
                    defined_target_pos = reset_result.1 ;          
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
                    && resource.resource_type() == ResourceType::Energy {
                    let r = creep.pickup(resource);
                    if r != ReturnCode::Ok {
                        warn!("couldn't pick-up: {:?}", r);
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
                        let r = creep.withdraw_all(ruin, ResourceType::Energy);
                        if r != ReturnCode::Ok {
                            warn!("couldn't harvest: {:?}", r);
                            continue;
                        }
                        is_harvested = true;
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

                for tobstone in tombstones.iter() {
                    if creep.pos().is_near_to(tobstone) {
                        let r = creep.withdraw_all(tobstone, ResourceType::Energy);
                        if r != ReturnCode::Ok {
                            warn!("couldn't harvest: {:?}", r);
                            continue;
                        }
                        is_harvested = true;
                        break;
                    } 
                }
            }
            
            //  check sources active.
            if is_harvested == false {

                let sources = &creep
                .room()
                .expect("room is not visible to you")
                .find(find::SOURCES_ACTIVE);

                for source in sources.iter() {
                    if creep.pos().is_near_to(source) {
                        let r = creep.harvest(source);
                        if r != ReturnCode::Ok {
                            warn!("couldn't harvest: {:?}", r);
                            continue;
                        }
                        is_harvested = true;
                        break;
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
                        warn!("couldn't move to source: {:?}", res);
                        if res == ReturnCode::NoPath {
                            creep.memory().del("target_pos");
                        }
                    }
                }

                let mut target_pos_count =  creep.memory().i32("target_pos_count").unwrap_or(Some(10)).unwrap_or(10); 
                target_pos_count -= 1 ;
                if target_pos_count <=0 {
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

            let mut is_finished = false ;
        
            for source in sources.iter() {
                if creep.pos().is_near_to(source) {

                    info!("fleeing from source!!");

                    let result = find_flee_path_from_active_source(&creep);
                    debug!("fleeing from source!!:{},{},{:?}", result.ops, result.cost, result.load_local_path());

                    let res = creep.move_by_path_search_result(&result);
                    debug!("fleeing from source!!:{:?}", res);

                    if res == ReturnCode::Ok {
                        is_finished = true ;
                    }

                    break ;
                } 
            }     
            
            if is_finished{
                continue ;
            }

            match role_string.as_str() {
                "harvester" => {
                    harvester::run_harvester(&creep) ;
                }

                "harvester_spawn" => {
                    harvester::run_harvester_spawn(&creep) ;
                }

                "builder" => {
                    builder::run_builder(&creep) ;
                }

                "upgrader" => {
                    upgrader::run_upgrader(&creep) ;
                }

                "repairer" => {
                    repairer::run_repairer(&creep) ;              
                }

                "attacker" => {
                    
                }

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
    screeps::memory::root().set("num_repairer", num_repairer);

    screeps::memory::root().set("opt_num_attackable_short", opt_num_attackable_short);
    screeps::memory::root().set("opt_num_attackable_long", opt_num_attackable_long);

    screeps::memory::root().set("total_num", screeps::game::creeps::values().len() as i32);   
}


