use log::*;
use screeps::{ConstructionSite, FindOptions, RoomObjectProperties, RoomPosition, Source, Structure, StructureProperties, pathfinder::*, LookResult};
use screeps::constants::find::*;
use screeps::constants::*;
use screeps::objects::{HasPosition, Resource} ;
use screeps::local::RoomName ;
use std::{collections::HashMap, u32, u8};
use std::cmp::* ;

use lazy_static::lazy_static;
use std::sync::RwLock;

const ROOM_SIZE_X:u8 = 50 ;
const ROOM_SIZE_Y:u8 = 50 ;


type Data = HashMap<RoomName, LocalCostMatrix>;

struct GlobalInitFlag {
    init_flag:bool ,
}

lazy_static!{
    static ref CACHE: RwLock<Data> = RwLock::new(HashMap::new());
    static ref FLAG: RwLock<GlobalInitFlag> = RwLock::new(GlobalInitFlag {init_flag: true});
}

pub fn clear_init_flag()  {
    let mut flag_struct = FLAG.write().unwrap() ;
    flag_struct.init_flag = true ;
}


fn calc_room_cost(room_name: RoomName) -> MultiRoomCostResult<'static>
{
    let room = screeps::game::rooms::get(room_name) ;
    let mut cost_matrix = LocalCostMatrix::default();
    let mut is_cache_used = false ;


    {
        let cost_matrix_cache = CACHE.read().unwrap();
        let flag_struct = FLAG.read().unwrap();

        let cache_data = cost_matrix_cache.get(&room_name) ;
    
        match cache_data {
            Some(value) => {

                if flag_struct.init_flag == false {
                    // use cached matrix.
                    debug!("Room:{}, cache is used.", room_name);
                    cost_matrix = value.clone();   
                    is_cache_used = true ;      
                } else {
                    info!("Room:{}, init flag is false.", room_name);    
                }      
            }
    
            None => {
                info!("Room:{}, cache is not found.", room_name);    
            }
        }
    }

    if is_cache_used == false {

        match room {
            Some(room_obj) => {

                let structures = room_obj.find(STRUCTURES) ;

                // 地形データを反映.
                for x_pos in 0..ROOM_SIZE_X {
                    for y_pos in 0..ROOM_SIZE_Y {
                        let this_terrain = room_obj.get_terrain().get(x_pos as u32, y_pos as u32) ;

                        match this_terrain {
                            Terrain::Plain => {cost_matrix.set(x_pos, y_pos, 2);}
                            Terrain::Swamp => {cost_matrix.set(x_pos, y_pos, 10);}
                            Terrain::Wall => {cost_matrix.set(x_pos, y_pos, 0xff);}
                        }
                    }
                }

                for chk_struct in structures {

                    // Roadのコストをさげる.
                    if chk_struct.structure_type() == StructureType::Road {
                        // Favor roads over plain tiles
                        cost_matrix.set(chk_struct.pos().x() as u8, chk_struct.pos().y() as u8, 1);

                    // 通行不能なStructureはブロック.
                    } else if chk_struct.structure_type() != StructureType::Container &&
                            (chk_struct.structure_type() != StructureType::Rampart ||
                            check_my_structure(&chk_struct) == false)  {

                            // Can't walk through non-walkable buildings
                            cost_matrix.set(chk_struct.pos().x() as u8, chk_struct.pos().y() as u8, 0xff);
                    }
                }

                // 自分のものかどうかを問わず、creepのいるマスも通行不可として扱う.
                let creeps = room_obj.find(CREEPS) ;       
                // Avoid creeps in the room
                for creep in creeps {
                    cost_matrix.set(creep.pos().x() as u8, creep.pos().y() as u8, 0xff);
                }

                // active sourceの周辺はコストをあげる.
                let item_list = room_obj
                .find(SOURCES_ACTIVE);
            
                for chk_item in item_list.iter() {
                    for x_pos_offset in 0..=2 {
                        for y_pos_offset in 0..=2 {

                            let new_x_pos : i8 = min(max(chk_item.pos().x() as i8 + x_pos_offset - 1 , 0), ROOM_SIZE_X as i8 - 1) ; 
                            let new_y_pos : i8 = min(max(chk_item.pos().y() as i8 + y_pos_offset - 1 , 0), ROOM_SIZE_Y as i8 - 1) ;                           

                            let cur_cost = cost_matrix.get(new_x_pos as u8, new_y_pos as u8) ;
                            // すでに通行不可としてマークされているマスは触らない.
                            if cur_cost < 0xff {
                                let new_cost = cur_cost + 20;
                                cost_matrix.set(new_x_pos as u8, new_y_pos as u8, new_cost) ;
                            }
                        }
                    }                     
                }
            }

            None => {
            }
        }

        {
            let mut cost_matrix_cache = CACHE.write().unwrap();
            let mut flag_struct = FLAG.write().unwrap();

            cost_matrix_cache.insert(room_name, cost_matrix.clone()) ;
            flag_struct.init_flag = false ;
        }
    }

    let room_cost_result = MultiRoomCostResult::CostMatrix(cost_matrix.upload()) ;
    return room_cost_result ;
}


