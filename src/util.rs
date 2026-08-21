use crate::constants::*;
use log::*;
use screeps::constants::*;
use screeps::enums::StructureObject;
use screeps::local::{Position, RoomName, RoomXY};
use screeps::objects::{ConstructionSite, Resource, RoomPosition, Source};
use screeps::pathfinder::{
    search, search_many, MultiRoomCostResult, SearchGoal, SearchOptions, SearchResults,
};
use screeps::prelude::*;
use screeps::look::LookResult;
use screeps::{game, CostMatrix, LocalCostMatrix};

use std::cmp::*;
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

lazy_static! {
    static ref MAP_CACHE: RwLock<Data> = RwLock::new(HashMap::new());
    static ref CONSTRUCTION_PROGRESS_AVERAGE_CACHE: RwLock<ConstructionProgressAverage> =
        RwLock::new(HashMap::new());
    static ref STRUCTURE_HP_AVERAGE_CACHE: RwLock<StructureHpAverage> = RwLock::new(HashMap::new());
    static ref CONSTRUCTION_PROGRESS_MIN_CACHE: RwLock<ConstructionProgressMin> =
        RwLock::new(HashMap::new());
    static ref STRUCTURE_HP_MIN_CACHE: RwLock<StructureHpMin> = RwLock::new(HashMap::new());
}

/// 0.23 で消えた `creep.move_by_path_search_result()` の代替。
/// path 上の現在位置から次の一歩へ move する。
pub fn move_by_search_result(
    creep: &screeps::objects::Creep,
    res: &SearchResults,
) -> Result<(), screeps::action_error_codes::CreepMoveToErrorCode> {
    let path = res.path();
    let pos = creep.pos();

    let next = match path.iter().position(|p| *p == pos) {
        Some(i) if i + 1 < path.len() => path[i + 1],
        Some(_) => return Ok(()), // 既に終点にいる.
        None => match path.first() {
            Some(p) => *p,
            None => return Err(screeps::action_error_codes::CreepMoveToErrorCode::NoPath),
        },
    };

    creep.move_to(next)
}

fn xy(x: u8, y: u8) -> RoomXY {
    RoomXY::checked_new(x, y).expect("coordinate out of range")
}

/// creep の現在室 + 見えている他室から `find` した結果を連結して返す.
/// find 定数は Copy ではないためクロージャで都度生成する.
fn find_all_rooms<T>(
    creep: &screeps::objects::Creep,
    make_ty: impl Fn() -> T,
) -> Vec<T::Item>
where
    T: find::FindConstant,
{
    let home = creep.room().expect("room is not visible to you");
    let mut item_list = home.find(make_ty(), None);

    for room_item in game::rooms().values() {
        if room_item.name() != home.name() {
            item_list.extend(room_item.find(make_ty(), None));
        }
    }

    item_list
}

fn default_search_options() -> SearchOptions<fn(RoomName) -> MultiRoomCostResult> {
    SearchOptions::new(calc_room_cost as fn(RoomName) -> MultiRoomCostResult)
        .plain_cost(2)
        .swamp_cost(10)
}

