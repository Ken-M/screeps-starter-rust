use crate::constants::*;
use log::*;
use screeps::constants::find::*;
use screeps::constants::*;
use screeps::local::Position;
use screeps::local::RoomName;
use screeps::objects::{HasPosition, Resource};
use screeps::{
    game, pathfinder::*, ConstructionSite, HasStore, LookResult, RoomObjectProperties,
    RoomPosition, Source, Structure, StructureProperties,
};

use std::cmp::*;
use std::collections::HashSet;
use std::{collections::HashMap, u32, u8};

use lazy_static::lazy_static;
use std::sync::RwLock;

const ROOM_SIZE_X: u8 = 50;
const ROOM_SIZE_Y: u8 = 50;

type Data = HashMap<RoomName, LocalCostMatrix>;

type ConstructionProgressAverage = HashMap<RoomName, u128>;
type StructureHpAverage = HashMap<RoomName, u128>;

type ConstructionProgressMin = HashMap<RoomName, u128>;
type StructureHpMin = HashMap<RoomName, u128>;

type RoomHashSet = HashSet<RoomName>;

struct GlobalInitFlag {
    init_flag: bool,
}

lazy_static! {
    static ref MAP_CACHE: RwLock<Data> = RwLock::new(HashMap::new());
    static ref CONSTRUCTION_PROGRESS_AVERAGE_CACHE: RwLock<ConstructionProgressAverage> =
        RwLock::new(HashMap::new());
    static ref STRUCTURE_HP_AVERAGE_CACHE: RwLock<StructureHpAverage> = RwLock::new(HashMap::new());
    static ref CONSTRUCTION_PROGRESS_MIN_CACHE: RwLock<ConstructionProgressMin> =
        RwLock::new(HashMap::new());
    static ref STRUCTURE_HP_MIN_CACHE: RwLock<StructureHpMin> = RwLock::new(HashMap::new());
}

pub fn clear_init_flag() {
    let mut cost_matrix_cache = MAP_CACHE.write().unwrap();
    cost_matrix_cache.clear();

    let mut construction_progress_average = CONSTRUCTION_PROGRESS_AVERAGE_CACHE.write().unwrap();
    construction_progress_average.clear();

    let mut structure_hp_average = STRUCTURE_HP_AVERAGE_CACHE.write().unwrap();
    structure_hp_average.clear();

    let mut construction_progress_min = CONSTRUCTION_PROGRESS_MIN_CACHE.write().unwrap();
    construction_progress_min.clear();

    let mut structure_hp_min = STRUCTURE_HP_MIN_CACHE.write().unwrap();
    structure_hp_min.clear();
}

#[derive(PartialEq, Debug)]
pub enum ResourceKind {
    ENERGY,
    MINELALS,
    POWER,
    COMMODITIES,
}

pub fn calc_average(room_name: &RoomName) {
    let mut construction_progress_average = CONSTRUCTION_PROGRESS_AVERAGE_CACHE.write().unwrap();
    let mut structure_hp_average = STRUCTURE_HP_AVERAGE_CACHE.write().unwrap();

    let mut construction_progress_min = CONSTRUCTION_PROGRESS_MIN_CACHE.write().unwrap();
    let mut structure_hp_min = STRUCTURE_HP_MIN_CACHE.write().unwrap();

    let room = screeps::game::rooms::get(*room_name);

    match room {
        Some(room_obj) => {
            let structures = room_obj.find(find::STRUCTURES);
            let construction_sites = room_obj.find(MY_CONSTRUCTION_SITES);

            let mut total_hp: u128 = 0;
            let mut hp_min: u128 = 0;

            let mut struct_count: u128 = 0;

            for chk_struct in structures {
                let cur_hp = get_hp(&chk_struct);

                match cur_hp {
                    Some(hp) => {
                        struct_count += 1 as u128;
                        total_hp += hp as u128;

                        if (hp_min > hp as u128) || (hp_min == 0) {
                            hp_min = hp as u128;
                        }
                    }
                    None => {}
                }
            }

            let mut sum_of_progress: u128 = 0;
            let mut progress_min: u128 = 0;
            let mut construction_count: u128 = 0;

            for construction_site in construction_sites.iter() {
                let left_progress = construction_site.progress_total() as u128
                    - construction_site.progress() as u128;
                sum_of_progress += left_progress;
                construction_count += 1;

                if (progress_min > left_progress) || (progress_min == 0) {
                    progress_min = left_progress;
                }
            }

            if struct_count > 0 {
                structure_hp_average.insert(*room_name, total_hp / struct_count);

                structure_hp_min.insert(*room_name, hp_min);
                info!(
                    "{:?}: structure_hp_average:{:?}/min:{:?}",
                    room_name,
                    total_hp / struct_count,
                    hp_min
                );
            } else {
                structure_hp_average.insert(*room_name, 0);
                structure_hp_min.insert(*room_name, 0);
            }

            if construction_count > 0 {
                construction_progress_average
                    .insert(*room_name, sum_of_progress / construction_count);
                construction_progress_min.insert(*room_name, progress_min);

                info!(
                    "{:?}: construction_progress_average:{:?}:min:{:?}",
                    *room_name,
                    sum_of_progress / construction_count,
                    progress_min
                );
            } else {
                construction_progress_average.insert(*room_name, 0);
            }
        }

        None => {}
    }
}

pub fn get_hp_average(room_name: &RoomName) -> (u128, u128) {
    {
        let structure_hp_average = STRUCTURE_HP_AVERAGE_CACHE.read().unwrap();
        let cache_value = structure_hp_average.get(&room_name);

        let structure_hp_min = STRUCTURE_HP_MIN_CACHE.read().unwrap();
        let cache_value_min = structure_hp_min.get(&room_name);

        match cache_value {
            Some(value) => {
                // use cached value.

                match cache_value_min {
                    Some(value_min) => {
                        return (*value, *value_min);
                    }

                    None => {}
                }
            }
            None => {}
        }
    }

    calc_average(room_name);

    {
        let structure_hp_average = STRUCTURE_HP_AVERAGE_CACHE.read().unwrap();
        let cache_value = structure_hp_average.get(&room_name);

        let structure_hp_min = STRUCTURE_HP_MIN_CACHE.read().unwrap();
        let cache_value_min = structure_hp_min.get(&room_name);

        match cache_value {
            Some(value) => {
                // use cached value.

                match cache_value_min {
                    Some(value_min) => {
                        return (*value, *value_min);
                    }

                    None => {}
                }
            }
            None => {}
        }
    }

    return (0, 0);
}

