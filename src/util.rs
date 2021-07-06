use log::*;
use screeps::constants::find::*;
use screeps::constants::*;
use screeps::local::Position;
use screeps::local::RoomName;
use screeps::objects::{HasPosition, Resource};
use screeps::{
    pathfinder::*, ConstructionSite, HasStore, LookResult, RoomObjectProperties, RoomPosition,
    Source, Structure, StructureProperties,
};
use std::cmp::*;
use std::{collections::HashMap, u32, u8};

use lazy_static::lazy_static;
use std::sync::RwLock;

const ROOM_SIZE_X: u8 = 50;
const ROOM_SIZE_Y: u8 = 50;

type Data = HashMap<RoomName, LocalCostMatrix>;

type ConstructionProgressAverage = HashMap<RoomName, u128>;
type RepairableHpAverage_Wall = HashMap<RoomName, u128>;
type StructureHpAverage_ExceptWall = HashMap<RoomName, u128>;

type ConstructionProgressMin = HashMap<RoomName, u128>;
type RepairableHpMax_Wall = HashMap<RoomName, u128>;
type StructureHpMin_ExceptWall = HashMap<RoomName, u128>;

struct GlobalInitFlag {
    init_flag: bool,
}

lazy_static! {
    static ref MAP_CACHE: RwLock<Data> = RwLock::new(HashMap::new());
    static ref CONSTRUCTION_PROGRESS_AVERAGE_CACHE: RwLock<ConstructionProgressAverage> =
        RwLock::new(HashMap::new());
    static ref REPAIRABLE_HP_AVERAGE_WALL_CACHE: RwLock<RepairableHpAverage_Wall> =
        RwLock::new(HashMap::new());
    static ref STRUCTURE_HP_AVERAGE_EXCEPTWALL_CACHE: RwLock<StructureHpAverage_ExceptWall> =
        RwLock::new(HashMap::new());
    static ref CONSTRUCTION_PROGRESS_MIN_CACHE: RwLock<ConstructionProgressMin> =
        RwLock::new(HashMap::new());
    static ref REPAIRABLE_HP_MAX_WALL_CACHE: RwLock<RepairableHpMax_Wall> =
        RwLock::new(HashMap::new());
    static ref STRUCTURE_HP_MIN_EXCEPTWALL_CACHE: RwLock<StructureHpMin_ExceptWall> =
        RwLock::new(HashMap::new());
}

