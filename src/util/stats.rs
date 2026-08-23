//! 部屋単位の統計 (建造物 HP と建設進捗の平均・最小)。
//! repairer と builder の閾値計算に使う。

use super::predicates::get_hp;
use lazy_static::lazy_static;
use log::*;
use screeps::local::RoomName;
use screeps::{find, game};
use std::collections::HashMap;
use std::sync::RwLock;

type ConstructionProgressAverage = HashMap<RoomName, u128>;
type StructureHpAverage = HashMap<RoomName, u128>;
type ConstructionProgressMin = HashMap<RoomName, u128>;
type StructureHpMin = HashMap<RoomName, u128>;

lazy_static! {
    static ref CONSTRUCTION_PROGRESS_AVERAGE_CACHE: RwLock<ConstructionProgressAverage> =
        RwLock::new(HashMap::new());
    static ref STRUCTURE_HP_AVERAGE_CACHE: RwLock<StructureHpAverage> = RwLock::new(HashMap::new());
    static ref CONSTRUCTION_PROGRESS_MIN_CACHE: RwLock<ConstructionProgressMin> =
        RwLock::new(HashMap::new());
    static ref STRUCTURE_HP_MIN_CACHE: RwLock<StructureHpMin> = RwLock::new(HashMap::new());
}

/// 統計キャッシュを破棄する。clear_init_flag から呼ばれる。
pub(super) fn clear_stats_caches() {
    CONSTRUCTION_PROGRESS_AVERAGE_CACHE.write().unwrap().clear();
    STRUCTURE_HP_AVERAGE_CACHE.write().unwrap().clear();
    CONSTRUCTION_PROGRESS_MIN_CACHE.write().unwrap().clear();
    STRUCTURE_HP_MIN_CACHE.write().unwrap().clear();
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
                debug!(
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

                debug!(
                    "{:?}: construction_progress_average:{:?}:min:{:?}",
                    *room_name,
                    sum_of_progress / construction_count,
                    progress_min
                );
            } else {
                construction_progress_average.insert(*room_name, 0);
                // MIN 側にも入れないと、下の get_construction_progress_average が
                // 常にキャッシュミス扱いになり calc_average を毎回呼び直してしまう。
                construction_progress_min.insert(*room_name, 0);
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

        // MIN キャッシュを読む。旧実装は2箇所とも AVERAGE を読んでおり、
        // MIN キャッシュは書かれるだけの dead store になっていた。その結果
        // builder の閾値 (stats.0 + stats.1) / 2 が「平均と最小の中間」ではなく
        // 単なる平均になり、建設優先度の制御が設計どおり効いていなかった。
        let construction_progress_min = CONSTRUCTION_PROGRESS_MIN_CACHE.read().unwrap();
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

        // MIN キャッシュを読む。旧実装は2箇所とも AVERAGE を読んでおり、
        // MIN キャッシュは書かれるだけの dead store になっていた。その結果
        // builder の閾値 (stats.0 + stats.1) / 2 が「平均と最小の中間」ではなく
        // 単なる平均になり、建設優先度の制御が設計どおり効いていなかった。
        let construction_progress_min = CONSTRUCTION_PROGRESS_MIN_CACHE.read().unwrap();
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
