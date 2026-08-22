use crate::constants::*;
use crate::util::*;
use log::*;
use screeps::enums::StructureObject;
use screeps::prelude::*;
use screeps::{find, game, Part, ResourceType, StructureType};

/// タワーの攻撃威力は距離で減衰する。
/// 射程 5 以内で最大 (600)、20 以上で最小 (150)、その間は線形。
fn tower_damage_at(range: u32) -> u32 {
    const MAX_DAMAGE: u32 = 600;
    const MIN_DAMAGE: u32 = 150;
    const OPTIMAL_RANGE: u32 = 5;
    const FALLOFF_RANGE: u32 = 20;

    if range <= OPTIMAL_RANGE {
        MAX_DAMAGE
    } else if range >= FALLOFF_RANGE {
        MIN_DAMAGE
    } else {
        let span = FALLOFF_RANGE - OPTIMAL_RANGE;
        let progressed = range - OPTIMAL_RANGE;
        MAX_DAMAGE - (MAX_DAMAGE - MIN_DAMAGE) * progressed / span
    }
}

pub fn run_tower() {
    for game_structure in game::structures().values() {
        let mut is_done = false;

        if check_my_structure(&game_structure) == true {
            match game_structure {
                StructureObject::StructureTower(my_tower) => {
                    debug!("check enemies {}", my_tower.id());
                    let Some(room) = my_tower.room() else {
                        continue;
                    };
                    let _room_name = room.name();
                    let enemies = room_hostiles(&room);

                    // 標的の選び方。
                    //
                    // 旧実装は配列の先頭の敵を撃って break していた。`attack()` は
                    // 射程外でも成功を返す (威力が減衰するだけ) ので、距離20以上の敵に
                    // 撃つと威力が 600 から 150 に落ちる。HEAL 持ちの敵ならこの威力は
                    // 回復量に相殺され、削れないまま全タワーのエネルギーを空にする。
                    //
                    // 距離で減衰した実効ダメージを見積もり、それが最も大きい敵を狙う。
                    // 同程度なら HEAL 持ちを優先する (放置すると全体が削れなくなるため)。
                    let target = enemies
                        .iter()
                        .max_by_key(|enemy| {
                            let range = my_tower.pos().get_range_to(enemy.pos()) as u32;
                            let damage = tower_damage_at(range);
                            let heal_parts = enemy
                                .body()
                                .iter()
                                .filter(|bp| bp.hits() > 0 && bp.part() == Part::Heal)
                                .count() as u32;
                            // ヒーラーには重み付けして優先度を上げる。
                            damage + heal_parts * 100
                        });

                    if let Some(enemy) = target {
                        debug!("try attack enemy {}", my_tower.id());
                        if my_tower.attack(enemy).is_ok() {
                            info!(
                                "attack to enemy at range {}",
                                my_tower.pos().get_range_to(enemy.pos())
                            );
                            is_done = true;
                        }
                    }

                    if is_done {
                        continue;
                    }

                    debug!("heal creeps {}", my_tower.id());
                    let my_creeps = my_tower
                        .room()
                        .expect("room is not visible to you")
                        .find(find::MY_CREEPS, None);

                    for my_creep in my_creeps {
                        if my_creep.hits() < my_creep.hits_max() {
                            debug!("heal my creep {}", my_tower.id());
                            let r = my_tower.heal(&my_creep);

                            if r.is_ok() {
                                info!("heal my creep!!");
                                is_done = true;
                                break;
                            }
                        }
                    }
                    if is_done {
                        continue;
                    }

                    if my_tower.store().get_used_capacity(Some(ResourceType::Energy))
                        > (my_tower.store().get_capacity(Some(ResourceType::Energy)) * 2 / 3)
                    {
                        debug!("repair structure {}", my_tower.id());

                        let my_structures = room_structures(&room);

                        // 残り時間が短いものを優先.
                        for structure in my_structures.iter() {
                            if structure.structure_type() != StructureType::Wall {
                                if check_repairable(structure) {
                                    if get_live_tickcount(structure).unwrap_or(10000)
                                        <= REPAIRER_DYING_THRESHOLD
                                    {
                                        if let Some(repairable) = structure.as_repairable() {
                                            let r = my_tower.repair(repairable);
                                            if r.is_ok() {
                                                info!("repair my structure!!");
                                                is_done = true;
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if is_done {
                            continue;
                        }

                        // 損傷率が最も高いものを1つ選んで直す。
                        //
                        // 旧実装は `min + (avg - min) / 1000` を閾値にしていたが、
                        // get_hp は破損した建造物しか返さないので avg/min は
                        // 「破損建造物の平均・最小」であり、この式は実質「最小値付近の
                        // ものだけ」を意味していた。しかも1周目では壁を除外しているのに
                        // ここでは除外していないため、HP1 の建設直後の壁が常に最小値を
                        // 占め、タワーの修理がほぼ全て壁に吸われていた。
                        // 壁とランパートは専用の運用 (目標HPまで少しずつ盛る) が要るので、
                        // ここでは対象外にする。
                        let repair_target = my_structures
                            .iter()
                            .filter(|s| {
                                s.structure_type() != StructureType::Wall
                                    && s.structure_type() != StructureType::Rampart
                                    && check_repairable(s)
                            })
                            .filter_map(|s| {
                                let attackable = s.as_attackable()?;
                                let max = attackable.hits_max();
                                if max == 0 {
                                    return None;
                                }
                                // 損傷率を千分率で。整数比較にして f64 を避ける。
                                let damage_permille =
                                    (max - attackable.hits()) as u64 * 1000 / max as u64;
                                Some((damage_permille, s))
                            })
                            .max_by_key(|(damage, _)| *damage);

                        if let Some((_, structure)) = repair_target {
                            if let Some(repairable) = structure.as_repairable() {
                                if my_tower.repair(repairable).is_ok() {
                                    info!(
                                        "repair my structure!!:{:?}",
                                        structure.structure_type()
                                    );
                                    is_done = true;
                                }
                            }
                        }
                        if is_done {
                            continue;
                        }
                    }
                }

                _ => {}
            }
        }
    }
}