pub fn check_walkable(position: &RoomPosition) -> bool {
    let chk_room = screeps::game::rooms::get(position.room_name()) ;

    if let Some(room) = chk_room {
        let objects = room.look_at(position) ;

        for object in objects {

            match object {
                LookResult::Creep(creep)=>{
                    return false ;
                }

                LookResult::Terrain(terrain)=>{
                    if terrain == Terrain::Wall {
                        return false ;
                    }
                }

                LookResult::Structure(structure)=>{

                    if structure.structure_type() != StructureType::Container &&
                    (structure.structure_type() != StructureType::Rampart ||
                    check_my_structure(&structure) == false)  {
                        return false;
                    }
                }           

                _ => {
                    // check next.
                }
            }
        }
    }

    return true ;
}   



pub fn check_my_structure(structure: &screeps::objects::Structure) -> bool
{
    match structure.as_owned() {     
        Some(my_structure) => {

            return  my_structure.my() ;
        }

        None => {
            //not my structure.
            return false ;
        }
    }
}



pub fn check_transferable(structure: &screeps::objects::Structure) -> bool
{
    match structure.as_owned() {     
        Some(my_structure) => {

            if my_structure.my() == false {
                return false ;
            }

            match structure.as_transferable() {
                Some(transf) => {

                    match structure.as_has_store() {
                        Some(has_store) => {

                            if has_store.store_free_capacity(Some(ResourceType::Energy)) > 0  {
                                return true;
                            }
                        }

                        None => {
                            //no store.
                        }
                    }
                }

                None => {
                    // my_struct is not transferable
                }
            }            
        }

        None => {
            //not my structure.
        }
    }

    return false;
}

pub fn check_repairable(structure: &screeps::objects::Structure) -> bool
{
    match structure.as_owned() {            
        Some(my_structure) => {

            if my_structure.my() == false {
                return false ;
            }
        
            match structure.as_attackable() {
                Some(attackable) => {
        
                    if attackable.hits() < attackable.hits_max() {
                        return true ;
                    }
                }
        
                None => {
                    // my_struct is not transferable.
                }
            }
        }

        None => {
            match structure.as_attackable() {
                Some(attackable) => {
        
                    if attackable.hits() < attackable.hits_max() {
                        return true ;
                    }
                }
        
                None => {
                    // my_struct is not transferable.
                }
            }
        }
    }
    return false;
}


pub fn find_nearest_transfarable_item(creep: &screeps::objects::Creep) -> screeps::pathfinder::SearchResults
{
    let item_list = &creep
    .room()
    .expect("room is not visible to you")
    .find(STRUCTURES);

    let mut find_item_list = Vec::<(Structure, u32)>::new() ;

    for chk_item in item_list {
        if check_transferable(chk_item) {
            find_item_list.push((chk_item.clone(), 1));
        }
    }

    let option = SearchOptions::new()
    .room_callback(calc_room_cost)
    .plain_cost(2)
    .swamp_cost(10);

    return search_many(creep, find_item_list, option)
}