fn search_goals<T: HasPosition>(list: &[(T, u32)]) -> Vec<SearchGoal> {
    list.iter()
        .map(|(item, range)| SearchGoal::new(item.pos(), *range))
        .collect()
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

    let room = game::rooms().get(*room_name);

    match room {
        Some(room_obj) => {
            let structures = room_obj.find(find::STRUCTURES, None);
            let construction_sites = room_obj.find(find::MY_CONSTRUCTION_SITES, None);

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

fn calc_room_cost(room_name: RoomName) -> MultiRoomCostResult {
    let room = game::rooms().get(room_name);
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
                let structures = room_obj.find(find::STRUCTURES, None);

                for chk_struct in structures {
                    // Roadのコストをさげる.
                    if chk_struct.structure_type() == StructureType::Road {
                        // Favor roads over plain tiles
                        cost_matrix.set(chk_struct.pos().xy(), 1);

                    // 通行不能なStructureはブロック.
                    } else if chk_struct.structure_type() != StructureType::Container
                        && (chk_struct.structure_type() != StructureType::Rampart
                            || check_my_structure(&chk_struct) == false)
                    {
                        // Can't walk through non-walkable buildings
                        cost_matrix.set(chk_struct.pos().xy(), 0xff);
                    }
                }

                // ConstructionSiteの通行不可なものをマーク.
                let construction_sites = room_obj.find(find::MY_CONSTRUCTION_SITES, None);
                for construction_site in construction_sites {
                    if construction_site.structure_type() != StructureType::Road
                        && construction_site.structure_type() != StructureType::Container
                        && construction_site.structure_type() != StructureType::Rampart
                    {
                        // Can't walk through non-walkable construction sites.
                        cost_matrix.set(construction_site.pos().xy(), 0xff);
                    }
                }

                // active sourceの周辺はコストをあげる.
                let item_list = room_obj.find(find::SOURCES_ACTIVE, None);

                for chk_item in item_list.iter() {
                    for x_pos_offset in 0..=2 {
                        for y_pos_offset in 0..=2 {
                            let new_x_pos: i8 = min(
                                max(chk_item.pos().x().u8() as i8 + x_pos_offset - 1, 0),
                                ROOM_SIZE_X as i8 - 1,
                            );
                            let new_y_pos: i8 = min(
                                max(chk_item.pos().y().u8() as i8 + y_pos_offset - 1, 0),
                                ROOM_SIZE_Y as i8 - 1,
                            );

                            let new_xy = xy(new_x_pos as u8, new_y_pos as u8);
                            let cur_cost = cost_matrix.get(new_xy);
                            // すでに通行不可としてマークされているマスは触らない.
                            if cur_cost < 0xff {
                                if room_obj.get_terrain().get(new_x_pos as u8, new_y_pos as u8)
                                    != Terrain::Wall
                                {
                                    let new_cost = 11;
                                    cost_matrix.set(new_xy, new_cost);
                                } else if room_obj
                                    .look_for_at_xy(look::STRUCTURES, new_x_pos as u8, new_y_pos as u8)
                                    .iter()
                                    .filter(|s| s.structure_type() == StructureType::Road)
                                    .count()
                                    > 0
                                {
                                    //Road かつ Wall.
                                    cost_matrix.set(new_xy, 2);
                                }
                            }
                        }
                    }
                }

                // 自分のものかどうかを問わず、creepのいるマスも通行不可として扱う.
                let creeps = room_obj.find(find::CREEPS, None);
                // Avoid creeps in the room
                for creep in creeps {
                    cost_matrix.set(creep.pos().xy(), 0xff);

                    // enemyの射程圏内は、Rampartが無い限りコストをあげる.
                    if creep.my() == false {
                        let mut enemy_range = 1;

                        for body_part in creep.body() {
                            if body_part.hits() > 0 {
                                match body_part.part() {
                                    Part::Attack => {
                                        enemy_range = 1;
                                    }

                                    Part::RangedAttack => {
                                        enemy_range = 3;
                                    }

                                    _ => {}
                                }
                            }
                        }

                        enemy_range = enemy_range * 2;

                        for x_pos_offset in 0..=enemy_range {
                            for y_pos_offset in 0..=enemy_range {
                                let new_x_pos: i8 = min(
                                    max(
                                        creep.pos().x().u8() as i8 + x_pos_offset - enemy_range,
                                        0,
                                    ),
                                    ROOM_SIZE_X as i8 - 1,
                                );
                                let new_y_pos: i8 = min(
                                    max(
                                        creep.pos().y().u8() as i8 + y_pos_offset - enemy_range,
                                        0,
                                    ),
                                    ROOM_SIZE_Y as i8 - 1,
                                );

                                let new_xy = xy(new_x_pos as u8, new_y_pos as u8);
                                let cur_cost = cost_matrix.get(new_xy);
                                // すでに通行不可としてマークされているマスは触らない.
                                if cur_cost < 0xff {
                                    let has_my_rampart = room_obj
                                        .look_for_at_xy(
                                            look::STRUCTURES,
                                            new_x_pos as u8,
                                            new_y_pos as u8,
                                        )
                                        .iter()
                                        .filter(|s| {
                                            s.structure_type() == StructureType::Rampart
                                                && s.as_owned()
                                                    .map(|os| os.my())
                                                    .unwrap_or(false)
                                                    == true
                                        })
                                        .count()
                                        > 0;

                                    if room_obj.get_terrain().get(new_x_pos as u8, new_y_pos as u8)
                                        != Terrain::Wall
                                    {
                                        if !has_my_rampart {
                                            cost_matrix.set(new_xy, cur_cost + 10);
                                        }
                                    } else if room_obj
                                        .look_for_at_xy(
                                            look::STRUCTURES,
                                            new_x_pos as u8,
                                            new_y_pos as u8,
                                        )
                                        .iter()
                                        .filter(|s| s.structure_type() == StructureType::Road)
                                        .count()
                                        > 0
                                    {
                                        //Road かつ Wall.
                                        if !has_my_rampart {
                                            cost_matrix.set(new_xy, cur_cost + 10);
                                        }
                                    }
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

    let room_cost_result = MultiRoomCostResult::CostMatrix(CostMatrix::from(cost_matrix));
    return room_cost_result;
}

pub fn check_walkable(position: &RoomPosition) -> bool {
    let chk_room = game::rooms().get(position.room_name());

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
                    let structure: StructureObject = structure.into();
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

pub fn check_my_structure(structure: &StructureObject) -> bool {
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
    structure: &StructureObject,
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
                            if has_store.store().get_free_capacity(Some(*resource_type))
                                > (has_store.store().get_capacity(Some(*resource_type)) as f64
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
                            if has_store.store().get_free_capacity(Some(*resource_type)) > 0 {
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

pub fn check_repairable(structure: &StructureObject) -> bool {
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
                    // my_struct is not attackable.
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
                    // my_struct is not attackable.
                }
            }
        }
    }
    return false;
}

pub fn get_repairable_hp(structure: &StructureObject) -> Option<u32> {
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
                    // my_struct is not attackable.
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
                    // my_struct is not attackable.
                }
            }
        }
    }
    return None;
}

fn live_tickcount_from_kind(
    structure: &StructureObject,
    attackable_hits: u32,
    this_terrain: Terrain,
) -> Option<u128> {
    match structure {
        StructureObject::StructureRoad(_road) => match this_terrain {
            Terrain::Plain => {
                return Some(
                    ROAD_DECAY_TIME as u128 * (attackable_hits as u128 / ROAD_DECAY_AMOUNT as u128),
                );
            }
            Terrain::Swamp => {
                return Some(
                    ROAD_DECAY_TIME as u128
                        * (attackable_hits as u128
                            / (ROAD_DECAY_AMOUNT as u128
                                * CONSTRUCTION_COST_ROAD_SWAMP_RATIO as u128)),
                );
            }
            Terrain::Wall => {
                return Some(
                    ROAD_DECAY_TIME as u128
                        * (attackable_hits as u128
                            / (ROAD_DECAY_AMOUNT as u128
                                * CONSTRUCTION_COST_ROAD_WALL_RATIO as u128)),
                );
            }
        },

        StructureObject::StructureContainer(_container) => {
            return Some(
                CONTAINER_DECAY_TIME_OWNED as u128 * (attackable_hits as u128 / CONTAINER_DECAY as u128),
            );
        }

        StructureObject::StructureRampart(_ramport) => {
            return Some(
                RAMPART_DECAY_TIME as u128 * (attackable_hits as u128 / RAMPART_DECAY_AMOUNT as u128),
            );
        }

        _ => {}
    }

    None
}

pub fn get_live_tickcount(structure: &StructureObject) -> Option<u128> {
    let room_obj = structure
        .as_structure()
        .room()
        .expect("room is not visible to you");

    match structure.as_owned() {
        Some(my_structure) => {
            if my_structure.my() == false {
                return None;
            }

            match structure.as_attackable() {
                Some(attackable) => {
                    let this_terrain = room_obj
                        .get_terrain()
                        .get(structure.pos().x().u8(), structure.pos().y().u8());

                    return live_tickcount_from_kind(structure, attackable.hits(), this_terrain);
                }

                None => {
                    // my_struct is not attackable.
                }
            }
        }

        None => {
            match structure.as_attackable() {
                Some(attackable) => {
                    let this_terrain = room_obj
                        .get_terrain()
                        .get(structure.pos().x().u8(), structure.pos().y().u8());

                    return live_tickcount_from_kind(structure, attackable.hits(), this_terrain);
                }

                None => {
                    // my_struct is not attackable.
                }
            }
        }
    }
    return None;
}

pub fn get_hp(structure: &StructureObject) -> Option<u32> {
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
                    // my_struct is not attackable.
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
                    // my_struct is not attackable.
                }
            }
        }
    }
    return None;
}

