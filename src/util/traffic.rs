//! 交通管理 (設計: docs/traffic-design.md)。
//!
//! 移動を「即時発行」から「意図の登録 + tick 末の一括解決」に変える。
//! ロール処理は request_move で意図を登録するだけにし、creep_loop の後で
//! resolve_traffic() が突き合わせて move_direction を発行する。
//!
//! **Phase 1 (現状)**: 配管のみ。意図は全許可で発行する。従来の
//! move_by_path と等価 (経路は毎 tick 引き直すので先頭1マスで十分)。
//! Phase 2 で隊列 / swap / 押し出しの解決器を有効化する。
//!
//! 例外: defender の野戦離脱 (defender.rs の move_direction 直呼び) は
//! 経路によらない1歩なので意図を経由しない。解決器導入後も defender は
//! 最優先 (priority 100) で、押し出し対象にもしない。

use log::*;
use screeps::local::Position;
use screeps::pathfinder::SearchResults;
use screeps::prelude::*;
use screeps::Creep;
use std::cell::RefCell;

/// 1件の移動意図。
struct Intent {
    creep: Creep,
    from: Position,
    to: Position,
    /// 競合時の優先度 (設計 §3.2)。Phase 2 の解決器で使う。
    #[allow(dead_code)]
    priority: u8,
}

thread_local! {
    /// tick 内で収集する意図。clear_init_flag が tick 先頭で破棄する。
    static INTENTS: RefCell<Vec<Intent>> = RefCell::new(Vec::new());
}

/// tick 先頭で意図を破棄する。clear_init_flag から呼ばれる。
pub fn clear_traffic() {
    INTENTS.with(|intents| intents.borrow_mut().clear());
}

/// 競合時の優先度 (設計 §3.2)。防衛 > 物流 > 採取 > アップグレード > 汎用。
/// 間隔を空けてあるのは将来の挿入余地。
fn role_priority(creep: &Creep) -> u8 {
    use crate::mem::MemoryExt;
    match creep
        .memory()
        .string(crate::mem::keys::ROLE)
        .ok()
        .flatten()
        .as_deref()
    {
        Some(crate::creeps::ROLE_DEFENDER) => 100,
        Some(crate::creeps::ROLE_HAULER) => 60,
        Some(crate::creeps::ROLE_MINER) => 50,
        Some(crate::creeps::ROLE_UPGRADER) => 40,
        _ => 30,
    }
}

/// 移動意図を登録する。経路の先頭1マスだけを使う (1 tick に1マスしか
/// 進めないので、それ以降は次 tick の再探索が引き直す)。
pub fn request_move(creep: &Creep, res: &SearchResults) {
    // 動けない creep の意図は Phase 2 の隊列判定を狂わせるだけなので
    // 登録しない (設計 §5)。
    if creep.fatigue() > 0 {
        return;
    }
    let from = creep.pos();
    // path の先頭要素が現在地のことがあるため読み飛ばす。
    let Some(&to) = res.path().iter().find(|p| **p != from) else {
        return;
    };
    let priority = role_priority(creep);
    let name = creep.name();
    INTENTS.with(|intents| {
        let mut intents = intents.borrow_mut();
        // 同一 creep の再登録は上書き (最後の意図が勝つ。設計 §5)。
        intents.retain(|i| i.creep.name() != name);
        intents.push(Intent {
            creep: creep.clone(),
            from,
            to,
            priority,
        });
    });
}

/// 収集した意図を move として発行する。creep_loop の後で1回だけ呼ぶ。
/// Phase 1 は全許可 (突き合わせなし)。
pub fn resolve_traffic() {
    INTENTS.with(|intents| {
        let intents = intents.borrow();
        for intent in intents.iter() {
            let Some(dir) = intent.from.get_direction_to(intent.to) else {
                continue;
            };
            if let Err(e) = intent.creep.move_direction(dir) {
                debug!("{} couldn't move {:?}: {:?}", intent.creep.name(), dir, e);
            }
        }
    });
}
