//! 運搬者。
//!
//! container / storage / 落ちている資源から拾い、拠点へ運ぶ。自分では掘らない。
//!
//! 採掘を静的採掘者に任せた分、運搬者は WORK を持たずに済む。同じエネルギー予算で
//! CARRY と MOVE だけを積めるので、1体あたりの輸送量が増える。
//!
//! 配達先の優先順は補給係と同じ (spawn → tower → extension) だが、
//! 補給係が自分で掘りに行くのに対し、運搬者は必ず貯蔵から引く。


use crate::mem::{keys, MemoryExt};
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

    // 満載まで拾い、空になるまで届けるステートマシン。
    //
    // 旧実装は「積荷が 1 でもあれば届ける」だったので、小さな落下物を
    // 1 回拾っただけで配達に切り替わり、容量 250 に対して 21〜84 のまま
    // source と spawn を往復し続けていた (実測)。この部屋は南北の行き来が
    // 長い迂回路なので、半端な積載の往復は輸送量をそのまま数分の一にする。
    let store = creep.store();
    let used = store.get_used_capacity(Some(ResourceType::Energy));
    let free = store.get_free_capacity(Some(ResourceType::Energy));

    let filling = if used == 0 {
        true
    } else if free == 0 {
        false
    } else {
        creep.memory().bool(keys::FILLING)
    };
    creep.memory().set(keys::FILLING, filling);

    if filling {
        if collect_energy(creep, &room) {
            return;
        }
        // 拾える物が何も見つからない。積荷があるなら先に届けてしまう。
        if used > 0 {
            creep.memory().set(keys::FILLING, false);
            deliver(creep, &room);
        }
        return;
    }

    deliver(creep, &room);
}

/// 拾う。近くに落ちているものを優先し、無ければ container / storage から引く。
/// worker もエネルギー補給に使う (spawn / extension は生産用なので触らない)。
/// 戻り値は「拾えた・引けた・拾いに向かった」か。false なら部屋に何も無い。
pub fn collect_energy(creep: &Creep, room: &screeps::objects::Room) -> bool {
    // 足下や隣に落ちている資源。採掘者が溢れさせた分がここに来る。
    for resource in room.find(find::DROPPED_RESOURCES, None).iter() {
        if resource.resource_type() != ResourceType::Energy {
            continue;
        }
        if creep.pos().is_near_to(resource.pos()) {
            if creep.pickup(resource).is_ok() {
                return true;
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
                    return true;
                }
            }
            screeps::enums::StructureObject::StructureStorage(st) => {
                if creep.withdraw(st, ResourceType::Energy, None).is_ok() {
                    return true;
                }
            }
            _ => {}
        }
    }

    // 近くに無いので探しに行く。貯蔵優先 (自分では掘らない)。
    let res = find_nearest_stored_source(creep, &ResourceKind::ENERGY, false);
    if !res.path().is_empty() {
        let _ = move_by_search_result(creep, &res);
        return true;
    }

    // 貯蔵が空なら、落ちている資源を探す。
    let res = find_nearest_dropped_resource(creep, ResourceKind::ENERGY);
    if !res.path().is_empty() {
        let _ = move_by_search_result(creep, &res);
        return true;
    }

    false
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

    // 届け先がすべて満杯。その場で待つと container の隣接マスを塞いで
    // 採取に来た worker を通せんぼする (実測: container の歩けるマス5個の
    // うち4個を待機 hauler が占有し、worker が外周で滞留)。spawn のそばまで
    // 下がって待機すれば、通路が空く上に extension が空いた瞬間に補給できる。
    debug!("{} has nowhere to deliver; parking near spawn", name);
    for spawn in room.find(find::MY_SPAWNS, None).iter() {
        if creep.pos().get_range_to(spawn.pos()) > 3 {
            let res = find_path(creep, &spawn.pos(), 3);
            if !res.path().is_empty() {
                let _ = move_by_search_result(creep, &res);
            }
        }
        return;
    }
}
