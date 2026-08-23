//! 「最寄りの◯◯」を経路探索で選ぶ関数群。
//! いずれも候補を集めて `search_many` に渡し、実際の歩行コストで最良を選ぶ。

use super::cache::{all_structures, find_all_rooms};
use super::pathing::{default_search_options, empty_search, search_goals};
use super::predicates::*;
use crate::constants::*;
use screeps::enums::StructureObject;
use screeps::local::Position;
use screeps::objects::{ConstructionSite, Resource};
use screeps::pathfinder::{search, search_many, SearchGoal, SearchResults};
use screeps::prelude::*;
use screeps::{find, ResourceType, StructureType};

pub fn find_nearest_repairable_item_hp(
    creep: &screeps::objects::Creep,
    threshold: u32,
) -> SearchResults {
    let item_list = all_structures();

    let mut find_item_list = Vec::<(StructureObject, u32)>::new();

    for chk_item in item_list.iter() {
        if check_repairable(&chk_item) {
            if get_hp(&chk_item).unwrap_or(0) <= threshold {
                find_item_list.push((chk_item.clone(), 3));
            }
        }
    }

    if find_item_list.is_empty() {
        return empty_search(creep);
    }

    let option = default_search_options();

    return search_many(
        creep.pos(),
        search_goals(&find_item_list).into_iter(),
        Some(option),
    );
}

pub fn find_nearest_repairable_item_except_wall_dying(
    creep: &screeps::objects::Creep,
    threshold: u128,
) -> SearchResults {
    let item_list = all_structures();

    let mut find_item_list = Vec::<(StructureObject, u32)>::new();

    for chk_item in item_list.iter() {
        if chk_item.structure_type() != StructureType::Wall {
            if check_repairable(&chk_item) {
                if get_live_tickcount(&chk_item).unwrap_or(10000) as u128 <= threshold {
                    find_item_list.push((chk_item.clone(), 3));
                }
            }
        }
    }

    if find_item_list.is_empty() {
        return empty_search(creep);
    }

    let option = default_search_options();

    return search_many(
        creep.pos(),
        search_goals(&find_item_list).into_iter(),
        Some(option),
    );
}

pub fn find_nearest_transferable_structure(
    creep: &screeps::objects::Creep,
    structure_type: &StructureType,
    resource_type: &ResourceType,
    max_cost: Option<f64>,
    capacity_rate: Option<f64>,
) -> SearchResults {
    let item_list = all_structures();

    let mut find_item_list = Vec::<(StructureObject, u32)>::new();

    for chk_item in item_list.iter() {
        if chk_item.structure_type() == *structure_type {
            if check_transferable(&chk_item, resource_type, capacity_rate) {
                find_item_list.push((chk_item.clone(), 1));
            }
        }
    }

    if find_item_list.is_empty() {
        return empty_search(creep);
    }

    let option = default_search_options().max_cost(max_cost.unwrap_or(f64::INFINITY));

    return search_many(
        creep.pos(),
        search_goals(&find_item_list).into_iter(),
        Some(option),
    );
}

pub fn find_nearest_construction_site(
    creep: &screeps::objects::Creep,
    threshold: u32,
) -> SearchResults {
    let item_list = find_all_rooms(creep, || find::MY_CONSTRUCTION_SITES);

    let mut find_item_list = Vec::<(ConstructionSite, u32)>::new();

    for chk_item in item_list.iter() {
        if (chk_item.progress_total() - chk_item.progress()) <= threshold {
            find_item_list.push((chk_item.clone(), 3));
        }
    }

    if find_item_list.is_empty() {
        return empty_search(creep);
    }

    let option = default_search_options();

    return search_many(
        creep.pos(),
        search_goals(&find_item_list).into_iter(),
        Some(option),
    );
}