pub fn check_repairable_hp(structure: &StructureObject, hp_th: u32) -> bool {
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
                    // my_struct is not attackable.
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
                    // my_struct is not attackable.
                }
            }
        }
    }
    return false;
}

pub fn check_stored(
    structure: &StructureObject,
    resource_type: &ResourceType,
    keep_amount: u32,
) -> bool {
    match structure.as_has_store() {
        Some(storage) => {
            if storage.store().get_used_capacity(Some(*resource_type)) > keep_amount {
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
) -> SearchResults {
    let item_list = find_all_rooms(creep, || find::STRUCTURES);

    let mut find_item_list = Vec::<(StructureObject, u32)>::new();
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
            if creep.store().get_used_capacity(Some(*resource_type)) > 0 as u32 {
                if check_transferable(chk_item, resource_type, None) {
                    find_item_list.push((chk_item.clone(), dist));
                    break;
                }
            }
        }
    }

    let option = default_search_options();

    return search_many(
        creep.pos(),
        search_goals(&find_item_list).into_iter(),
        Some(option),
    );
}

pub fn find_nearest_transfarable_terminal(
    creep: &screeps::objects::Creep,
    resource_kind: &ResourceKind,
) -> SearchResults {
    let item_list = find_all_rooms(creep, || find::STRUCTURES);

    let mut find_item_list = Vec::<(StructureObject, u32)>::new();
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
            if creep.store().get_used_capacity(Some(*resource_type)) > 0 as u32 {
                if check_transferable(chk_item, resource_type, None) {
                    find_item_list.push((chk_item.clone(), dist));
                    break;
                }
            }
        }
    }

    let option = default_search_options();

    return search_many(
        creep.pos(),
        search_goals(&find_item_list).into_iter(),
        Some(option),
    );
}