pub fn get_construction_progress_average(room_name: &RoomName) -> (u128, u128) {
    {
        let construction_progress_average = CONSTRUCTION_PROGRESS_AVERAGE_CACHE.read().unwrap();
        let cache_value = construction_progress_average.get(&room_name);

        let construction_progress_min = CONSTRUCTION_PROGRESS_AVERAGE_CACHE.read().unwrap();
        let cache_value_min = construction_progress_min.get(&room_name);

        match cache_value {
            Some(value) => {
                // use cached value.

                match cache_value_min {
                    Some(value_min) => {
                        return (*value, *value_min);
                    }

                    None => {}
                }
            }
            None => {}
        }
    }

    calc_average(room_name);

    {
        let construction_progress_average = CONSTRUCTION_PROGRESS_AVERAGE_CACHE.read().unwrap();
        let cache_value = construction_progress_average.get(&room_name);

        let construction_progress_min = CONSTRUCTION_PROGRESS_AVERAGE_CACHE.read().unwrap();
        let cache_value_min = construction_progress_min.get(&room_name);

        match cache_value {
            Some(value) => {
                // use cached value.

                match cache_value_min {
                    Some(value_min) => {
                        return (*value, *value_min);
                    }

                    None => {}
                }
            }
            None => {}
        }
    }

    return (0, 0);
}

fn calc_room_cost(room_name: RoomName) -> MultiRoomCostResult<'static> {
    let room = screeps::game::rooms::get(room_name);
    let mut cost_matrix = LocalCostMatrix::default();
    let mut is_cache_used = false;

    {
        let cost_matrix_cache = MAP_CACHE.read().unwrap();
        let cache_data = cost_matrix_cache.get(&room_name);

        match cache_data {
            Some(value) => {
                // use cached matrix.
                debug!("Room:{}, cache is used.", room_name);
                cost_matrix = value.clone();
                is_cache_used = true;
            }

            None => {
                info!("Room:{}, cache is not found.", room_name);
            }
        }
    }

    if is_cache_used == false {
        match room {
            Some(room_obj) => {
                let structures = room_obj.find(find::STRUCTURES);

                for chk_struct in structures {
                    // Roadのコストをさげる.
                    if chk_struct.structure_type() == StructureType::Road {
                        // Favor roads over plain tiles
                        cost_matrix.set(chk_struct.pos().x() as u8, chk_struct.pos().y() as u8, 1);

                    // 通行不能なStructureはブロック.
                    } else if chk_struct.structure_type() != StructureType::Container
                        && (chk_struct.structure_type() != StructureType::Rampart
                            || check_my_structure(&chk_struct) == false)
                    {
                        // Can't walk through non-walkable buildings
                        cost_matrix.set(
                            chk_struct.pos().x() as u8,
                            chk_struct.pos().y() as u8,
                            0xff,
                        );
                    }
                }

                // 自分のものかどうかを問わず、creepのいるマスも通行不可として扱う.
                let creeps = room_obj.find(find::CREEPS);
                // Avoid creeps in the room
                for creep in creeps {
                    cost_matrix.set(creep.pos().x() as u8, creep.pos().y() as u8, 0xff);
                }

                // ConstructionSiteの通行不可なものをマーク.
                let construction_sites = room_obj.find(MY_CONSTRUCTION_SITES);
                for construction_site in construction_sites {
                    if construction_site.structure_type() != StructureType::Road
                        && construction_site.structure_type() != StructureType::Container
                        && construction_site.structure_type() != StructureType::Rampart
                    {
                        // Can't walk through non-walkable construction sites.
                        cost_matrix.set(
                            construction_site.pos().x() as u8,
                            construction_site.pos().y() as u8,
                            0xff,
                        );
                    }
                }

                // active sourceの周辺はコストをあげる.
                let item_list = room_obj.find(SOURCES_ACTIVE);

                for chk_item in item_list.iter() {
                    for x_pos_offset in 0..=2 {
                        for y_pos_offset in 0..=2 {
                            let new_x_pos: i8 = min(
                                max(chk_item.pos().x() as i8 + x_pos_offset - 1, 0),
                                ROOM_SIZE_X as i8 - 1,
                            );
                            let new_y_pos: i8 = min(
                                max(chk_item.pos().y() as i8 + y_pos_offset - 1, 0),
                                ROOM_SIZE_Y as i8 - 1,
                            );

                            let cur_cost = cost_matrix.get(new_x_pos as u8, new_y_pos as u8);
                            // すでに通行不可としてマークされているマスは触らない.
                            if cur_cost < 0xff {
                                if room_obj
                                    .get_terrain()
                                    .get(new_x_pos as u32, new_y_pos as u32)
                                    != Terrain::Wall
                                {
                                    let new_cost = 11;
                                    cost_matrix.set(new_x_pos as u8, new_y_pos as u8, new_cost);
                                }
                            }
                        }
                    }
                }
            }

            None => {
                //デフォルトのまま.
            }
        }

        {
            let mut cost_matrix_cache = MAP_CACHE.write().unwrap();
            cost_matrix_cache.insert(room_name, cost_matrix.clone());
        }
    }

    let room_cost_result = MultiRoomCostResult::CostMatrix(cost_matrix.upload());
    return room_cost_result;
}

pub fn check_walkable(position: &RoomPosition) -> bool {
    let chk_room = screeps::game::rooms::get(position.room_name());

    if let Some(room) = chk_room {
        let objects = room.look_at(position);

        for object in objects {
            match object {
                LookResult::Creep(_creep) => {
                    return false;
                }

                LookResult::Terrain(terrain) => {
                    if terrain == Terrain::Wall {
                        return false;
                    }
                }

                LookResult::Structure(structure) => {
                    if structure.structure_type() != StructureType::Container
                        && (structure.structure_type() != StructureType::Rampart
                            || check_my_structure(&structure) == false)
                    {
                        return false;
                    }
                }

                _ => {
                    // check next.
                }
            }
        }
    }

    return true;
}

