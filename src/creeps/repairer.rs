use crate::constants::*;
use std::u128;

use crate::mem::{keys, MemoryExt};
use crate::util::*;
use log::*;
use screeps::action_error_codes::CreepRepairErrorCode;
use screeps::enums::StructureObject;
use screeps::prelude::*;
use screeps::{find, Creep};

/// 一度選んだ修理対象に粘着する余裕幅 (選定閾値への上乗せ)。
///
/// 選定閾値は「その時点の最小HP + ごく僅か」なので、閾値ぎりぎりで手を
/// 離すと数 tick の修理 (4 WORK = 400 hits/tick) で最小HPが別の建物に移り、
/// 「西の rampart 群へ行進 → 中央へ戻る」の往復行軍になる (実測: worker
/// 5体が約1分周期で地図を往復し、大半の時間を移動に費やしていた)。
/// 4 WORK worker の約75 tick 分を持たせ、一度の訪問で意味のある量を直す。
const REPAIR_HYSTERESIS: u128 = 30_000;

/// 修理する。仕事が無いときの委譲は呼び出し側 (worker) が持つ。
pub fn run_repairer_task(creep: &Creep) {
    let name = creep.name();
    debug!("repairing {}", creep.name());

    debug!("check spawns {}", name);
    let room = creep.room().expect("room is not visible to you");
    let my_spawns = &room.find(find::MY_SPAWNS, None);

    for my_spawn in my_spawns.iter() {
        if my_spawn.hits() < my_spawn.hits_max() {
            debug!("try repair spawns {}", name);
            let r = creep.repair(my_spawn);

            if r.is_ok() {
                info!("repair spawn!!");
                return;
            }
        }
    }

    let structures = room_structures(&room);
    let room_name = &room.name();

    // 粘着中の対象があれば続ける。回復し切る (選定閾値 + 余裕幅) か
    // 消滅するまで同じ対象を直し、標的の乗り換えによる往復を防ぐ。
    if let Some(target) = sticky_target(creep, &structures, room_name) {
        repair_or_approach(creep, &target);
        return;
    }

    let mut is_skip_repair = false;

    // 残り時間が短いものを優先.
    for structure in structures.iter() {
        if check_repairable(structure) {
            if get_live_tickcount(structure).unwrap_or(10000) as u128 <= REPAIRER_DYING_THRESHOLD {
                if let Some(repairable) = structure.as_repairable() {
                    let r = creep.repair(repairable);

                    if r.is_ok() {
                        debug!(
                            "repair my_structure!!:{:?},{:?},{:?}",
                            structure.structure_type(),
                            structure.pos().x(),
                            structure.pos().y()
                        );
                        remember_target(creep, structure);
                        return;
                    }

                    if r == Err(CreepRepairErrorCode::NotInRange) {
                        is_skip_repair = true;
                    }
                }
            }
        }
    }

    // 残りhpが少ない物を優先.
    if is_skip_repair == false {
        let threshold = selection_threshold(room_name);

        for structure in structures.iter() {
            if check_repairable(structure) {
                if get_hp(structure).unwrap_or(0) as u128 <= (threshold + 1) {
                    if let Some(repairable) = structure.as_repairable() {
                        let r = creep.repair(repairable);

                        if r.is_ok() {
                            debug!(
                                "repair my_structure!!:{:?},{:?},{:?}",
                                structure.structure_type(),
                                structure.pos().x(),
                                structure.pos().y()
                            );
                            remember_target(creep, structure);
                            return;
                        }
                    }
                }
            }
        }
    }

    //----------------------------------------
    // 残り時間が少ない物を優先.
    let res = find_nearest_repairable_item_except_wall_dying(&creep, REPAIRER_DYING_THRESHOLD);

    if res.path().len() > 0 {
        remember_target_near_goal(creep, &structures, &res, &|s| {
            get_live_tickcount(s).unwrap_or(10000) as u128 <= REPAIRER_DYING_THRESHOLD
        });
        let res = move_by_search_result(&creep, &res);
        if let Err(e) = res {
            debug!("couldn't move to repair: {:?}", e);
        }
        return;
    }

    // 残りhpが少ない物を優先.
    let threshold = selection_threshold(room_name);

    let res = find_nearest_repairable_item_hp(&creep, (threshold + 1) as u32);

    if res.path().len() > 0 {
        remember_target_near_goal(creep, &structures, &res, &|s| {
            get_hp(s).unwrap_or(0) as u128 <= threshold + 1
        });
        let res = move_by_search_result(&creep, &res);
        if let Err(e) = res {
            debug!("couldn't move to repair: {:?}", e);
        }
        return;
    }
}

