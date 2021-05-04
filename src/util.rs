use log::*;
use screeps::{ConstructionSite, FindOptions, RoomObjectProperties, Source, Structure, pathfinder::*};
use screeps::constants::find::*;
use screeps::constants::*;
use screeps::objects::HasPosition ;



fn check_transferablle(structure: &screeps::objects::Structure) -> bool
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

    let option = SearchOptions::new();

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

    let option = SearchOptions::new();

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

    let option = SearchOptions::new();

    return search_many(creep, find_item_list, option)
}