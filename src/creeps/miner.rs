//! 静的採掘者。
//!
//! source の隣の container に住み着き、二度と動かずに掘り続ける。
//!
//! 分業していない creep は「source まで歩く → 掘る → 拠点へ戻る → 降ろす」を
//! 1体で繰り返す。この部屋は spawn と source が20マス以上離れているため往復に
//! 80〜90 tick かかり、実際に掘っている時間はごくわずかになる。
//!
//! 採掘を静的にすると移動時間がゼロになり、WORK パーツをフルに回せる。
//! WORK 5個で毎tick 10エネルギー、これは source の再生速度 (3000 ÷ 300 tick) と
//! ちょうど一致するので、採掘者1体で source 1つを汲み尽くせる。
//! 運んでくるのは運搬者の仕事になる。

use crate::mem::MemoryExt;
use crate::util::*;
use log::*;
use screeps::prelude::*;
use screeps::{find, Creep, ResourceType, StructureType};

pub fn run_miner(creep: &Creep) {
    let name = creep.name();
    let cmem = creep.memory();

    let Some(room) = creep.room() else {
        return;
    };

    // 担当の source を決める。一度決めたら変えない。
    // 変えると採掘者どうしが同じ source を取り合って往復する。
    let sources = room.find(find::SOURCES, None);
    if sources.is_empty() {
        return;
    }

    let assigned = cmem.string("mine_at").ok().flatten();

    let source = match assigned.as_deref().and_then(|id| {
        sources
            .iter()
            .find(|s| String::from(s.id().to_string()) == id)
    }) {
        Some(s) => s.clone(),
        None => {
            // まだ担当が無い。他の採掘者が就いていない source を選ぶ。
            let taken: Vec<String> = screeps::game::creeps()
                .values()
                .filter(|c| c.name() != name)
                .filter_map(|c| c.memory().string("mine_at").ok().flatten())
                .collect();

            let Some(free) = sources
                .iter()
                .find(|s| !taken.contains(&s.id().to_string()))
                .or_else(|| sources.first())
            else {
                return;
            };

            cmem.set("mine_at", free.id().to_string());
            info!("{} assigned to source {}", name, free.id());
            free.clone()
        }
    };

    // 定位置は source 隣の container の上。container がまだ無ければ source の隣。
    let seat = seat_position(&room, source.pos());

    if creep.pos() != seat {
        // 定位置へ向かう。到着したら以後は動かない。
        let res = find_path(creep, &seat, 0);
        if res.path().is_empty() {
            // 定位置に他の creep がいる等で到達できない。隣接まで寄れれば掘れる。
            if !creep.pos().is_near_to(source.pos()) {
                let approach = find_path(creep, &source.pos(), 1);
                let _ = move_by_search_result(creep, &approach);
                return;
            }
        } else {
            let _ = move_by_search_result(creep, &res);
            // 隣接していれば移動しながらでも掘れる。
            if !creep.pos().is_near_to(source.pos()) {
                return;
            }
        }
    }

    // 掘る。
    if let Err(e) = creep.harvest(&source) {
        debug!("{} couldn't harvest: {:?}", name, e);
    }

    // 足下や隣の container へ移す。
    // CARRY を1個だけ持たせてあるので、毎tick移せば溢れない。
    if creep.store().get_used_capacity(Some(ResourceType::Energy)) == 0 {
        return;
    }

    let container = room_structures(&room).iter().find(|s| {
        s.structure_type() == StructureType::Container && s.pos().is_near_to(creep.pos())
    }).cloned();

    if let Some(container) = container {
        if let Some(transferable) = container.as_transferable() {
            let _ = creep.transfer(transferable, ResourceType::Energy, None);
        }
    }
    // container がまだ無い間は creep の中に溜まる。溢れた分は地面に落ちるので、
    // 運搬者が拾っていく。
}

/// 採掘者の定位置。source 隣の container があればその上、無ければ隣接の空きマス。
fn seat_position(room: &screeps::objects::Room, source_pos: screeps::Position) -> screeps::Position {
    let container = room_structures(room).iter().find(|s| {
        s.structure_type() == StructureType::Container && s.pos().is_near_to(source_pos)
    }).map(|s| s.pos());

    container.unwrap_or(source_pos)
}