pub fn clear_init_flag() {
    let mut cost_matrix_cache = MAP_CACHE.write().unwrap();
    cost_matrix_cache.clear();

    let mut construction_progress_average = CONSTRUCTION_PROGRESS_AVERAGE_CACHE.write().unwrap();
    construction_progress_average.clear();

    let mut repairable_hp_average_wall = REPAIRABLE_HP_AVERAGE_WALL_CACHE.write().unwrap();
    repairable_hp_average_wall.clear();

    let mut structure_hp_average_exceptwall =
        STRUCTURE_HP_AVERAGE_EXCEPTWALL_CACHE.write().unwrap();
    structure_hp_average_exceptwall.clear();

    let mut construction_progress_min = CONSTRUCTION_PROGRESS_MIN_CACHE.write().unwrap();
    construction_progress_min.clear();

    let mut repairable_hp_max_wall = REPAIRABLE_HP_MAX_WALL_CACHE.write().unwrap();
    repairable_hp_max_wall.clear();

    let mut structure_hp_min_exceptwall = STRUCTURE_HP_MIN_EXCEPTWALL_CACHE.write().unwrap();
    structure_hp_min_exceptwall.clear();
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
    let mut repairable_hp_average_wall = REPAIRABLE_HP_AVERAGE_WALL_CACHE.write().unwrap();
    let mut structure_hp_average_exceptwall =
        STRUCTURE_HP_AVERAGE_EXCEPTWALL_CACHE.write().unwrap();

    let mut construction_progress_min = CONSTRUCTION_PROGRESS_MIN_CACHE.write().unwrap();
    let mut repairable_hp_max_wall = REPAIRABLE_HP_MAX_WALL_CACHE.write().unwrap();
    let mut structure_hp_min_exceptwall = STRUCTURE_HP_MIN_EXCEPTWALL_CACHE.write().unwrap();

    let room = screeps::game::rooms::get(*room_name);

    match room {
        Some(room_obj) => {
            let structures = room_obj.find(STRUCTURES);
            let construction_sites = room_obj.find(MY_CONSTRUCTION_SITES);

            let mut total_repair_hp: u128 = 0;
            let mut total_hp: u128 = 0;

            let mut repair_hp_max: u128 = 0;
            let mut hp_min: u128 = 0;

            let mut struct_count_wall: u128 = 0;
            let mut struct_count_except_wall: u128 = 0;

            for chk_struct in structures {
                if chk_struct.structure_type() == StructureType::Wall {
                    let repair_hp = get_repairable_hp(&chk_struct);

                    match repair_hp {
                        Some(hp) => {
                            struct_count_wall += 1 as u128;
                            total_repair_hp += hp as u128;

                            if (repair_hp_max < hp as u128) || (repair_hp_max == 0) {
                                repair_hp_max = hp as u128;
                            }
                        }
                        None => {}
                    }
                } else {
                    let cur_hp = get_hp_rate(&chk_struct);

                    match cur_hp {
                        Some(hp) => {
                            struct_count_except_wall += 1 as u128;
                            total_hp += hp as u128;

                            if (hp_min > hp as u128) || (hp_min == 0) {
                                hp_min = hp as u128;
                            }
                        }
                        None => {}
                    }
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

            if struct_count_wall > 0 {
                repairable_hp_average_wall.insert(*room_name, total_repair_hp / struct_count_wall);
                repairable_hp_max_wall.insert(*room_name, repair_hp_max);
                info!(
                    "{:?}: repairable_hp_average_wall:{:?}/max:{:?}",
                    room_name,
                    total_repair_hp / struct_count_wall,
                    repair_hp_max
                );
            } else {
                repairable_hp_average_wall.insert(*room_name, 0);
                repairable_hp_max_wall.insert(*room_name, 0);
            }

            if struct_count_except_wall > 0 {
                structure_hp_average_exceptwall
                    .insert(*room_name, total_hp / struct_count_except_wall);

                structure_hp_min_exceptwall.insert(*room_name, hp_min);
                info!(
                    "{:?}: structure_hp_average_exceptwall:{:?}/min:{:?}",
                    room_name,
                    total_hp / struct_count_except_wall,
                    hp_min
                );
            } else {
                structure_hp_average_exceptwall.insert(*room_name, 0);
                structure_hp_min_exceptwall.insert(*room_name, 0);
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

pub fn get_repairable_hp_average_wall(room_name: &RoomName) -> (u128, u128) {
    {
        let repairable_hp_average_wall = REPAIRABLE_HP_AVERAGE_WALL_CACHE.read().unwrap();
        let cache_value = repairable_hp_average_wall.get(&room_name);

        let repairable_hp_max_wall = REPAIRABLE_HP_MAX_WALL_CACHE.read().unwrap();
        let cache_value_max = repairable_hp_max_wall.get(&room_name);

        match cache_value {
            Some(value) => {
                // use cached value.

                match cache_value_max {
                    Some(value_max) => {
                        return (*value, *value_max);
                    }

                    None => {}
                }
            }
            None => {}
        }
    }

    calc_average(room_name);

    {
        let repairable_hp_average_wall = REPAIRABLE_HP_AVERAGE_WALL_CACHE.read().unwrap();
        let cache_value = repairable_hp_average_wall.get(&room_name);

        let repairable_hp_max_wall = REPAIRABLE_HP_MAX_WALL_CACHE.read().unwrap();
        let cache_value_max = repairable_hp_max_wall.get(&room_name);

        match cache_value {
            Some(value) => {
                // use cached value.

                match cache_value_max {
                    Some(value_max) => {
                        return (*value, *value_max);
                    }

                    None => {}
                }
            }
            None => {}
        }
    }

    return (0, 0);
}

pub fn get_hp_average_exceptwall(room_name: &RoomName) -> (u128, u128) {
    {
        let structure_hp_average_exceptwall = STRUCTURE_HP_AVERAGE_EXCEPTWALL_CACHE.read().unwrap();
        let cache_value = structure_hp_average_exceptwall.get(&room_name);

        let structure_hp_min_exceptwall = STRUCTURE_HP_MIN_EXCEPTWALL_CACHE.read().unwrap();
        let cache_value_min = structure_hp_min_exceptwall.get(&room_name);

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
        let structure_hp_average_exceptwall = STRUCTURE_HP_AVERAGE_EXCEPTWALL_CACHE.read().unwrap();
        let cache_value = structure_hp_average_exceptwall.get(&room_name);

        let structure_hp_min_exceptwall = STRUCTURE_HP_MIN_EXCEPTWALL_CACHE.read().unwrap();
        let cache_value_min = structure_hp_min_exceptwall.get(&room_name);

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
                let structures = room_obj.find(STRUCTURES);

                // 地形データを反映.
                for x_pos in 0..ROOM_SIZE_X {
                    for y_pos in 0..ROOM_SIZE_Y {
                        let this_terrain = room_obj.get_terrain().get(x_pos as u32, y_pos as u32);

                        match this_terrain {
                            Terrain::Plain => {
                                cost_matrix.set(x_pos, y_pos, 2);
                            }
                            Terrain::Swamp => {
                                cost_matrix.set(x_pos, y_pos, 10);
                            }
                            Terrain::Wall => {
                                cost_matrix.set(x_pos, y_pos, 0xff);
                            }
                        }
                    }
                }

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
                let creeps = room_obj.find(CREEPS);
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
                                let new_cost = cur_cost + 15;
                                cost_matrix.set(new_x_pos as u8, new_y_pos as u8, new_cost);
                            }
                        }
                    }
                }
            }

            None => {
                // 地形データだけを反映.
                info!("Room:{}, blocked.", room_name);
                for x_pos in 0..ROOM_SIZE_X {
                    for y_pos in 0..ROOM_SIZE_Y {
                        cost_matrix.set(x_pos, y_pos, 0xff);
                    }
                }
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
                                    ROAD_DECAY_TIME as u128 * (attackable.hits() as u128 / 100),
                                );
                            }
                            Terrain::Swamp => {
                                return Some(
                                    ROAD_DECAY_TIME as u128 * (attackable.hits() as u128 / 500),
                                );
                            }
                            Terrain::Wall => {
                                return Some(
                                    ROAD_DECAY_TIME as u128 * (attackable.hits() as u128 / 1500),
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
                                    ROAD_DECAY_TIME as u128 * (attackable.hits() as u128 / 100),
                                );
                            }
                            Terrain::Swamp => {
                                return Some(
                                    ROAD_DECAY_TIME as u128 * (attackable.hits() as u128 / 500),
                                );
                            }
                            Terrain::Wall => {
                                return Some(
                                    ROAD_DECAY_TIME as u128 * (attackable.hits() as u128 / 1500),
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

pub fn get_hp_rate(structure: &screeps::objects::Structure) -> Option<u32> {
    match structure.as_owned() {
        Some(my_structure) => {
            if my_structure.my() == false {
                return None;
            }

            match structure.as_attackable() {
                Some(attackable) => {
                    if (attackable.hits() > 0) && (attackable.hits() < attackable.hits_max()) {
                        return Some(
                            ((attackable.hits() as u128 * 10000) / attackable.hits_max() as u128)
                                as u32,
                        );
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
                        return Some(
                            ((attackable.hits() as u128 * 10000) / attackable.hits_max() as u128)
                                as u32,
                        );
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
pub fn check_stored(structure: &screeps::objects::Structure, resource_type: &ResourceType) -> bool {
    match structure.as_has_store() {
        Some(storage) => {
            if storage.store_of(*resource_type) > 0 {
                return true;
            }
        }

        None => {}
    }
    return false;
}

pub fn make_resoucetype_list(resource_kind: &ResourceKind) -> Vec<ResourceType> {
    let mut resource_type_list = Vec::<ResourceType>::new();

    match resource_kind {
        ResourceKind::ENERGY => {
            resource_type_list.push(ResourceType::Energy);
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

            resource_type_list.extend(templist);
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
            resource_type_list.extend(templist);
        }

        ResourceKind::POWER => {
            resource_type_list.push(ResourceType::Power);
            resource_type_list.push(ResourceType::Ops);
        }
    }

    return resource_type_list;
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
) -> screeps::pathfinder::SearchResults {
    let item_list = &creep
        .room()
        .expect("room is not visible to you")
        .find(STRUCTURES);

    let mut find_item_list = Vec::<(Structure, u32)>::new();
    let resource_type_list = make_resoucetype_list(resource_kind);

    for chk_item in item_list {
        if chk_item.structure_type() == StructureType::Lab
            && *resource_kind == ResourceKind::MINELALS
        {
            continue;
        }

        if *is_except_storages == true
            && (chk_item.structure_type() == StructureType::Container
                || chk_item.structure_type() == StructureType::Storage
                || (*resource_kind == ResourceKind::ENERGY
                    && chk_item.structure_type() == StructureType::Terminal))
        {
            //前回storage系からresourceを調達している場合はもどさないようにする.

            continue;
        }

        let mut dist = 1;
        if chk_item.structure_type() == StructureType::Container {
            dist = 0;
        }

        for resource_type in resource_type_list.iter() {
            if check_transferable(chk_item, resource_type) {
                find_item_list.push((chk_item.clone(), dist));
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

pub fn find_nearest_repairable_item_onlywall_repair_hp(
    creep: &screeps::objects::Creep,
    threshold: u32,
) -> screeps::pathfinder::SearchResults {
    let item_list = &creep
        .room()
        .expect("room is not visible to you")
        .find(STRUCTURES);

    let mut find_item_list = Vec::<(Structure, u32)>::new();

    for chk_item in item_list {
        if chk_item.structure_type() == StructureType::Wall {
            let repair_hp = get_repairable_hp(chk_item);
            match repair_hp {
                Some(hp) => {
                    if hp >= threshold {
                        find_item_list.push((chk_item.clone(), 3));
                    }
                }

                None => {}
            }
        }
    }

    let option = SearchOptions::new()
        .room_callback(calc_room_cost)
        .plain_cost(2)
        .swamp_cost(10);

    return search_many(creep, find_item_list, option);
}

pub fn find_nearest_repairable_item_onlywall_hp(
    creep: &screeps::objects::Creep,
    threshold: u32,
) -> screeps::pathfinder::SearchResults {
    let item_list = &creep
        .room()
        .expect("room is not visible to you")
        .find(STRUCTURES);

    let mut find_item_list = Vec::<(Structure, u32)>::new();

    for chk_item in item_list {
        if chk_item.structure_type() == StructureType::Wall {
            if check_repairable_hp(chk_item, threshold) {
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

pub fn find_nearest_repairable_item_except_wall_hp(
    creep: &screeps::objects::Creep,
    threshold: u32,
) -> screeps::pathfinder::SearchResults {
    let item_list = &creep
        .room()
        .expect("room is not visible to you")
        .find(STRUCTURES);

    let mut find_item_list = Vec::<(Structure, u32)>::new();

    for chk_item in item_list {
        if chk_item.structure_type() != StructureType::Wall {
            if check_repairable(chk_item) {
                if get_hp_rate(chk_item).unwrap_or(0) <= threshold {
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

pub fn find_nearest_repairable_item_except_wall_dying(
    creep: &screeps::objects::Creep,
) -> screeps::pathfinder::SearchResults {
    let item_list = &creep
        .room()
        .expect("room is not visible to you")
        .find(STRUCTURES);

    let mut find_item_list = Vec::<(Structure, u32)>::new();

    for chk_item in item_list {
        if chk_item.structure_type() != StructureType::Wall {
            if check_repairable(chk_item) {
                if get_live_tickcount(chk_item).unwrap_or(10000) as u128 <= 1000 {
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
) -> screeps::pathfinder::SearchResults {
    let item_list = &creep
        .room()
        .expect("room is not visible to you")
        .find(STRUCTURES);

    let mut find_item_list = Vec::<(Structure, u32)>::new();

    for chk_item in item_list {
        if chk_item.structure_type() == *structure_type {
            if check_transferable(chk_item, resource_type) {
                find_item_list.push((chk_item.clone(), 1));
            }
        }
    }

    let option = SearchOptions::new()
        .room_callback(calc_room_cost)
        .plain_cost(2)
        .swamp_cost(10);

    return search_many(creep, find_item_list, option);
}

pub fn find_nearest_construction_site(
    creep: &screeps::objects::Creep,
    threshold: u32,
) -> screeps::pathfinder::SearchResults {
    let item_list = &creep
        .room()
        .expect("room is not visible to you")
        .find(MY_CONSTRUCTION_SITES);

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
        let item_list = &creep
            .room()
            .expect("room is not visible to you")
            .find(DROPPED_RESOURCES);

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
        let item_list = &creep
            .room()
            .expect("room is not visible to you")
            .find(TOMBSTONES);

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
        let item_list = &creep
            .room()
            .expect("room is not visible to you")
            .find(RUINS);

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

    if *resource_kind == ResourceKind::ENERGY {
        // active source.
        let item_list = &creep
            .room()
            .expect("room is not visible to you")
            .find(SOURCES_ACTIVE);

        for chk_item in item_list.iter() {
            let mut object: Position = creep.pos();
            object.set_x(chk_item.pos().x());
            object.set_y(chk_item.pos().y());
            object.set_room_name(chk_item.room().unwrap().name());

            find_item_list.push((object.clone(), 1));
        }
    } else if *resource_kind == ResourceKind::MINELALS {
        // minerals.
        let item_list = &creep
            .room()
            .expect("room is not visible to you")
            .find(MINERALS);

        for chk_item in item_list.iter() {
            let mut object: Position = creep.pos();
            object.set_x(chk_item.pos().x());
            object.set_y(chk_item.pos().y());
            object.set_room_name(chk_item.room().unwrap().name());

            find_item_list.push((object.clone(), 1));
        }
    } else if *resource_kind == ResourceKind::COMMODITIES {
        // comodities.
        let item_list = &creep
            .room()
            .expect("room is not visible to you")
            .find(DEPOSITS);

        for chk_item in item_list.iter() {
            let mut object: Position = creep.pos();
            object.set_x(chk_item.pos().x());
            object.set_y(chk_item.pos().y());
            object.set_room_name(chk_item.room().unwrap().name());

            find_item_list.push((object.clone(), 1));
        }
    } else {
        // power.
        let item_list = &creep
            .room()
            .expect("room is not visible to you")
            .find(STRUCTURES);

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
        let item_list = &creep
            .room()
            .expect("room is not visible to you")
            .find(DROPPED_RESOURCES);

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
        let item_list = &creep
            .room()
            .expect("room is not visible to you")
            .find(TOMBSTONES);

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
        let item_list = &creep
            .room()
            .expect("room is not visible to you")
            .find(RUINS);

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

    let item_list = &creep
        .room()
        .expect("room is not visible to you")
        .find(STRUCTURES);

    for chk_item in item_list.iter() {
        if chk_item.structure_type() == StructureType::Container
            || chk_item.structure_type() == StructureType::Storage
            || chk_item.structure_type() == StructureType::Lab
            || (*resource_kind == ResourceKind::ENERGY
                && chk_item.structure_type() == StructureType::Terminal)
        {
            if check_my_structure(chk_item)
                || (chk_item.structure_type() == StructureType::Container)
            {
                for resource_type in resource_type_list.iter() {
                    if check_stored(chk_item, resource_type) {
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

    let option = SearchOptions::new()
        .room_callback(calc_room_cost)
        .plain_cost(2)
        .swamp_cost(10);

    return search_many(creep, find_item_list, option);
}

pub fn find_nearest_source(
    creep: &screeps::objects::Creep,
    harvest_kind: &ResourceKind,
) -> screeps::pathfinder::SearchResults {
    let mut find_item_list = Vec::<(Position, u32)>::new();

    match harvest_kind {
        ResourceKind::ENERGY => {
            let item_list = &creep
                .room()
                .expect("room is not visible to you")
                .find(find::SOURCES);

            for chk_item in item_list.iter() {
                let mut object: Position = creep.pos();
                object.set_x(chk_item.pos().x());
                object.set_y(chk_item.pos().y());
                object.set_room_name(chk_item.room().unwrap().name());

                find_item_list.push((object.clone(), 1));
            }
        }

        ResourceKind::MINELALS => {
            let item_list = &creep
                .room()
                .expect("room is not visible to you")
                .find(find::MINERALS);

            for chk_item in item_list.iter() {
                let mut object: Position = creep.pos();
                object.set_x(chk_item.pos().x());
                object.set_y(chk_item.pos().y());
                object.set_room_name(chk_item.room().unwrap().name());

                find_item_list.push((object.clone(), 1));
            }
        }

        _ => {
            let item_list = &creep
                .room()
                .expect("room is not visible to you")
                .find(find::SOURCES);

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
    let item_list = &creep
        .room()
        .expect("room is not visible to you")
        .find(DROPPED_RESOURCES);

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
    let item_list = &creep
        .room()
        .expect("room is not visible to you")
        .find(SOURCES_ACTIVE);

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