pub fn find_nearest_stored_source(
    creep: &screeps::objects::Creep,
    resource_kind: &ResourceKind,
    is_2nd_check: bool,
    exclude_controller_stock: bool,
) -> SearchResults {
    let mut find_item_list = Vec::<(Position, u32)>::new();
    let resource_type_list = make_resoucetype_list(&resource_kind);

    if is_2nd_check == false {
        // dropped resource.
        let item_list = find_all_rooms(creep, || find::DROPPED_RESOURCES);

        for chk_item in item_list.iter() {
            for resource in resource_type_list.iter() {
                if chk_item.resource_type() == *resource {
                    find_item_list.push((chk_item.pos(), 1));
                    break;
                }
            }
        }

        // TOMBSTONES.
        let item_list = find_all_rooms(creep, || find::TOMBSTONES);

        for chk_item in item_list.iter() {
            for resource in resource_type_list.iter() {
                if chk_item.store().get_used_capacity(Some(*resource)) > 0 {
                    find_item_list.push((chk_item.pos(), 1));
                    break;
                }
            }
        }

        // RUINs.
        let item_list = find_all_rooms(creep, || find::RUINS);

        for chk_item in item_list.iter() {
            for resource in resource_type_list.iter() {
                if chk_item.store().get_used_capacity(Some(*resource)) > 0 {
                    find_item_list.push((chk_item.pos(), 1));
                    break;
                }
            }
        }
    }

    if find_item_list.is_empty() {
        let item_list = all_structures();

        for chk_item in item_list.iter() {
            // controller 脇の補給 container は hauler の汲み出し先ではない
            // (hauler が配達した端から引き出す空回りを防ぐ)。
            if exclude_controller_stock && is_controller_stock(chk_item) {
                continue;
            }
            if chk_item.structure_type() == StructureType::Container
                || chk_item.structure_type() == StructureType::Storage
                || chk_item.structure_type() == StructureType::Link
                || ((chk_item.structure_type() == StructureType::Terminal)
                    && (*resource_kind == ResourceKind::ENERGY))
                || ((chk_item.structure_type() == StructureType::Lab)
                    && (*resource_kind == ResourceKind::MINELALS))
            {
                if check_my_structure(chk_item)
                    || (chk_item.structure_type() == StructureType::Container)
                {
                    for resource_type in resource_type_list.iter() {
                        let mut keep_amount = 0 as u32;
                        if chk_item.structure_type() == StructureType::Terminal {
                            keep_amount = TERMINAL_KEEP_ENERGY;
                        }

                        if check_stored(chk_item, resource_type, keep_amount) {
                            let mut dist = 1;
                            if chk_item.structure_type() == StructureType::Container {
                                dist = 0;
                            }

                            find_item_list.push((chk_item.pos(), dist));
                            break;
                        }
                    }
                }
            }
        }
    }

    if find_item_list.is_empty() {
        return empty_search(creep);
    }

    let option = default_search_options();

    return search_many(
        creep.pos(),
        find_item_list
            .into_iter()
            .map(|(pos, range)| SearchGoal::new(pos, range)),
        Some(option),
    );
}

pub fn find_nearest_dropped_resource(
    creep: &screeps::objects::Creep,
    resource_kind: ResourceKind,
) -> SearchResults {
    let item_list = find_all_rooms(creep, || find::DROPPED_RESOURCES);

    let mut find_item_list = Vec::<(Resource, u32)>::new();
    let resource_type_list = make_resoucetype_list(&resource_kind);

    for chk_item in item_list.iter() {
        for resource_type in resource_type_list.iter() {
            if chk_item.resource_type() == *resource_type {
                find_item_list.push((chk_item.clone(), 1));
                break;
            }
        }
    }

    if find_item_list.is_empty() {
        return empty_search(creep);
    }

    let option = default_search_options();

    return search_many(
        creep.pos(),
        search_goals(&find_item_list).into_iter(),
        Some(option),
    );
}

pub fn find_nearest_enemy(creep: &screeps::objects::Creep, range: u32) -> SearchResults {
    let item_list = creep
        .room()
        .expect("room is not visible to you")
        .find(find::HOSTILE_CREEPS, None);

    // not nessesary to find another room hostile_creeps.

    let mut find_item_list = Vec::<(screeps::objects::Creep, u32)>::new();

    for chk_item in item_list.iter() {
        find_item_list.push((chk_item.clone(), range));
    }

    if find_item_list.is_empty() {
        return empty_search(creep);
    }

    let option = default_search_options();

    return search_many(
        creep.pos(),
        search_goals(&find_item_list).into_iter(),
        Some(option),
    );
}

pub fn find_nearest_room_controler(creep: &screeps::objects::Creep) -> SearchResults {
    let item_list = all_structures();

    let mut find_item_list = Vec::<(StructureObject, u32)>::new();

    for chk_item in item_list.iter() {
        if chk_item.structure_type() == StructureType::Controller {
            if check_my_structure(chk_item) == true {
                find_item_list.push((chk_item.clone(), 3));
            }
        }
    }

    if find_item_list.is_empty() {
        return empty_search(creep);
    }

    let option = default_search_options();

    return search_many(
        creep.pos(),
        search_goals(&find_item_list).into_iter(),
        Some(option),
    );
}

pub fn find_path(
    creep: &screeps::objects::Creep,
    target_pos: &Position,
    range: u32,
) -> SearchResults {
    let option = default_search_options();

    return search(creep.pos(), *target_pos, range, Some(option));
}
