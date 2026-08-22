//! 運搬者。
//!
//! container / storage / 落ちている資源から拾い、拠点へ運ぶ。自分では掘らない。
//!
//! 採掘を静的採掘者に任せた分、運搬者は WORK を持たずに済む。同じエネルギー予算で
//! CARRY と MOVE だけを積めるので、1体あたりの輸送量が増える。
//!
//! 配達先の優先順は補給係と同じ (spawn → tower → extension) だが、
//! 補給係が自分で掘りに行くのに対し、運搬者は必ず貯蔵から引く。


use crate::util::*;
use log::*;
use screeps::prelude::*;
use screeps::{find, Creep, ResourceType, StructureType};

pub fn run_hauler(creep: &Creep) {
    let name = creep.name();
    info!("running hauler {}", name);

    let Some(room) = creep.room() else {
        return;
    };

    // 空なら拾いに行く。
    if creep.store().get_used_capacity(Some(ResourceType::Energy)) == 0 {
        collect(creep, &room);
        return;
    }

    // 積んでいるなら届ける。
    deliver(creep, &room);
}

/// 拾う。近くに落ちているものを優先し、無ければ container / storage から引く。
fn collect(creep: &Creep, room: &screeps::objects::Room) {
    // 足下や隣に落ちている資源。採掘者が溢れさせた分がここに来る。
    for resource in room.find(find::DROPPED_RESOURCES, None).iter() {
        if resource.resource_type() != ResourceType::Energy {
            continue;
        }
        if creep.pos().is_near_to(resource.pos()) {
            if creep.pickup(resource).is_ok() {
                return;
            }
        }
    }

    // 隣接する container / storage から引く。
    for structure in room_structures(room).iter() {
        if !matches!(
            structure.structure_type(),
            StructureType::Container | StructureType::Storage
        ) {
            continue;
        }
        if !creep.pos().is_near_to(structure.pos()) {
            continue;
        }
        if !check_stored(structure, &ResourceType::Energy, 0) {
            continue;
        }
        // withdraw は具象型を要求するので種類ごとに扱う。
        match structure {
            screeps::enums::StructureObject::StructureContainer(c) => {
                if creep.withdraw(c, ResourceType::Energy, None).is_ok() {
                    return;
                }
            }
            screeps::enums::StructureObject::StructureStorage(st) => {
                if creep.withdraw(st, ResourceType::Energy, None).is_ok() {
                    return;
                }
            }
            _ => {}
        }
    }

    // 近くに無いので探しに行く。貯蔵優先 (自分では掘らない)。
    let res = find_nearest_stored_source(creep, &ResourceKind::ENERGY, false);
    if !res.path().is_empty() {
        let _ = move_by_search_result(creep, &res);
        return;
    }

    // 貯蔵が空なら、落ちている資源を探す。
    let res = find_nearest_dropped_resource(creep, ResourceKind::ENERGY);
    if !res.path().is_empty() {
        let _ = move_by_search_result(creep, &res);
    }
}

/// 届ける。spawn → tower → extension の順。
fn deliver(creep: &Creep, room: &screeps::objects::Room) {
    let name = creep.name();

    // 隣接している搬入先があれば即座に入れる。
    for structure in room_structures(room).iter() {
        if !matches!(
            structure.structure_type(),
            StructureType::Spawn | StructureType::Tower | StructureType::Extension
        ) {
            continue;
        }
        if !creep.pos().is_near_to(structure.pos()) {
            continue;
        }
        if !check_transferable(structure, &ResourceType::Energy, None) {
            continue;
        }
        if let Some(transferable) = structure.as_transferable() {
            if creep.transfer(transferable, ResourceType::Energy, None).is_ok() {
                return;
            }
        }
    }

    // 無ければ最寄りの搬入先へ向かう。
    for ty in [
        StructureType::Spawn,
        StructureType::Tower,
        StructureType::Extension,
        StructureType::Storage,
    ] {
        let res = find_nearest_transferable_structure(
            creep,
            &ty,
            &ResourceType::Energy,
            None,
            None,
        );
        if !res.path().is_empty() {
            let _ = move_by_search_result(creep, &res);
            return;
        }
    }

    debug!("{} has nowhere to deliver", name);
}
