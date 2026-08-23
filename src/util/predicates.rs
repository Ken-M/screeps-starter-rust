//! 建造物の状態判定と資源の分類。

use crate::constants::barrier_target_hp;
use screeps::constants::*;
use screeps::enums::StructureObject;
use screeps::prelude::*;
use screeps::ResourceType;
use screeps::StructureType;
use screeps::Terrain;

use super::cache::room_terrain;

#[derive(PartialEq, Debug)]
#[allow(dead_code)] // POWER / COMMODITIES は将来の採取対象として定義だけ残す
pub enum ResourceKind {
    ENERGY,
    MINELALS,
    POWER,
    COMMODITIES,
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
    // 所有型の建造物は自分の物だけ直す。
    // 無所有 (道路・container・壁) は所有判定なしで対象。
    if let Some(my_structure) = structure.as_owned() {
        if my_structure.my() == false {
            return false;
        }
    }

    let Some(attackable) = structure.as_attackable() else {
        // not attackable (= hits を持たない).
        return false;
    };

    let hits = attackable.hits();
    if hits == 0 {
        return false;
    }

    hits < repair_target_hp(structure, attackable.hits_max())
}

/// どこまで修理するかの目標 HP。
///
/// 通常の建造物は hits_max。Rampart / Wall だけは RCL 連動の目標
/// (`barrier_target_hp`) で頭打ちにする。hits_max (Rampart 1M〜、Wall 300M) を
/// 目標にすると事実上永久に「修理対象あり」になり、worker の委譲チェーンが
/// upgrade まで到達しなくなるため (実測で修理労働の100%が Rampart に吸われた)。
fn repair_target_hp(structure: &StructureObject, hits_max: u32) -> u32 {
    match structure.structure_type() {
        StructureType::Rampart | StructureType::Wall => {
            let rcl = super::cache::room_rcl(structure.pos().room_name());
            hits_max.min(barrier_target_hp(rcl))
        }
        _ => hits_max,
    }
}

/// controller 脇の補給 container か (controller の range 2 以内)。
/// 物流の向きを決める区別: hauler は「ここへ届ける (引き出さない)」、
/// worker は「ここから引き出す」。source 脇 container (miner が注ぎ、
/// hauler が汲む) と役割が逆なので、混同すると配達→回収の空回りになる。
pub fn is_controller_stock(structure: &StructureObject) -> bool {
    if structure.structure_type() != StructureType::Container {
        return false;
    }
    let pos = structure.pos();
    let Some(room) = screeps::game::rooms().get(pos.room_name()) else {
        return false;
    };
    let Some(controller) = room.controller() else {
        return false;
    };
    controller.my() && pos.get_range_to(controller.pos()) <= 2
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

/// 部屋の地形 (wasm 側にコピー済み) を tick 単位でキャッシュする。
///
/// `get_live_tickcount` は構造物1個ごとに `room.get_terrain()` を呼んでいた。
/// 1500 構造物なら 1500 回の JS 呼び出し + オブジェクト生成になる。
/// `LocalRoomTerrain` は 2500 バイトを wasm 線形メモリへ写すので、部屋あたり

pub fn get_live_tickcount(structure: &StructureObject) -> Option<u128> {
    let room_obj = structure
        .as_structure()
        .room()
        .expect("room is not visible to you");
    let terrain = room_terrain(&room_obj);

    match structure.as_owned() {
        Some(my_structure) => {
            if my_structure.my() == false {
                return None;
            }

            match structure.as_attackable() {
                Some(attackable) => {
                    let this_terrain = terrain.get_xy(structure.pos().xy());

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
                    let this_terrain = terrain.get_xy(structure.pos().xy());

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

/// 資源分類ごとの資源型リスト。
///
/// 旧実装は呼び出しのたびに `vec![]` で最大41要素をヒープ確保していた。
/// dropped resource のループ内からも呼ばれるので、creep 数 × 資源数だけ

static ENERGY_TYPES: &[ResourceType] = &[ResourceType::Energy];
static POWER_TYPES: &[ResourceType] = &[ResourceType::Power, ResourceType::Ops];
static MINERAL_TYPES: &[ResourceType] = &[ResourceType::Hydrogen,
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
                ResourceType::CatalyzedGhodiumAlkalide,];
static COMMODITY_TYPES: &[ResourceType] = &[ResourceType::Silicon,
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
                ResourceType::Essence,];

pub fn resource_types(resource_kind: &ResourceKind) -> &'static [ResourceType] {
    match resource_kind {
        ResourceKind::ENERGY => ENERGY_TYPES,
        ResourceKind::MINELALS => MINERAL_TYPES,
        ResourceKind::COMMODITIES => COMMODITY_TYPES,
        ResourceKind::POWER => POWER_TYPES,
    }
}

/// 旧 API 互換。呼び出し側が Vec を期待している箇所のため残す。
pub fn make_resoucetype_list(resource_kind: &ResourceKind) -> Vec<ResourceType> {
    resource_types(resource_kind).to_vec()
}
