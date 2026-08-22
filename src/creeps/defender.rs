//! 防衛 creep。敵の侵入を検知したときだけ生産される。
//!
//! 旧実装は平時から creep 総数の 1/3 に攻撃パーツを混ぜていた。ハイブリッド
//! body は攻撃も経済も中途半端で、しかも高い。防衛の主力はタワー (RCL3+) に
//! 任せ、地上戦力は必要になった時だけ専用 body で出す。
//! 敵が消えたら worker として働き、寿命とともに自然に退役する。

use crate::util::*;
use log::*;
use screeps::prelude::*;
use screeps::Creep;

pub fn run_defender(creep: &Creep) {
    let Some(room) = creep.room() else {
        return;
    };

    let enemies = room_hostiles(&room);

    if enemies.is_empty() {
        // 敵がいない間は遊ばせない。戦闘 body でも運搬や修理の足しにはなる。
        super::worker::run_worker(creep, false);
        return;
    }

    // 最も近い敵を追う。隣接していれば殴る (攻撃と移動は同 tick に出せる)。
    let target = enemies
        .iter()
        .min_by_key(|e| creep.pos().get_range_to(e.pos()));

    let Some(enemy) = target else {
        return;
    };

    if creep.pos().is_near_to(enemy.pos()) {
        if creep.attack(enemy).is_ok() {
            info!("{} attacks enemy", creep.name());
        }
    }

    let res = find_nearest_enemy(creep, 1);
    if !res.path().is_empty() {
        let _ = move_by_search_result(creep, &res);
    }
}