/// 修理対象の選定閾値 (≒その時点の最小HP)。
fn selection_threshold(room_name: &screeps::local::RoomName) -> u128 {
    let stats = get_hp_average(room_name);
    stats.1 + (stats.0 - stats.1) / 1000
}

/// 粘着中の修理対象。回復し切った・消滅した・修理対象外になったら
/// claim を消して None を返す。
fn sticky_target(
    creep: &Creep,
    structures: &[StructureObject],
    room_name: &screeps::local::RoomName,
) -> Option<StructureObject> {
    let cmem = creep.memory();
    let s = cmem.string(keys::REPAIR_AT).ok().flatten()?;

    let target = s.split_once(',').and_then(|(x, y)| {
        let (x, y): (u8, u8) = (x.parse().ok()?, y.parse().ok()?);
        structures
            .iter()
            .find(|st| {
                let p = st.pos();
                p.x().u8() == x && p.y().u8() == y && check_repairable(st)
            })
            .cloned()
    });

    match target {
        Some(t)
            if (get_hp(&t).unwrap_or(0) as u128)
                < selection_threshold(room_name) + REPAIR_HYSTERESIS =>
        {
            Some(t)
        }
        _ => {
            cmem.del(keys::REPAIR_AT);
            None
        }
    }
}

/// 対象を粘着 claim する。
fn remember_target(creep: &Creep, structure: &StructureObject) {
    let pos = structure.pos();
    creep.memory().set(
        keys::REPAIR_AT,
        format!("{},{}", pos.x().u8(), pos.y().u8()),
    );
}

/// 経路の到達点 (range 3 goal) の近くで条件に合う最小HPの建造物を
/// 粘着対象として記録する。finder は経路しか返さないため、到達点から
/// 対象を逆引きする。
fn remember_target_near_goal(
    creep: &Creep,
    structures: &[StructureObject],
    res: &screeps::pathfinder::SearchResults,
    filter: &dyn Fn(&StructureObject) -> bool,
) {
    let path = res.path();
    let Some(goal) = path.last() else {
        return;
    };
    let mut best: Option<(u32, &StructureObject)> = None;
    for st in structures.iter() {
        if !check_repairable(st) || !filter(st) {
            continue;
        }
        if st.pos().get_range_to(*goal) > 3 {
            continue;
        }
        let hp = get_hp(st).unwrap_or(u32::MAX);
        if best.is_none_or(|(b, _)| hp < b) {
            best = Some((hp, st));
        }
    }
    if let Some((_, st)) = best {
        remember_target(creep, st);
    }
}

/// 粘着対象を修理する。射程外なら近づく (移動しながらは直せないが、
/// 対象は固定なので翌 tick 以降に射程内で直し続ける)。
fn repair_or_approach(creep: &Creep, target: &StructureObject) {
    if let Some(repairable) = target.as_repairable() {
        if creep.repair(repairable).is_ok() {
            return;
        }
    }
    let res = find_path(creep, &target.pos(), 3);
    if !res.path().is_empty() && !res.incomplete() {
        let _ = move_by_search_result(creep, &res);
    }
}
