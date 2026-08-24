//! 運搬者。
//!
//! container / storage / 落ちている資源から拾い、拠点へ運ぶ。自分では掘らない。
//!
//! 採掘を静的採掘者に任せた分、運搬者は WORK を持たずに済む。同じエネルギー予算で
//! CARRY と MOVE だけを積めるので、1体あたりの輸送量が増える。
//!
//! 配達先の優先順は spawn → tower → extension → controller 脇の補給
//! container → storage。ただし補給 container が枯れかけのときだけ
//! spawn の直後に割り込む (専任 upgrader を止めないため)。


use crate::mem::{keys, MemoryExt};
use crate::util::*;
use log::*;
use screeps::prelude::*;
use screeps::local::RoomXY;
use screeps::{find, look, Creep, Position, ResourceType, StructureType};

pub fn run_hauler(creep: &Creep) {
    let name = creep.name();
    debug!("running hauler {}", name);

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

    // 隣に落ちているエネルギーは、積み込み中かどうかに関わらず拾っておく。
    // pickup は move / transfer / withdraw と同 tick に併用できる別枠の
    // アクションなので、配達の途中でも1 tick も失わずに回収できる。
    // 落下物は放置すると毎 tick 減衰で目減りする。
    if free > 0 {
        for resource in room.find(find::DROPPED_RESOURCES, None).iter() {
            if resource.resource_type() == ResourceType::Energy
                && creep.pos().is_near_to(resource.pos())
            {
                let _ = creep.pickup(resource);
                break;
            }
        }
    }

    let filling = if used == 0 {
        true
    } else if free == 0 {
        false
    } else {
        creep.memory().bool(keys::FILLING)
    };
    creep.memory().set(keys::FILLING, filling);

    if filling {
        // hauler は controller 脇の補給 container から引き出さない
        // (自分の配達先を自分で空にする空回りになる)。
        if collect_energy(creep, &room, false) {
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
/// use_controller_stock: controller 脇の補給 container から引き出してよいか。
/// worker (消費側) は true、hauler (配達側) は false。
/// 戻り値は「拾えた・引けた・拾いに向かった」か。false なら部屋に何も無い。
pub fn collect_energy(creep: &Creep, room: &screeps::objects::Room, use_controller_stock: bool) -> bool {
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

    // 隣接する墓標・廃墟から引き出す。
    //
    // 経路探索 (find_nearest_stored_source) は墓標・廃墟を目標に含めて
    // hauler をそこまで誘導するのに、旧実装は到着後に引き出すコードを
    // 持っていなかった。hauler は墓標の隣で立ち尽くし、中身は減衰で
    // 蒸発していた。落下物と同じ減衰物なので container より先に確認する。
    for tombstone in room.find(find::TOMBSTONES, None).iter() {
        if !creep.pos().is_near_to(tombstone.pos()) {
            continue;
        }
        if tombstone
            .store()
            .get_used_capacity(Some(ResourceType::Energy))
            == 0
        {
            continue;
        }
        if creep.withdraw(tombstone, ResourceType::Energy, None).is_ok() {
            return true;
        }
    }
    for ruin in room.find(find::RUINS, None).iter() {
        if !creep.pos().is_near_to(ruin.pos()) {
            continue;
        }
        if ruin.store().get_used_capacity(Some(ResourceType::Energy)) == 0 {
            continue;
        }
        if creep.withdraw(ruin, ResourceType::Energy, None).is_ok() {
            return true;
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
        if !use_controller_stock && is_controller_stock(structure) {
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
    let res =
        find_nearest_stored_source(creep, &ResourceKind::ENERGY, false, !use_controller_stock);
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

    // 無ければ優先順に搬入先へ向かう。spawn (生産の要) は常に最優先。
    if seek_transferable(creep, StructureType::Spawn) {
        return;
    }

    // controller 脇の補給 container が枯れかけなら tower / extension より
    // 先に届ける。常に後回しだと、生産が回って extension が満ちない間は
    // 一度も届かず、専任 upgrader (MOVE 1 の座り仕事 body) が自力調達に
    // 出て寿命を移動で溶かす (実測: 補給が絶えた時間帯に進捗が 1/3 に低下)。
    if controller_stock_running_low(room) && deliver_controller_stock(creep, room) {
        return;
    }

    if seek_transferable(creep, StructureType::Tower)
        || seek_transferable(creep, StructureType::Extension)
    {
        return;
    }

    // 生産用の備蓄が満ちていたら、controller 脇の補給 container へ。
    // 余剰エネルギーをアップグレード係の手元へ届ける物流の後半分。
    if deliver_controller_stock(creep, room) {
        return;
    }

    // それも満ちていれば storage へ退蔵する。
    {
        let res = find_nearest_transferable_structure(
            creep,
            &StructureType::Storage,
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
            return;
        }
        // 待機位置に着いた。道路の上で止まると幹線の対向を塞ぐので一歩どく。
        step_off_road(creep, room, spawn.pos());
        return;
    }
}

/// 道路上で待機しない。隣の非道路マス (待機圏 range 3 の内側) へ一歩どく。
/// 待機圏の外へ出ると次 tick に戻ってきて発振するので圏内に限る。
fn step_off_road(creep: &Creep, room: &screeps::objects::Room, anchor: Position) {
    let cur = creep.pos();
    if !tile_has_road(room, cur.xy()) {
        return;
    }
    for dx in -1..=1i8 {
        for dy in -1..=1i8 {
            if dx == 0 && dy == 0 {
                continue;
            }
            let x = cur.x().u8() as i8 + dx;
            let y = cur.y().u8() as i8 + dy;
            if !(0..50).contains(&x) || !(0..50).contains(&y) {
                continue;
            }
            let Ok(xy) = RoomXY::checked_new(x as u8, y as u8) else {
                continue;
            };
            if tile_has_road(room, xy) || !is_walkable_tile(room, xy) {
                continue;
            }
            let pos = Position::new(xy.x, xy.y, room.name());
            if anchor.get_range_to(pos) > 3 {
                continue;
            }
            if !room.look_for_at_xy(look::CREEPS, x as u8, y as u8).is_empty() {
                continue;
            }
            let res = find_path(creep, &pos, 0);
            if !res.path().is_empty() {
                let _ = move_by_search_result(creep, &res);
            }
            return;
        }
    }
}

/// controller 脇の補給 container の在庫がこれを下回ったら「枯れかけ」とみなし、
/// tower / extension より先に補給する。専任 upgrader (WORK 全振り) が
/// source 数 + 増員分いると消費は 20〜50/tick 程度で、hauler の一往復の間に
/// 使い切られる量。容量 2000 の半分より手前で先回りする。
const CONTROLLER_STOCK_LOW: u32 = 1000;

/// controller 脇の補給 container のどれかが枯れかけか。
fn controller_stock_running_low(room: &screeps::objects::Room) -> bool {
    room_structures(room)
        .iter()
        .filter(|s| is_controller_stock(s))
        .any(|s| !check_stored(s, &ResourceType::Energy, CONTROLLER_STOCK_LOW))
}

/// 種別 ty の受け入れ可能な最寄りの施設へ移動する。対象が無ければ false。
fn seek_transferable(creep: &Creep, ty: StructureType) -> bool {
    let res =
        find_nearest_transferable_structure(creep, &ty, &ResourceType::Energy, None, None);
    if res.path().is_empty() {
        return false;
    }
    let _ = move_by_search_result(creep, &res);
    true
}

/// controller 脇の補給 container へ届ける。
/// 隣接していれば transfer、遠ければ移動。対象が無い・満杯なら false。
fn deliver_controller_stock(creep: &Creep, room: &screeps::objects::Room) -> bool {
    for structure in room_structures(room).iter() {
        if !is_controller_stock(structure) {
            continue;
        }
        if !check_transferable(structure, &ResourceType::Energy, None) {
            continue;
        }
        if creep.pos().is_near_to(structure.pos()) {
            if let Some(transferable) = structure.as_transferable() {
                if creep
                    .transfer(transferable, ResourceType::Energy, None)
                    .is_ok()
                {
                    return true;
                }
            }
            continue;
        }
        let res = find_path(creep, &structure.pos(), 1);
        if !res.path().is_empty() {
            let _ = move_by_search_result(creep, &res);
            return true;
        }
    }
    false
}