pub fn find_nearest_repairable_item_hp(
    creep: &screeps::objects::Creep,
    threshold: u32,
) -> SearchResults {
    let item_list = find_all_rooms(creep, || find::STRUCTURES);

    let mut find_item_list = Vec::<(StructureObject, u32)>::new();

    for chk_item in item_list {
        if check_repairable(&chk_item) {
            if get_hp(&chk_item).unwrap_or(0) <= threshold {
                find_item_list.push((chk_item.clone(), 3));
            }
        }
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
    let item_list = find_all_rooms(creep, || find::STRUCTURES);

    let mut find_item_list = Vec::<(StructureObject, u32)>::new();

    for chk_item in item_list {
        if chk_item.structure_type() != StructureType::Wall {
            if check_repairable(&chk_item) {
                if get_live_tickcount(&chk_item).unwrap_or(10000) as u128 <= threshold {
                    find_item_list.push((chk_item.clone(), 3));
                }
            }
        }
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
    let item_list = find_all_rooms(creep, || find::STRUCTURES);

    let mut find_item_list = Vec::<(StructureObject, u32)>::new();

    for chk_item in item_list {
        if chk_item.structure_type() == *structure_type {
            if check_transferable(&chk_item, resource_type, capacity_rate) {
                find_item_list.push((chk_item.clone(), 1));
            }
        }
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

    let option = default_search_options();

    return search_many(
        creep.pos(),
        search_goals(&find_item_list).into_iter(),
        Some(option),
    );
}

pub fn find_nearest_active_source(
    creep: &screeps::objects::Creep,
    resource_kind: &ResourceKind,
    is_2nd_check: bool,
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

    if find_item_list.len() <= 0 {
        if *resource_kind == ResourceKind::ENERGY {
            // active source.
            let item_list = find_all_rooms(creep, || find::SOURCES_ACTIVE);

            for chk_item in item_list.iter() {
                find_item_list.push((chk_item.pos(), 1));
            }
        } else if *resource_kind == ResourceKind::MINELALS {
            // minerals.
            let item_list = find_all_rooms(creep, || find::MINERALS);

            for chk_item in item_list.iter() {
                let look_result = creep.room().expect("I can't see").look_for_at_xy(
                    look::STRUCTURES,
                    chk_item.pos().x().u8(),
                    chk_item.pos().y().u8(),
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
                    find_item_list.push((chk_item.pos(), 1));
                }
            }
        } else if *resource_kind == ResourceKind::COMMODITIES {
            // comodities.
            let item_list = find_all_rooms(creep, || find::DEPOSITS);

            for chk_item in item_list.iter() {
                find_item_list.push((chk_item.pos(), 1));
            }
        } else {
            // power.
            let item_list = find_all_rooms(creep, || find::STRUCTURES);

            for chk_item in item_list.iter() {
                if chk_item.structure_type() == StructureType::PowerBank {
                    find_item_list.push((chk_item.pos(), 1));
                }
            }
        }
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

pub fn find_nearest_stored_source(
    creep: &screeps::objects::Creep,
    resource_kind: &ResourceKind,
    is_2nd_check: bool,
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

    if find_item_list.len() <= 0 {
        let item_list = find_all_rooms(creep, || find::STRUCTURES);

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

    let option = default_search_options();

    return search_many(
        creep.pos(),
        find_item_list
            .into_iter()
            .map(|(pos, range)| SearchGoal::new(pos, range)),
        Some(option),
    );
}

pub fn find_nearest_exhausted_source(
    creep: &screeps::objects::Creep,
    harvest_kind: &ResourceKind,
) -> SearchResults {
    let mut find_item_list = Vec::<(Position, u32)>::new();

    match harvest_kind {
        ResourceKind::ENERGY => {
            let item_list = find_all_rooms(creep, || find::SOURCES);

            for chk_item in item_list.iter() {
                if (chk_item.energy() <= 0) && (chk_item.ticks_to_regeneration().unwrap_or(0) < 50)
                {
                    find_item_list.push((chk_item.pos(), 1));
                }
            }
        }

        ResourceKind::MINELALS => {
            let item_list = find_all_rooms(creep, || find::MINERALS);

            for chk_item in item_list.iter() {
                let look_result = creep.room().expect("I can't see").look_for_at_xy(
                    look::STRUCTURES,
                    chk_item.pos().x().u8(),
                    chk_item.pos().y().u8(),
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
                    find_item_list.push((chk_item.pos(), 1));
                }
            }
        }

        _ => {
            let item_list = find_all_rooms(creep, || find::SOURCES);

            for chk_item in item_list.iter() {
                find_item_list.push((chk_item.pos(), 1));
            }
        }
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

    let option = default_search_options();

    return search_many(
        creep.pos(),
        search_goals(&find_item_list).into_iter(),
        Some(option),
    );
}

pub fn find_flee_path_from_active_source(creep: &screeps::objects::Creep) -> SearchResults {
    let item_list = find_all_rooms(creep, || find::SOURCES_ACTIVE);

    let mut find_item_list = Vec::<(Source, u32)>::new();

    for chk_item in item_list.iter() {
        find_item_list.push((chk_item.clone(), 3));
    }

    let option = default_search_options().flee(true);

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

    let option = default_search_options();

    return search_many(
        creep.pos(),
        search_goals(&find_item_list).into_iter(),
        Some(option),
    );
}

pub fn find_nearest_room_controler(creep: &screeps::objects::Creep) -> SearchResults {
    let item_list = find_all_rooms(creep, || find::STRUCTURES);

    let mut find_item_list = Vec::<(StructureObject, u32)>::new();

    for chk_item in item_list.iter() {
        if chk_item.structure_type() == StructureType::Controller {
            if check_my_structure(chk_item) == true {
                find_item_list.push((chk_item.clone(), 3));
            }
        }
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