pub fn check_my_structure(structure: &screeps::objects::Structure) -> bool {
    match structure.as_owned() {
        Some(my_structure) => {
            return my_structure.my();
        }

        None => {
            //not my structure.
            return false;
        }
    }
}

pub fn check_transferable(
    structure: &screeps::objects::Structure,
    resource_type: &ResourceType,
    capacity_rate: Option<f64>,
) -> bool {
    match structure.as_owned() {
        Some(my_structure) => {
            if my_structure.my() == false {
                return false;
            }

            match structure.as_transferable() {
                Some(_transf) => {
                    match structure.as_has_store() {
                        Some(has_store) => {
                            if has_store.store_free_capacity(Some(*resource_type))
                                > (has_store.store_capacity(Some(*resource_type)) as f64
                                    * capacity_rate.unwrap_or(0 as f64))
                                    as i32
                            {
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
            match structure.as_transferable() {
                Some(_transf) => {
                    match structure.as_has_store() {
                        Some(has_store) => {
                            if has_store.store_free_capacity(Some(*resource_type)) > 0 {
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
    }

    return false;
}

pub fn check_repairable(structure: &screeps::objects::Structure) -> bool {
    match structure.as_owned() {
        Some(my_structure) => {
            if my_structure.my() == false {
                return false;
            }

            match structure.as_attackable() {
                Some(attackable) => {
                    if attackable.hits() < attackable.hits_max() {
                        if attackable.hits() > 0 {
                            return true;
                        }
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
                        if attackable.hits() > 0 {
                            return true;
                        }
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

pub fn get_repairable_hp(structure: &screeps::objects::Structure) -> Option<u32> {
    match structure.as_owned() {
        Some(my_structure) => {
            if my_structure.my() == false {
                return None;
            }

            match structure.as_attackable() {
                Some(attackable) => {
                    if attackable.hits() > 0 {
                        return Some(attackable.hits_max() - attackable.hits());
                    } else {
                        return None;
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
                    if attackable.hits() > 0 {
                        return Some(attackable.hits_max() - attackable.hits());
                    } else {
                        return None;
                    }
                }

                None => {
                    // my_struct is not transferable.
                }
            }
        }
    }
    return None;
}

pub fn get_live_tickcount(structure: &screeps::objects::Structure) -> Option<u128> {
    let room_obj = structure.room().expect("room is not visible to you");

    match structure.as_owned() {
        Some(my_structure) => {
            if my_structure.my() == false {
                return None;
            }

            match structure.as_attackable() {
                Some(attackable) => {
                    let this_terrain = room_obj
                        .get_terrain()
                        .get(structure.pos().x(), structure.pos().y());

                    match structure {
                        Structure::Road(_road) => match this_terrain {
                            Terrain::Plain => {
                                return Some(
                                    ROAD_DECAY_TIME as u128
                                        * (attackable.hits() as u128 / ROAD_DECAY_AMOUNT as u128),
                                );
                            }
                            Terrain::Swamp => {
                                return Some(
                                    ROAD_DECAY_TIME as u128
                                        * (attackable.hits() as u128
                                            / (ROAD_DECAY_AMOUNT as u128
                                                * CONSTRUCTION_COST_ROAD_SWAMP_RATIO as u128)),
                                );
                            }
                            Terrain::Wall => {
                                return Some(
                                    ROAD_DECAY_TIME as u128
                                        * (attackable.hits() as u128
                                            / (ROAD_DECAY_AMOUNT as u128
                                                * CONSTRUCTION_COST_ROAD_WALL_RATIO as u128)),
                                );
                            }
                        },

                        Structure::Container(_container) => {
                            return Some(
                                CONTAINER_DECAY_TIME_OWNED as u128
                                    * (attackable.hits() as u128 / CONTAINER_DECAY as u128),
                            );
                        }

                        Structure::Rampart(_ramport) => {
                            return Some(
                                RAMPART_DECAY_TIME as u128
                                    * (attackable.hits() as u128 / RAMPART_DECAY_AMOUNT as u128),
                            );
                        }

                        _ => {}
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
                    let this_terrain = room_obj
                        .get_terrain()
                        .get(structure.pos().x(), structure.pos().y());

                    match structure {
                        Structure::Road(_road) => match this_terrain {
                            Terrain::Plain => {
                                return Some(
                                    ROAD_DECAY_TIME as u128
                                        * (attackable.hits() as u128 / ROAD_DECAY_AMOUNT as u128),
                                );
                            }
                            Terrain::Swamp => {
                                return Some(
                                    ROAD_DECAY_TIME as u128
                                        * (attackable.hits() as u128
                                            / (ROAD_DECAY_AMOUNT as u128
                                                * CONSTRUCTION_COST_ROAD_SWAMP_RATIO as u128)),
                                );
                            }
                            Terrain::Wall => {
                                return Some(
                                    ROAD_DECAY_TIME as u128
                                        * (attackable.hits() as u128
                                            / (ROAD_DECAY_AMOUNT as u128
                                                * CONSTRUCTION_COST_ROAD_WALL_RATIO as u128)),
                                );
                            }
                        },

                        Structure::Container(_container) => {
                            return Some(
                                CONTAINER_DECAY_TIME_OWNED as u128
                                    * (attackable.hits() as u128 / CONTAINER_DECAY as u128),
                            );
                        }

                        Structure::Rampart(_ramport) => {
                            return Some(
                                RAMPART_DECAY_TIME as u128
                                    * (attackable.hits() as u128 / RAMPART_DECAY_AMOUNT as u128),
                            );
                        }

                        _ => {}
                    }
                }

                None => {
                    // my_struct is not transferable.
                }
            }
        }
    }
    return None;
}

pub fn get_hp(structure: &screeps::objects::Structure) -> Option<u32> {
    match structure.as_owned() {
        Some(my_structure) => {
            if my_structure.my() == false {
                return None;
            }

            match structure.as_attackable() {
                Some(attackable) => {
                    if (attackable.hits() > 0) && (attackable.hits() < attackable.hits_max()) {
                        return Some((attackable.hits()) as u32);
                    } else {
                        return None;
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
                    if (attackable.hits() > 0) && (attackable.hits() < attackable.hits_max()) {
                        return Some((attackable.hits()) as u32);
                    } else {
                        return None;
                    }
                }

                None => {
                    // my_struct is not transferable.
                }
            }
        }
    }
    return None;
}

pub fn check_repairable_hp(structure: &screeps::objects::Structure, hp_th: u32) -> bool {
    match structure.as_owned() {
        Some(my_structure) => {
            if my_structure.my() == false {
                return false;
            }

            match structure.as_attackable() {
                Some(attackable) => {
                    if attackable.hits() < attackable.hits_max() {
                        if (attackable.hits() < hp_th) && (attackable.hits() > 0) {
                            return true;
                        }
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
                        if (attackable.hits() < hp_th) && (attackable.hits() > 0) {
                            return true;
                        }
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
pub fn check_stored(
    structure: &screeps::objects::Structure,
    resource_type: &ResourceType,
    keep_amount: u32,
) -> bool {
    match structure.as_has_store() {
        Some(storage) => {
            if storage.store_of(*resource_type) > keep_amount {
                return true;
            }
        }

        None => {}
    }
    return false;
}

pub fn make_resoucetype_list(resource_kind: &ResourceKind) -> Vec<ResourceType> {
    match resource_kind {
        ResourceKind::ENERGY => {
            let templist = vec![ResourceType::Energy];
            return templist;
        }

        ResourceKind::MINELALS => {
            let templist = vec![
                ResourceType::Hydrogen,
                ResourceType::Oxygen,
                ResourceType::Utrium,
                ResourceType::Lemergium,
                ResourceType::Keanium,
                ResourceType::Zynthium,
                ResourceType::Catalyst,
                ResourceType::Ghodium,
                ResourceType::Hydroxide,
                ResourceType::ZynthiumKeanite,
                ResourceType::UtriumLemergite,
                ResourceType::UtriumHydride,
                ResourceType::UtriumOxide,
                ResourceType::KeaniumHydride,
                ResourceType::KeaniumOxide,
                ResourceType::LemergiumHydride,
                ResourceType::LemergiumOxide,
                ResourceType::ZynthiumHydride,
                ResourceType::ZynthiumOxide,
                ResourceType::GhodiumHydride,
                ResourceType::GhodiumOxide,
                ResourceType::UtriumAcid,
                ResourceType::UtriumAlkalide,
                ResourceType::KeaniumAcid,
                ResourceType::KeaniumAlkalide,
                ResourceType::LemergiumAcid,
                ResourceType::LemergiumAlkalide,
                ResourceType::ZynthiumAcid,
                ResourceType::ZynthiumAlkalide,
                ResourceType::GhodiumAcid,
                ResourceType::GhodiumAlkalide,
                ResourceType::CatalyzedUtriumAcid,
                ResourceType::CatalyzedUtriumAlkalide,
                ResourceType::CatalyzedKeaniumAcid,
                ResourceType::CatalyzedKeaniumAlkalide,
                ResourceType::CatalyzedLemergiumAcid,
                ResourceType::CatalyzedLemergiumAlkalide,
                ResourceType::CatalyzedZynthiumAcid,
                ResourceType::CatalyzedZynthiumAlkalide,
                ResourceType::CatalyzedGhodiumAcid,
                ResourceType::CatalyzedGhodiumAlkalide,
            ];

            return templist;
        }

        ResourceKind::COMMODITIES => {
            let templist = vec![
                ResourceType::Silicon,
                ResourceType::Metal,
                ResourceType::Biomass,
                ResourceType::Mist,
                ResourceType::UtriumBar,
                ResourceType::LemergiumBar,
                ResourceType::ZynthiumBar,
                ResourceType::KeaniumBar,
                ResourceType::GhodiumMelt,
                ResourceType::Oxidant,
                ResourceType::Reductant,
                ResourceType::Purifier,
                ResourceType::Battery,
                ResourceType::Composite,
                ResourceType::Crystal,
                ResourceType::Liquid,
                ResourceType::Wire,
                ResourceType::Switch,
                ResourceType::Transistor,
                ResourceType::Microchip,
                ResourceType::Circuit,
                ResourceType::Device,
                ResourceType::Cell,
                ResourceType::Phlegm,
                ResourceType::Tissue,
                ResourceType::Muscle,
                ResourceType::Organoid,
                ResourceType::Organism,
                ResourceType::Alloy,
                ResourceType::Tube,
                ResourceType::Fixtures,
                ResourceType::Frame,
                ResourceType::Hydraulics,
                ResourceType::Machine,
                ResourceType::Condensate,
                ResourceType::Concentrate,
                ResourceType::Extract,
                ResourceType::Spirit,
                ResourceType::Emanation,
                ResourceType::Essence,
            ];
            return templist;
        }

        ResourceKind::POWER => {
            let templist = vec![ResourceType::Power, ResourceType::Ops];
            return templist;
        }
    }
}

pub fn check_resouce_type_kind_matching(
    resource_type: &ResourceType,
    resource_kind: &ResourceKind,
) -> bool {
    let resrouce_type_list = make_resoucetype_list(resource_kind);
    for chk_resource_type in resrouce_type_list {
        if *resource_type == chk_resource_type {
            return true;
        }
    }

    return false;
}

pub fn find_nearest_transfarable_item(
    creep: &screeps::objects::Creep,
    resource_kind: &ResourceKind,
    is_except_storages: &bool,
    is_except_terminal: &bool,
    is_except_link: &bool,
) -> screeps::pathfinder::SearchResults {
    let item_list = &mut creep
        .room()
        .expect("room is not visible to you")
        .find(find::STRUCTURES);

    {
        let room_list = game::rooms::values();

        for room_item in room_list.iter() {
            if room_item.name() != *(&creep.room().expect("room is not visible to you").name()) {
                let local_list = room_item.find(find::STRUCTURES);
                item_list.extend(local_list);
            }
        }
    }

    let mut find_item_list = Vec::<(Structure, u32)>::new();
    let resource_type_list = make_resoucetype_list(resource_kind);

    for chk_item in item_list.iter() {
        if chk_item.structure_type() == StructureType::Lab
            && *resource_kind == ResourceKind::MINELALS
        {
            continue;
        }

        if *is_except_storages == true
            && (chk_item.structure_type() == StructureType::Container
                || chk_item.structure_type() == StructureType::Storage)
        {
            //前回storage系からresourceを調達している場合はもどさないようにする.

            continue;
        }

        if *is_except_terminal == true
            && (*resource_kind == ResourceKind::ENERGY
                && chk_item.structure_type() == StructureType::Terminal)
        {
            //前回Terminalからresourceを調達している場合はもどさないようにする.

            continue;
        }

        if *is_except_link == true && (chk_item.structure_type() == StructureType::Link) {
            //前回Linkからresourceを調達している場合はもどさないようにする.

            continue;
        }

        let mut dist = 1;
        if chk_item.structure_type() == StructureType::Container {
            dist = 0;
        }

        for resource_type in resource_type_list.iter() {
            if creep.store_of(*resource_type) > 0 as u32 {
                if check_transferable(chk_item, resource_type, None) {
                    find_item_list.push((chk_item.clone(), dist));
                    break;
                }
            }
        }
    }

    let option = SearchOptions::new()
        .room_callback(calc_room_cost)
        .plain_cost(2)
        .swamp_cost(10);

    return search_many(creep, find_item_list, option);
}

pub fn find_nearest_transfarable_terminal(
    creep: &screeps::objects::Creep,
    resource_kind: &ResourceKind,
) -> screeps::pathfinder::SearchResults {
    let item_list = &mut creep
        .room()
        .expect("room is not visible to you")
        .find(find::STRUCTURES);

    {
        let room_list = game::rooms::values();

        for room_item in room_list.iter() {
            if room_item.name() != *(&creep.room().expect("room is not visible to you").name()) {
                let local_list = room_item.find(find::STRUCTURES);
                item_list.extend(local_list);
            }
        }
    }

    let mut find_item_list = Vec::<(Structure, u32)>::new();
    let resource_type_list = make_resoucetype_list(resource_kind);

    for chk_item in item_list.iter() {
        if chk_item.structure_type() != StructureType::Terminal {
            //Terminal以外は除外.
            continue;
        }

        let mut dist = 1;
        if chk_item.structure_type() == StructureType::Container {
            dist = 0;
        }

        for resource_type in resource_type_list.iter() {
            if creep.store_of(*resource_type) > 0 as u32 {
                if check_transferable(chk_item, resource_type, None) {
                    find_item_list.push((chk_item.clone(), dist));
                    break;
                }
            }
        }
    }

    let option = SearchOptions::new()
        .room_callback(calc_room_cost)
        .plain_cost(2)
        .swamp_cost(10);

    return search_many(creep, find_item_list, option);
}

pub fn find_nearest_repairable_item_hp(
    creep: &screeps::objects::Creep,
    threshold: u32,
) -> screeps::pathfinder::SearchResults {
    let item_list = &mut creep
        .room()
        .expect("room is not visible to you")
        .find(find::STRUCTURES);

    {
        let room_list = game::rooms::values();

        for room_item in room_list.iter() {
            if room_item.name() != *(&creep.room().expect("room is not visible to you").name()) {
                let local_list = room_item.find(find::STRUCTURES);
                item_list.extend(local_list);
            }
        }
    }

    let mut find_item_list = Vec::<(Structure, u32)>::new();

    for chk_item in item_list {
        if check_repairable(chk_item) {
            if get_hp(chk_item).unwrap_or(0) <= threshold {
                find_item_list.push((chk_item.clone(), 3));
            }
        }
    }

    let option = SearchOptions::new()
        .room_callback(calc_room_cost)
        .plain_cost(2)
        .swamp_cost(10);

    return search_many(creep, find_item_list, option);
}

pub fn find_nearest_repairable_item_except_wall_dying(
    creep: &screeps::objects::Creep,
    threshold: u128,
) -> screeps::pathfinder::SearchResults {
    let item_list = &mut creep
        .room()
        .expect("room is not visible to you")
        .find(find::STRUCTURES);

    {
        let room_list = game::rooms::values();

        for room_item in room_list.iter() {
            if room_item.name() != *(&creep.room().expect("room is not visible to you").name()) {
                let local_list = room_item.find(find::STRUCTURES);
                item_list.extend(local_list);
            }
        }
    }

    let mut find_item_list = Vec::<(Structure, u32)>::new();

    for chk_item in item_list {
        if chk_item.structure_type() != StructureType::Wall {
            if check_repairable(chk_item) {
                if get_live_tickcount(chk_item).unwrap_or(10000) as u128 <= threshold {
                    find_item_list.push((chk_item.clone(), 3));
                }
            }
        }
    }

    let option = SearchOptions::new()
        .room_callback(calc_room_cost)
        .plain_cost(2)
        .swamp_cost(10);

    return search_many(creep, find_item_list, option);
}

pub fn find_nearest_transferable_structure(
    creep: &screeps::objects::Creep,
    structure_type: &StructureType,
    resource_type: &ResourceType,
    max_cost: Option<f64>,
    capacity_rate: Option<f64>,
) -> screeps::pathfinder::SearchResults {
    let item_list = &mut creep
        .room()
        .expect("room is not visible to you")
        .find(find::STRUCTURES);

    {
        let room_list = game::rooms::values();

        for room_item in room_list.iter() {
            if room_item.name() != *(&creep.room().expect("room is not visible to you").name()) {
                let local_list = room_item.find(find::STRUCTURES);
                item_list.extend(local_list);
            }
        }
    }

    let mut find_item_list = Vec::<(Structure, u32)>::new();

    for chk_item in item_list {
        if chk_item.structure_type() == *structure_type {
            if check_transferable(chk_item, resource_type, capacity_rate) {
                find_item_list.push((chk_item.clone(), 1));
            }
        }
    }

    let option = SearchOptions::new()
        .room_callback(calc_room_cost)
        .plain_cost(2)
        .swamp_cost(10)
        .max_cost(max_cost.unwrap_or(f64::INFINITY));

    return search_many(creep, find_item_list, option);
}

pub fn find_nearest_construction_site(
    creep: &screeps::objects::Creep,
    threshold: u32,
) -> screeps::pathfinder::SearchResults {
    let item_list = &mut creep
        .room()
        .expect("room is not visible to you")
        .find(MY_CONSTRUCTION_SITES);

    {
        let room_list = game::rooms::values();

        for room_item in room_list.iter() {
            if room_item.name() != *(&creep.room().expect("room is not visible to you").name()) {
                let local_list = room_item.find(find::MY_CONSTRUCTION_SITES);
                item_list.extend(local_list);
            }
        }
    }

    let mut find_item_list = Vec::<(ConstructionSite, u32)>::new();

    for chk_item in item_list.iter() {
        if (chk_item.progress_total() - chk_item.progress()) <= threshold {
            find_item_list.push((chk_item.clone(), 3));
        }
    }

    let option = SearchOptions::new()
        .room_callback(calc_room_cost)
        .plain_cost(2)
        .swamp_cost(10);

    return search_many(creep, find_item_list, option);
}

pub fn find_nearest_active_source(
    creep: &screeps::objects::Creep,
    resource_kind: &ResourceKind,
    is_2nd_check: bool,
) -> screeps::pathfinder::SearchResults {
    let mut find_item_list = Vec::<(Position, u32)>::new();
    let resource_type_list = make_resoucetype_list(&resource_kind);

    if is_2nd_check == false {
        // dropped resource.
        let item_list = &mut creep
            .room()
            .expect("room is not visible to you")
            .find(DROPPED_RESOURCES);

        {
            let room_list = game::rooms::values();

            for room_item in room_list.iter() {
                if room_item.name() != *(&creep.room().expect("room is not visible to you").name())
                {
                    let local_list = room_item.find(find::DROPPED_RESOURCES);
                    item_list.extend(local_list);
                }
            }
        }

        for chk_item in item_list.iter() {
            for resource in resource_type_list.iter() {
                if chk_item.resource_type() == *resource {
                    let mut object: Position = creep.pos();
                    object.set_x(chk_item.pos().x());
                    object.set_y(chk_item.pos().y());
                    object.set_room_name(chk_item.room().unwrap().name());

                    find_item_list.push((object.clone(), 1));
                    break;
                }
            }
        }

        // TOMBSTONES.
        let item_list = &mut creep
            .room()
            .expect("room is not visible to you")
            .find(find::TOMBSTONES);

        {
            let room_list = game::rooms::values();

            for room_item in room_list.iter() {
                if room_item.name() != *(&creep.room().expect("room is not visible to you").name())
                {
                    let local_list = room_item.find(find::TOMBSTONES);
                    item_list.extend(local_list);
                }
            }
        }

        for chk_item in item_list.iter() {
            for resource in resource_type_list.iter() {
                if chk_item.store_of(*resource) > 0 {
                    let mut object: Position = creep.pos();
                    object.set_x(chk_item.pos().x());
                    object.set_y(chk_item.pos().y());
                    object.set_room_name(chk_item.room().unwrap().name());

                    find_item_list.push((object.clone(), 1));
                    break;
                }
            }
        }

        // RUINs.
        let item_list = &mut creep
            .room()
            .expect("room is not visible to you")
            .find(find::RUINS);

        {
            let room_list = game::rooms::values();

            for room_item in room_list.iter() {
                if room_item.name() != *(&creep.room().expect("room is not visible to you").name())
                {
                    let local_list = room_item.find(find::RUINS);
                    item_list.extend(local_list);
                }
            }
        }

        for chk_item in item_list.iter() {
            for resource in resource_type_list.iter() {
                if chk_item.store_of(*resource) > 0 {
                    let mut object: Position = creep.pos();
                    object.set_x(chk_item.pos().x());
                    object.set_y(chk_item.pos().y());
                    object.set_room_name(chk_item.room().unwrap().name());

                    find_item_list.push((object.clone(), 1));
                    break;
                }
            }
        }
    }

    if find_item_list.len() <= 0 {
        if *resource_kind == ResourceKind::ENERGY {
            // active source.
            let item_list = &mut creep
                .room()
                .expect("room is not visible to you")
                .find(SOURCES_ACTIVE);

            {
                let room_list = game::rooms::values();

                for room_item in room_list.iter() {
                    if room_item.name()
                        != *(&creep.room().expect("room is not visible to you").name())
                    {
                        let local_list = room_item.find(find::SOURCES_ACTIVE);
                        item_list.extend(local_list);
                    }
                }
            }

            for chk_item in item_list.iter() {
                let mut object: Position = creep.pos();
                object.set_x(chk_item.pos().x());
                object.set_y(chk_item.pos().y());
                object.set_room_name(chk_item.room().unwrap().name());

                find_item_list.push((object.clone(), 1));
            }
        } else if *resource_kind == ResourceKind::MINELALS {
            // minerals.
            let item_list = &mut creep
                .room()
                .expect("room is not visible to you")
                .find(find::MINERALS);
            {
                let room_list = game::rooms::values();

                for room_item in room_list.iter() {
                    if room_item.name()
                        != *(&creep.room().expect("room is not visible to you").name())
                    {
                        let local_list = room_item.find(find::MINERALS);
                        item_list.extend(local_list);
                    }
                }
            }

            for chk_item in item_list.iter() {
                let look_result = creep.room().expect("I can't see").look_for_at_xy(
                    look::STRUCTURES,
                    chk_item.pos().x(),
                    chk_item.pos().y(),
                );

                let mut is_extractor_equited = false;

                for one_result in look_result {
                    if one_result.structure_type() == StructureType::Extractor
                        && check_my_structure(&one_result)
                    {
                        is_extractor_equited = true;
                        break;
                    }
                }

                if is_extractor_equited {
                    let mut object: Position = creep.pos();

                    object.set_x(chk_item.pos().x());
                    object.set_y(chk_item.pos().y());
                    object.set_room_name(chk_item.room().unwrap().name());

                    find_item_list.push((object.clone(), 1));
                }
            }
        } else if *resource_kind == ResourceKind::COMMODITIES {
            // comodities.
            let item_list = &mut creep
                .room()
                .expect("room is not visible to you")
                .find(find::DEPOSITS);

            {
                let room_list = game::rooms::values();

                for room_item in room_list.iter() {
                    if room_item.name()
                        != *(&creep.room().expect("room is not visible to you").name())
                    {
                        let local_list = room_item.find(find::DEPOSITS);
                        item_list.extend(local_list);
                    }
                }
            }

            for chk_item in item_list.iter() {
                let mut object: Position = creep.pos();
                object.set_x(chk_item.pos().x());
                object.set_y(chk_item.pos().y());
                object.set_room_name(chk_item.room().unwrap().name());

                find_item_list.push((object.clone(), 1));
            }
        } else {
            // power.
            let item_list = &mut creep
                .room()
                .expect("room is not visible to you")
                .find(find::STRUCTURES);
            {
                let room_list = game::rooms::values();

                for room_item in room_list.iter() {
                    if room_item.name()
                        != *(&creep.room().expect("room is not visible to you").name())
                    {
                        let local_list = room_item.find(find::STRUCTURES);
                        item_list.extend(local_list);
                    }
                }
            }

            for chk_item in item_list.iter() {
                if chk_item.structure_type() == StructureType::PowerBank {
                    let mut object: Position = creep.pos();
                    object.set_x(chk_item.pos().x());
                    object.set_y(chk_item.pos().y());
                    object.set_room_name(chk_item.room().unwrap().name());

                    find_item_list.push((object.clone(), 1));
                }
            }
        }
    }

    let option = SearchOptions::new()
        .room_callback(calc_room_cost)
        .plain_cost(2)
        .swamp_cost(10);

    return search_many(creep, find_item_list, option);
}

pub fn find_nearest_stored_source(
    creep: &screeps::objects::Creep,
    resource_kind: &ResourceKind,
    is_2nd_check: bool,
) -> screeps::pathfinder::SearchResults {
    let mut find_item_list = Vec::<(Position, u32)>::new();
    let resource_type_list = make_resoucetype_list(&resource_kind);

    if is_2nd_check == false {
        // dropped resource.
        let item_list = &mut creep
            .room()
            .expect("room is not visible to you")
            .find(DROPPED_RESOURCES);
        {
            let room_list = game::rooms::values();

            for room_item in room_list.iter() {
                if room_item.name() != *(&creep.room().expect("room is not visible to you").name())
                {
                    let local_list = room_item.find(find::DROPPED_RESOURCES);
                    item_list.extend(local_list);
                }
            }
        }

        for chk_item in item_list.iter() {
            for resource in resource_type_list.iter() {
                if chk_item.resource_type() == *resource {
                    let mut object: Position = creep.pos();
                    object.set_x(chk_item.pos().x());
                    object.set_y(chk_item.pos().y());
                    object.set_room_name(chk_item.room().unwrap().name());

                    find_item_list.push((object.clone(), 1));
                    break;
                }
            }
        }

        // TOMBSTONES.
        let item_list = &mut creep
            .room()
            .expect("room is not visible to you")
            .find(find::TOMBSTONES);
        {
            let room_list = game::rooms::values();

            for room_item in room_list.iter() {
                if room_item.name() != *(&creep.room().expect("room is not visible to you").name())
                {
                    let local_list = room_item.find(find::TOMBSTONES);
                    item_list.extend(local_list);
                }
            }
        }

        for chk_item in item_list.iter() {
            for resource in resource_type_list.iter() {
                if chk_item.store_of(*resource) > 0 {
                    let mut object: Position = creep.pos();
                    object.set_x(chk_item.pos().x());
                    object.set_y(chk_item.pos().y());
                    object.set_room_name(chk_item.room().unwrap().name());

                    find_item_list.push((object.clone(), 1));
                    break;
                }
            }
        }

        // RUINs.
        let item_list = &mut creep
            .room()
            .expect("room is not visible to you")
            .find(find::RUINS);
        {
            let room_list = game::rooms::values();

            for room_item in room_list.iter() {
                if room_item.name() != *(&creep.room().expect("room is not visible to you").name())
                {
                    let local_list = room_item.find(find::RUINS);
                    item_list.extend(local_list);
                }
            }
        }

        for chk_item in item_list.iter() {
            for resource in resource_type_list.iter() {
                if chk_item.store_of(*resource) > 0 {
                    let mut object: Position = creep.pos();
                    object.set_x(chk_item.pos().x());
                    object.set_y(chk_item.pos().y());
                    object.set_room_name(chk_item.room().unwrap().name());

                    find_item_list.push((object.clone(), 1));
                    break;
                }
            }
        }
    }

    if find_item_list.len() <= 0 {
        let item_list = &mut creep
            .room()
            .expect("room is not visible to you")
            .find(find::STRUCTURES);
        {
            let room_list = game::rooms::values();

            for room_item in room_list.iter() {
                if room_item.name() != *(&creep.room().expect("room is not visible to you").name())
                {
                    let local_list = room_item.find(find::STRUCTURES);
                    item_list.extend(local_list);
                }
            }
        }

        for chk_item in item_list.iter() {
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
                            let mut object: Position = creep.pos();
                            object.set_x(chk_item.pos().x());
                            object.set_y(chk_item.pos().y());
                            object.set_room_name(chk_item.room().unwrap().name());

                            let mut dist = 1;
                            if chk_item.structure_type() == StructureType::Container {
                                dist = 0;
                            }

                            find_item_list.push((object.clone(), dist));
                            break;
                        }
                    }
                }
            }
        }
    }

    let option = SearchOptions::new()
        .room_callback(calc_room_cost)
        .plain_cost(2)
        .swamp_cost(10);

    return search_many(creep, find_item_list, option);
}

pub fn find_nearest_exhausted_source(
    creep: &screeps::objects::Creep,
    harvest_kind: &ResourceKind,
) -> screeps::pathfinder::SearchResults {
    let mut find_item_list = Vec::<(Position, u32)>::new();

    match harvest_kind {
        ResourceKind::ENERGY => {
            let item_list = &mut creep
                .room()
                .expect("room is not visible to you")
                .find(find::SOURCES);
            {
                let room_list = game::rooms::values();

                for room_item in room_list.iter() {
                    if room_item.name()
                        != *(&creep.room().expect("room is not visible to you").name())
                    {
                        let local_list = room_item.find(find::SOURCES);
                        item_list.extend(local_list);
                    }
                }
            }

            for chk_item in item_list.iter() {
                if (chk_item.energy() <= 0) && (chk_item.ticks_to_regeneration() < 50) {
                    let mut object: Position = creep.pos();
                    object.set_x(chk_item.pos().x());
                    object.set_y(chk_item.pos().y());
                    object.set_room_name(chk_item.room().unwrap().name());

                    find_item_list.push((object.clone(), 1));
                }
            }
        }

        ResourceKind::MINELALS => {
            let item_list = &mut creep
                .room()
                .expect("room is not visible to you")
                .find(find::MINERALS);
            {
                let room_list = game::rooms::values();

                for room_item in room_list.iter() {
                    if room_item.name()
                        != *(&creep.room().expect("room is not visible to you").name())
                    {
                        let local_list = room_item.find(find::MINERALS);
                        item_list.extend(local_list);
                    }
                }
            }

            for chk_item in item_list.iter() {
                let look_result = creep.room().expect("I can't see").look_for_at_xy(
                    look::STRUCTURES,
                    chk_item.pos().x(),
                    chk_item.pos().y(),
                );

                let mut is_extractor_equited = false;

                for one_result in look_result {
                    if one_result.structure_type() == StructureType::Extractor
                        && check_my_structure(&one_result)
                    {
                        is_extractor_equited = true;
                        break;
                    }
                }

                if is_extractor_equited {
                    let mut object: Position = creep.pos();

                    object.set_x(chk_item.pos().x());
                    object.set_y(chk_item.pos().y());
                    object.set_room_name(chk_item.room().unwrap().name());

                    find_item_list.push((object.clone(), 1));
                }
            }
        }

        _ => {
            let item_list = &mut creep
                .room()
                .expect("room is not visible to you")
                .find(find::SOURCES);
            {
                let room_list = game::rooms::values();

                for room_item in room_list.iter() {
                    if room_item.name()
                        != *(&creep.room().expect("room is not visible to you").name())
                    {
                        let local_list = room_item.find(find::SOURCES);
                        item_list.extend(local_list);
                    }
                }
            }

            for chk_item in item_list.iter() {
                let mut object: Position = creep.pos();
                object.set_x(chk_item.pos().x());
                object.set_y(chk_item.pos().y());
                object.set_room_name(chk_item.room().unwrap().name());

                find_item_list.push((object.clone(), 1));
            }
        }
    }

    let option = SearchOptions::new()
        .room_callback(calc_room_cost)
        .plain_cost(2)
        .swamp_cost(10);

    return search_many(creep, find_item_list, option);
}

pub fn find_nearest_dropped_resource(
    creep: &screeps::objects::Creep,
    resource_kind: ResourceKind,
) -> screeps::pathfinder::SearchResults {
    let item_list = &mut creep
        .room()
        .expect("room is not visible to you")
        .find(DROPPED_RESOURCES);
    {
        let room_list = game::rooms::values();

        for room_item in room_list.iter() {
            if room_item.name() != *(&creep.room().expect("room is not visible to you").name()) {
                let local_list = room_item.find(find::DROPPED_RESOURCES);
                item_list.extend(local_list);
            }
        }
    }

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

    let option = SearchOptions::new()
        .room_callback(calc_room_cost)
        .plain_cost(2)
        .swamp_cost(10);

    return search_many(creep, find_item_list, option);
}

pub fn find_flee_path_from_active_source(
    creep: &screeps::objects::Creep,
) -> screeps::pathfinder::SearchResults {
    let item_list = &mut creep
        .room()
        .expect("room is not visible to you")
        .find(SOURCES_ACTIVE);
    {
        let room_list = game::rooms::values();

        for room_item in room_list.iter() {
            if room_item.name() != *(&creep.room().expect("room is not visible to you").name()) {
                let local_list = room_item.find(find::SOURCES_ACTIVE);
                item_list.extend(local_list);
            }
        }
    }

    let mut find_item_list = Vec::<(Source, u32)>::new();

    for chk_item in item_list.iter() {
        find_item_list.push((chk_item.clone(), 3));
    }

    let option = SearchOptions::new()
        .room_callback(calc_room_cost)
        .plain_cost(2)
        .swamp_cost(10)
        .flee(true);

    return search_many(creep, find_item_list, option);
}

pub fn find_nearest_enemy(
    creep: &screeps::objects::Creep,
    range: u32,
) -> screeps::pathfinder::SearchResults {
    let item_list = &creep
        .room()
        .expect("room is not visible to you")
        .find(HOSTILE_CREEPS);

    // not nessesary to find another room hostile_creeps.

    let mut find_item_list = Vec::<(screeps::objects::Creep, u32)>::new();

    for chk_item in item_list.iter() {
        find_item_list.push((chk_item.clone(), range));
    }

    let option = SearchOptions::new()
        .room_callback(calc_room_cost)
        .plain_cost(2)
        .swamp_cost(10);

    return search_many(creep, find_item_list, option);
}

pub fn find_nearest_room_controler(
    creep: &screeps::objects::Creep,
) -> screeps::pathfinder::SearchResults {
    let item_list = &mut creep
        .room()
        .expect("room is not visible to you")
        .find(STRUCTURES);
    {
        let room_list = game::rooms::values();

        for room_item in room_list.iter() {
            if room_item.name() != *(&creep.room().expect("room is not visible to you").name()) {
                let local_list = room_item.find(find::STRUCTURES);
                item_list.extend(local_list);
            }
        }
    }

    let mut find_item_list = Vec::<(Structure, u32)>::new();

    for chk_item in item_list.iter() {
        if chk_item.structure_type() == StructureType::Controller {
            if check_my_structure(chk_item) == true {
                find_item_list.push((chk_item.clone(), 3));
            }
        }
    }

    let option = SearchOptions::new()
        .room_callback(calc_room_cost)
        .plain_cost(2)
        .swamp_cost(10);

    return search_many(creep, find_item_list, option);
}

pub fn find_path(
    creep: &screeps::objects::Creep,
    target_pos: &RoomPosition,
    range: u32,
) -> screeps::pathfinder::SearchResults {
    let option = SearchOptions::new()
        .room_callback(calc_room_cost)
        .plain_cost(2)
        .swamp_cost(10);

    return search(creep, target_pos, range, option);
}
