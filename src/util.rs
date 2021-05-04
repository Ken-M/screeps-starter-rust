use log::*;
use screeps::{ConstructionSite, FindOptions, RoomObjectProperties, Source, Structure, StructureProperties, pathfinder::*};
use screeps::constants::find::*;
use screeps::constants::*;
use screeps::objects::HasPosition ;
use screeps::local::RoomName ;
use screeps::pathfinder::* ;
use std::collections::HashMap;

use lazy_static::lazy_static;
use std::sync::RwLock;

type Data = HashMap<RoomName, CostMatrix<'static>>;

lazy_static!{
    static ref CACHE: RwLock<Data> = RwLock::new(HashMap::new());
}


fn calc_room_cost(room_name: RoomName) -> MultiRoomCostResult<'static>
{
    let room = screeps::game::rooms::get(room_name) ;
    let mut cost_matrix = CostMatrix::default();

    //{
    //    let cost_matrix_cache = CACHE.read().unwrap();
    //    let cache_data = cost_matrix_cache.get(&room_name) ;
    //
    //    match cache_data {
    //        Some(value) => {
    //            let data = value.clone();                
    //        }
    //
    //        None => {
    //
    //        }
    //    }
    //}

    match room {
        Some(room_obj) => {

            let structures = room_obj.find(STRUCTURES) ;

            #[derive(Clone, Copy)]
            struct LocalPosition{
                pos_x: u8,
                pos_y: u8
            } ;

            impl LocalPosition {
                fn set_x(&mut self, x: u8) {self.pos_x = x;}
                fn set_y(&mut self, y: u8) {self.pos_y = y;}
            }

            impl HasLocalPosition for LocalPosition {
                fn x(&self) -> u8 {return self.pos_x;}
                fn y(&self) -> u8 {return self.pos_y;}
            }

            for chk_struct in structures {
                let mut local_pos  = LocalPosition{pos_x:0, pos_y:0} ;
                local_pos.set_x(chk_struct.pos().x() as u8) ;
                local_pos.set_y(chk_struct.pos().y() as u8) ;

                if chk_struct.structure_type() == StructureType::Road {
                    // Favor roads over plain tiles
                    cost_matrix.set(local_pos.clone(), 1);

                } else if chk_struct.structure_type() != StructureType::Container &&
                          chk_struct.structure_type() != StructureType::Rampart ||
                          check_my_structure(&chk_struct) == false  {

                        // Can't walk through non-walkable buildings
                        cost_matrix.set(local_pos.clone(), 0xff);
                }
            }

            let creeps = room_obj.find(CREEPS) ;       
            // Avoid creeps in the room
            for creep in creeps {
                let mut local_pos  = LocalPosition{pos_x:0, pos_y:0} ;
                local_pos.set_x(creep.pos().x() as u8) ;
                local_pos.set_y(creep.pos().y() as u8) ;

                cost_matrix.set(local_pos.clone(), 0xff);
            }
        }

        None => {
        }
    }

    let room_cost_result = MultiRoomCostResult::CostMatrix(cost_matrix) ;
    return room_cost_result ;
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



pub fn check_transferablle(structure: &screeps::objects::Structure) -> bool
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
        if check_transferablle(chk_item) {
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


pub fn find_nearest_spawn(creep: &screeps::objects::Creep) -> screeps::pathfinder::SearchResults
{
    let item_list = &creep
    .room()
    .expect("room is not visible to you")
    .find(STRUCTURES);

    let mut find_item_list = Vec::<(Structure, u32)>::new() ;

    for chk_item in item_list {
        if chk_item.structure_type() == StructureType::Spawn {
            if check_transferablle(chk_item) {
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