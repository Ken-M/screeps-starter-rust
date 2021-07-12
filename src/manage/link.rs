use crate::util::*;
use log::*;

use screeps::Structure;
use screeps::{
    find, game, local::ObjectId, objects::StructureLink, pathfinder::SearchResults, prelude::*,
    Attackable, Creep, Part, ResourceType, ReturnCode, RoomObjectProperties, StructureType,
};

pub fn run_link() {
    let mut min_link: Option<ObjectId<StructureLink>> = None;
    let mut max_link: Option<ObjectId<StructureLink>> = None;

    for game_structure in screeps::game::structures::values() {
        if check_my_structure(&game_structure) == true {
            match game_structure {
                Structure::Link(my_link) => {
                    debug!("check links {}", my_link.id());

                    if min_link == None {
                        min_link = Some(my_link.id());
                    } else if my_link.store_of(ResourceType::Energy)
                        < game::get_object_typed(min_link.unwrap())
                            .unwrap()
                            .unwrap()
                            .store_of(ResourceType::Energy)
                    {
                        min_link = Some(my_link.id());
                    }

                    if max_link == None {
                        max_link = Some(my_link.id());
                    } else if my_link.store_of(ResourceType::Energy)
                        > game::get_object_typed(max_link.unwrap())
                            .unwrap()
                            .unwrap()
                            .store_of(ResourceType::Energy)
                    {
                        max_link = Some(my_link.id());
                    }
                }

                _ => {}
            }
        }
    }

    info!("Link: Max:{:?}, Min:{:?}", max_link, min_link);
    if min_link == None || max_link == None || min_link == max_link {
        return;
    }

    let max_link_structure = game::get_object_typed(max_link.unwrap()).unwrap().unwrap();
    let min_link_structure = game::get_object_typed(min_link.unwrap()).unwrap().unwrap();

    let diff = max_link_structure.store_of(ResourceType::Energy)
        - min_link_structure.store_of(ResourceType::Energy);

    if diff >= 300 {
        if max_link_structure.cooldown() <= 0 {
            let r = max_link_structure.transfer_energy(&min_link_structure, Some(diff / 2));

            if r != ReturnCode::Ok {
                warn!("couldn't transfer to another link:{:?}", r);
            }
        }
    }
}