pub fn find_nearest_repairable_item(creep: &screeps::objects::Creep) -> screeps::pathfinder::SearchResults
{
    let item_list = &creep
    .room()
    .expect("room is not visible to you")
    .find(STRUCTURES);

    let mut find_item_list = Vec::<(Structure, u32)>::new() ;

    for chk_item in item_list {
        if check_repairable(chk_item) {
            find_item_list.push((chk_item.clone(), 1));
        }
    }

    let option = SearchOptions::new()
    .room_callback(calc_room_cost)
    .plain_cost(2)
    .swamp_cost(10);

    return search_many(creep, find_item_list, option)
}


pub fn find_nearest_transferable_structure(creep: &screeps::objects::Creep, structure_type: StructureType) -> screeps::pathfinder::SearchResults
{
    let item_list = &creep
    .room()
    .expect("room is not visible to you")
    .find(STRUCTURES);

    let mut find_item_list = Vec::<(Structure, u32)>::new() ;

    for chk_item in item_list {
        if chk_item.structure_type() == structure_type {
            if check_transferable(chk_item) {
                find_item_list.push((chk_item.clone(), 1));
            }
        }
    }

    let option = SearchOptions::new()
    .room_callback(calc_room_cost)
    .plain_cost(2)
    .swamp_cost(10);

    return search_many(creep, find_item_list, option)
}

pub fn find_nearest_construction_site(creep: &screeps::objects::Creep) -> screeps::pathfinder::SearchResults
{
    let item_list = &creep
    .room()
    .expect("room is not visible to you")
    .find(MY_CONSTRUCTION_SITES);

    let mut find_item_list = Vec::<(ConstructionSite, u32)>::new() ;

    for chk_item in item_list.iter() {
        find_item_list.push((chk_item.clone(), 1));
    }

    let option = SearchOptions::new()
    .room_callback(calc_room_cost)
    .plain_cost(2)
    .swamp_cost(10);

    return search_many(creep, find_item_list, option)
}

pub fn find_nearest_active_source(creep: &screeps::objects::Creep) -> screeps::pathfinder::SearchResults
{
    let item_list = &creep
    .room()
    .expect("room is not visible to you")
    .find(SOURCES_ACTIVE);

    let mut find_item_list = Vec::<(Source, u32)>::new() ;

    for chk_item in item_list.iter() {
        find_item_list.push((chk_item.clone(), 1));     
    }

    let option = SearchOptions::new()
    .room_callback(calc_room_cost)
    .plain_cost(2)
    .swamp_cost(10);

    return search_many(creep, find_item_list, option)
}

pub fn find_nearest_dropped_energy(creep: &screeps::objects::Creep) -> screeps::pathfinder::SearchResults
{
    let item_list = &creep
    .room()
    .expect("room is not visible to you")
    .find(DROPPED_RESOURCES);

    let mut find_item_list = Vec::<(Resource, u32)>::new() ;

    for chk_item in item_list.iter() {
        if chk_item.resource_type() == ResourceType::Energy {
            find_item_list.push((chk_item.clone(), 1));     
        }
    }

    let option = SearchOptions::new()
    .room_callback(calc_room_cost)
    .plain_cost(2)
    .swamp_cost(10);

    return search_many(creep, find_item_list, option)
}


pub fn find_flee_path_from_active_source(creep: &screeps::objects::Creep) -> screeps::pathfinder::SearchResults
{
    let item_list = &creep
    .room()
    .expect("room is not visible to you")
    .find(SOURCES_ACTIVE);

    let mut find_item_list = Vec::<(Source, u32)>::new() ;

    for chk_item in item_list.iter() {
        find_item_list.push((chk_item.clone(), 2));     
    }

    let option = SearchOptions::new()
    .room_callback(calc_room_cost)
    .plain_cost(2)
    .swamp_cost(10)
    .flee(true);

    return search_many(creep, find_item_list, option)
}


pub fn find_nearest_enemy(creep: &screeps::objects::Creep, range:u32) -> screeps::pathfinder::SearchResults
{
    let item_list = &creep
    .room()
    .expect("room is not visible to you")
    .find(HOSTILE_CREEPS);

    let mut find_item_list = Vec::<(screeps::objects::Creep, u32)>::new() ;

    for chk_item in item_list.iter() {
        find_item_list.push((chk_item.clone(), range));     
    }

    let option = SearchOptions::new()
    .room_callback(calc_room_cost)
    .plain_cost(2)
    .swamp_cost(10);

    return search_many(creep, find_item_list, option)
}