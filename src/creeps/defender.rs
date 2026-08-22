//! 防衛 creep。敵の侵入を検知したときだけ生産される。
//!
//! 旧実装は平時から creep 総数の 1/3 に攻撃パーツを混ぜていた。ハイブリッド
//! body は攻撃も経済も中途半端で、しかも高い。防衛の主力はタワー (RCL3+) に
//! 任せ、地上戦力は必要になった時だけ専用 body で出す。
//! 敵が消えたら worker として働き、寿命とともに自然に退役する。
//!
//! # 位置取りの原則 (rampart 戦)
//!
//! 自軍 rampart の上に立てば、被弾は rampart が受けて creep は無傷で攻撃
//! できる。したがって戦闘は可能な限り rampart 上から行う:
//!
//! - 近接 (ATTACK) は敵に最も近い rampart = 栓の最前列で受け止める。
//!   敵が rampart を齧りに隣接してきたところを range 1 で殴り返す。
//! - 遠隔 (RANGED_ATTACK) は敵から距離3以内で最も遠い rampart に下がる。
//!   敵近接 (range 1) の間合いの外、自分の range 3 の内という非対称な
//!   位置から一方的に撃つ。build planner が栓の2マス内側に張る射撃陣地が
//!   ちょうどこの条件を満たす。
//! - 敵の射程圏に rampart が無い (野外の敵 / rampart 未整備) 場合のみ
//!   追撃する。このとき遠隔は距離3を保ち、詰められたら離れる (カイト)。

use crate::util::*;
use log::*;
use screeps::local::Position;
use screeps::prelude::*;
use screeps::{find, Creep, Part, StructureType};
use std::collections::HashSet;

/// rampart を戦闘位置として使う、敵からの最大距離。
/// 遠隔の射程 (RANGED_ATTACK = 3) と同じ。これより遠い rampart に
/// 立っても攻撃が届かない。
const RAMPART_ENGAGE_RANGE: u32 = 3;

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

    let is_ranged = creep
        .body()
        .iter()
        .any(|p| p.part() == Part::RangedAttack && p.hits() > 0);

    // --- 攻撃 (攻撃と移動は同 tick に併発できるので、先に撃ってから動く) ---
    if is_ranged {
        let in_range = enemies
            .iter()
            .filter(|e| creep.pos().get_range_to(e.pos()) <= 3)
            .count();
        if in_range >= 3 {
            // 密集した敵には全体攻撃のほうが総ダメージが出る。
            let _ = creep.ranged_mass_attack();
        } else if let Some(enemy) = enemies
            .iter()
            .filter(|e| creep.pos().get_range_to(e.pos()) <= 3)
            .min_by_key(|e| e.hits())
        {
            if creep.ranged_attack(enemy).is_ok() {
                info!("{} ranged-attacks enemy", creep.name());
            }
        }
    } else if let Some(enemy) = enemies.iter().find(|e| creep.pos().is_near_to(e.pos())) {
        if creep.attack(enemy).is_ok() {
            info!("{} attacks enemy", creep.name());
        }
    }

    // --- 位置取り ---
    let Some(nearest) = enemies
        .iter()
        .min_by_key(|e| creep.pos().get_range_to(e.pos()))
    else {
        return;
    };
    let enemy_pos = nearest.pos();

    // 自軍 creep が立っているマスへは移動できない (スタック不可)。
    let occupied: HashSet<(u8, u8)> = room
        .find(find::MY_CREEPS, None)
        .iter()
        .filter(|c| c.name() != creep.name())
        .map(|c| (c.pos().x().u8(), c.pos().y().u8()))
        .collect();

    // 敵の周囲 RAMPART_ENGAGE_RANGE 以内の自軍 rampart から戦闘位置を選ぶ。
    let mut best: Option<(u32, Position)> = None;
    for s in room_structures(&room).iter() {
        if s.structure_type() != StructureType::Rampart || !check_my_structure(s) {
            continue;
        }
        let p = s.pos();
        if occupied.contains(&(p.x().u8(), p.y().u8())) {
            continue;
        }
        let d = p.get_range_to(enemy_pos);
        if d > RAMPART_ENGAGE_RANGE {
            continue;
        }
        // 小さいほど良いスコア。近接は前列 (敵に近い) を、遠隔は後列
        // (射程内で敵から遠い = 敵近接が届かない) を取る。
        let score = if is_ranged { RAMPART_ENGAGE_RANGE - d } else { d };
        if best.is_none_or(|(b, _)| score < b) {
            best = Some((score, p));
        }
    }

    if let Some((_, target)) = best {
        if creep.pos() == target {
            return; // 配置完了。rampart の上から撃ち続ける。
        }
        let res = find_path(creep, &target, 0);
        if !res.path().is_empty() {
            let _ = move_by_search_result(creep, &res);
        }
        return;
    }

    // --- rampart で受けられない敵は野戦で追う ---
    // 遠隔は距離3を保つ。詰められたら反対方向へ離れて間合いを作り直す
    // (rampart 外では敵近接に隣接されると一方的に不利)。
    if is_ranged {
        let d = creep.pos().get_range_to(enemy_pos);
        if d <= 1 {
            if let Some(dir) = creep.pos().get_direction_to(enemy_pos) {
                let _ = creep.move_direction(dir.multi_rot(4));
            }
            return;
        }
        if d > 3 {
            let res = find_nearest_enemy(creep, 3);
            if !res.path().is_empty() {
                let _ = move_by_search_result(creep, &res);
            }
        }
        return;
    }

    let res = find_nearest_enemy(creep, 1);
    if !res.path().is_empty() {
        let _ = move_by_search_result(creep, &res);
    }
}
