use crate::util::*;
use log::*;

use screeps::enums::StructureObject;
use screeps::prelude::*;
use screeps::{game, local::ObjectId, objects::StructureLink, ResourceType};

pub fn run_link() {
    let mut min_link: Option<ObjectId<StructureLink>> = None;
    let mut max_link: Option<ObjectId<StructureLink>> = None;

    for game_structure in game::structures().values() {
        if check_my_structure(&game_structure) == true {
            match game_structure {
                StructureObject::StructureLink(my_link) => {
                    debug!("check links {}", my_link.id());

                    if min_link == None {
                        min_link = Some(my_link.id());
                    } else if my_link.store().get_used_capacity(Some(ResourceType::Energy))
                        < game::get_object_by_id_typed(&min_link.unwrap())
                            .unwrap()
                            .store()
                            .get_used_capacity(Some(ResourceType::Energy))
                    {
                        min_link = Some(my_link.id());
                    }

                    if max_link == None {
                        max_link = Some(my_link.id());
                    } else if my_link.store().get_used_capacity(Some(ResourceType::Energy))
                        > game::get_object_by_id_typed(&max_link.unwrap())
                            .unwrap()
                            .store()
                            .get_used_capacity(Some(ResourceType::Energy))
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

    let max_link_structure = game::get_object_by_id_typed(&max_link.unwrap()).unwrap();
    let min_link_structure = game::get_object_by_id_typed(&min_link.unwrap()).unwrap();

    let diff = max_link_structure
        .store()
        .get_used_capacity(Some(ResourceType::Energy))
        - min_link_structure
            .store()
            .get_used_capacity(Some(ResourceType::Energy));

    if diff >= 300 {
        if max_link_structure.cooldown() <= 0 {
            let r = max_link_structure.transfer_energy(&min_link_structure, Some(diff / 2));

            if let Err(e) = r {
                warn!("couldn't transfer to another link:{:?}", e);
            }
        }
    }
}
