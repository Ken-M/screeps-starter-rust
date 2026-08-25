//! 交通管理 (設計: docs/traffic-design.md)。
//!
//! 移動を「即時発行」から「意図の登録 + tick 末の一括解決」に変える。
//! ロール処理は request_move で意図を登録するだけにし、creep_loop の後で
//! resolve_traffic() が突き合わせて move_direction を発行する。
//!
//! **Phase 2 (現状)**: 解決器を有効化。部屋ごとに意図を突き合わせ、
//! 隊列 (明け渡し追従) / swap・循環 / 遊休 creep の押し出しを成立させ、
//! 着席・硬直・敵に向かう移動は却下して 1 tick 待つ。
//! 解決器の中核 (resolve_intents) は game API 非依存の純ロジック。
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
    /// 競合時の優先度 (設計 §3.2)。
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

/// 却下がこの回数連続したら警告する。1 tick 待ちは正常なので、
/// 事実上のスタックになっている場合だけ拾う。
const TRAFFIC_DENIED_WARN_EVERY: i32 = 10;

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

/// 定位置に着いた座り仕事か (miner が source 隣 / upgrader が claim 席の上)。
/// 待っても退かない creep の共通判定で、経路コスト (0xff) と交通解決
/// (押し出し不可・隊列の終端) の両方がこれを使う。
pub fn is_parked(creep: &Creep, sources: &[screeps::objects::Source]) -> bool {
    use crate::mem::MemoryExt;
    let cmem = creep.memory();
    match cmem
        .string(crate::mem::keys::ROLE)
        .ok()
        .flatten()
        .as_deref()
    {
        Some(crate::creeps::ROLE_MINER) => {
            sources.iter().any(|s| creep.pos().is_near_to(s.pos()))
        }
        Some(crate::creeps::ROLE_UPGRADER) => {
            let xy = creep.pos().xy();
            cmem.string(crate::mem::keys::UPGRADE_SEAT)
                .ok()
                .flatten()
                .is_some_and(|s| s == format!("{},{}", xy.x.u8(), xy.y.u8()))
        }
        _ => false,
    }
}

/// 部屋内タイル同士の方向。同一マスなら None。
fn dir_between(from: (u8, u8), to: (u8, u8)) -> Option<screeps::Direction> {
    use screeps::Direction::*;
    let dx = (to.0 as i16 - from.0 as i16).signum();
    let dy = (to.1 as i16 - from.1 as i16).signum();
    Some(match (dx, dy) {
        (0, -1) => Top,
        (1, -1) => TopRight,
        (1, 0) => Right,
        (1, 1) => BottomRight,
        (0, 1) => Bottom,
        (-1, 1) => BottomLeft,
        (-1, 0) => Left,
        (-1, -1) => TopLeft,
        _ => return None,
    })
}

fn xy8(pos: Position) -> (u8, u8) {
    (pos.x().u8(), pos.y().u8())
}

/// 意図どおりの move を発行する。
fn issue(intent: &Intent) {
    let Some(dir) = intent.from.get_direction_to(intent.to) else {
        return;
    };
    if let Err(e) = intent.creep.move_direction(dir) {
        debug!("{} couldn't move {:?}: {:?}", intent.creep.name(), dir, e);
    }
}

/// 収集した意図を突き合わせて発行する。creep_loop の後で1回だけ呼ぶ。
/// Phase 2: 部屋ごとに解決器 (resolve_intents) へかけ、隊列 / swap /
/// 押し出しを成立させる。部屋間をまたぐ意図と視界の無い部屋は従来どおり
/// そのまま発行する (設計 §5)。
pub fn resolve_traffic() {
    use std::collections::HashMap;

    INTENTS.with(|intents| {
        let intents = intents.borrow();
        if intents.is_empty() {
            return;
        }

        let mut rooms: HashMap<screeps::local::RoomName, Vec<usize>> = HashMap::new();
        for (i, intent) in intents.iter().enumerate() {
            rooms.entry(intent.from.room_name()).or_default().push(i);
        }

        for (room_name, ids) in rooms {
            let Some(room) = screeps::game::rooms().get(room_name) else {
                for &i in &ids {
                    issue(&intents[i]);
                }
                continue;
            };

            // 部屋内で完結する意図だけを解決器へ。部屋境界をまたぐ移動は
            // 先に発行し、占有上は「動くが追従不可」として扱う (下)。
            let mut data: Vec<IntentData> = Vec::new();
            let mut cross_room: Vec<String> = Vec::new();
            for &i in &ids {
                let intent = &intents[i];
                if intent.to.room_name() == room_name {
                    data.push(IntentData {
                        id: i,
                        to: xy8(intent.to),
                        priority: intent.priority,
                    });
                } else {
                    issue(intent);
                    cross_room.push(intent.creep.name());
                }
            }

            // 占有マップ (設計 §3.3)。
            let sources = room.find(screeps::find::SOURCES, None);
            let intent_by_name: HashMap<String, usize> = data
                .iter()
                .map(|d| (intents[d.id].creep.name(), d.id))
                .collect();
            let mut occupied: HashMap<(u8, u8), Occupant> = HashMap::new();
            let mut by_tile: HashMap<(u8, u8), Creep> = HashMap::new();
            for creep in room.find(screeps::find::CREEPS, None) {
                let tile = xy8(creep.pos());
                if !creep.my() {
                    occupied.insert(tile, Occupant::Enemy);
                    continue;
                }
                let name = creep.name();
                let occ = if let Some(&iid) = intent_by_name.get(&name) {
                    Occupant::Moving(iid)
                } else if cross_room.contains(&name) {
                    // 動くが解決器からは追えない → 追従も押し出しも不可。
                    Occupant::Moving(usize::MAX)
                } else if is_parked(&creep, &sources) {
                    Occupant::Parked
                } else if creep.fatigue() > 0 || is_defender(&creep) {
                    // defender は遊休でも押し出さない (戦闘の一歩を上書きしない)。
                    Occupant::Fatigued
                } else {
                    Occupant::Idle
                };
                occupied.insert(tile, occ);
                by_tile.insert(tile, creep);
            }

            let walkable = |t: (u8, u8)| {
                screeps::local::RoomXY::checked_new(t.0, t.1)
                    .is_ok_and(|xy| super::predicates::is_walkable_tile(&room, xy))
            };
            let road = |t: (u8, u8)| {
                screeps::local::RoomXY::checked_new(t.0, t.1)
                    .is_ok_and(|xy| super::predicates::tile_has_road(&room, xy))
            };

            let res = resolve_intents(&data, &occupied, &walkable, &road);

            for id in &res.granted {
                issue(&intents[*id]);
            }
            for (from_tile, to_tile) in &res.shoves {
                let Some(creep) = by_tile.get(from_tile) else {
                    continue;
                };
                let Some(dir) = dir_between(*from_tile, *to_tile) else {
                    continue;
                };
                debug!("traffic: shove {} {:?}", creep.name(), dir);
                let _ = creep.move_direction(dir);
            }
            // 却下は 1 tick 待つだけの想定だが、同じ creep が同じ行き先で
            // 却下され続けると事実上のスタックになる。連続回数を Memory に
            // 積み、しきい値を超えたら警告する (原因の切り分け用)。
            for id in &res.denied {
                use crate::mem::MemoryExt;
                let intent = &intents[*id];
                let cmem = intent.creep.memory();
                let key = crate::mem::keys::TRAFFIC_DENIED;
                let n = cmem.i32(key).unwrap_or(None).unwrap_or(0) + 1;
                cmem.set(key, n);
                if n % TRAFFIC_DENIED_WARN_EVERY == 0 {
                    warn!(
                        "traffic: {} denied {}x at ({},{}) -> ({},{})",
                        intent.creep.name(),
                        n,
                        intent.from.x().u8(),
                        intent.from.y().u8(),
                        intent.to.x().u8(),
                        intent.to.y().u8()
                    );
                }
            }
            for id in &res.granted {
                use crate::mem::MemoryExt;
                intents[*id]
                    .creep
                    .memory()
                    .del(crate::mem::keys::TRAFFIC_DENIED);
            }
        }
    });
}

fn is_defender(creep: &Creep) -> bool {
    use crate::mem::MemoryExt;
    creep
        .memory()
        .string(crate::mem::keys::ROLE)
        .ok()
        .flatten()
        .as_deref()
        == Some(crate::creeps::ROLE_DEFENDER)
}

// ---------------------------------------------------------------------------
// 解決器の純ロジック (Phase 2)。game API に依存しない (設計 §7)。
// ---------------------------------------------------------------------------

/// 解決器に渡す意図。座標は部屋内の (x, y)。
/// 現在位置は occupied 側の Moving(id) が持つので from は要らない。
#[derive(Clone, Copy)]
pub struct IntentData {
    pub id: usize,
    pub to: (u8, u8),
    pub priority: u8,
}

/// マスの占有者の分類 (設計 §3.3)。
#[derive(Clone, Copy, PartialEq)]
pub enum Occupant {
    /// 着席 (miner / 指定席の upgrader)。退かない。
    Parked,
    /// 今 tick は動けない (fatigue 中・spawning・押し出し不可の防衛など)。
    Fatigued,
    /// 意図なしの遊休。押し出し可。
    Idle,
    /// 移動意図あり (値は intent id)。
    Moving(usize),
    /// 敵。障害物。
    Enemy,
}

/// 解決結果。granted / denied は intent id、shoves は (退く creep のいるマス, 退避先)。
#[derive(Default)]
pub struct Resolution {
    pub granted: Vec<usize>,
    pub shoves: Vec<((u8, u8), (u8, u8))>,
    pub denied: Vec<usize>,
}

#[derive(Clone, Copy, PartialEq)]
enum Verdict {
    Unknown,
    Granted,
    Denied,
}

/// 意図と占有状態から、許可する移動と押し出しを決める (設計 §4)。
///
/// 優先度降順に処理し、行き先の占有チェーンを辿る:
/// - 空きマス → 許可 / 隊列 (許可済みの後続) → 許可 / 循環 (swap 含む) → 許可
/// - 遊休 creep → 深さ1の押し出しを試行
/// - 着席・硬直・敵 → 却下 (1 tick 待ち)
/// 許可した行き先は予約し、同一マスの取り合いを事前に排除する。
pub fn resolve_intents(
    intents: &[IntentData],
    occupied: &std::collections::HashMap<(u8, u8), Occupant>,
    walkable: &dyn Fn((u8, u8)) -> bool,
    road: &dyn Fn((u8, u8)) -> bool,
) -> Resolution {
    use std::collections::{HashMap, HashSet};

    // id → 配列添字。id は呼び出し側の連番とは限らない。
    let index: HashMap<usize, usize> = intents.iter().enumerate().map(|(i, d)| (d.id, i)).collect();
    let mut verdicts: Vec<Verdict> = vec![Verdict::Unknown; intents.len()];
    let mut reserved: HashSet<(u8, u8)> = HashSet::new();
    let mut shoves: Vec<((u8, u8), (u8, u8))> = Vec::new();

    // 優先度降順 (同値は登録順) に処理する。
    let mut order: Vec<usize> = (0..intents.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(intents[i].priority));

    for start in order {
        if verdicts[start] != Verdict::Unknown {
            continue;
        }

        // 行き先のチェーンを辿り、末端の状態で一括判定する。
        let mut chain: Vec<usize> = Vec::new();
        let mut visited: HashSet<usize> = HashSet::new();
        let mut cur = start;
        let outcome = loop {
            visited.insert(cur);
            chain.push(cur);
            let to = intents[cur].to;

            if reserved.contains(&to) {
                break Verdict::Denied;
            }
            match occupied.get(&to) {
                None => break Verdict::Granted,
                Some(Occupant::Enemy) | Some(Occupant::Parked) | Some(Occupant::Fatigued) => {
                    break Verdict::Denied;
                }
                Some(Occupant::Idle) => {
                    // 深さ1の押し出し (設計 §4.2)。非道路マス優先。
                    let mut cand: Vec<(u8, u8)> = Vec::new();
                    for dx in -1..=1i16 {
                        for dy in -1..=1i16 {
                            if dx == 0 && dy == 0 {
                                continue;
                            }
                            let (nx, ny) = (to.0 as i16 + dx, to.1 as i16 + dy);
                            if !(0..50).contains(&nx) || !(0..50).contains(&ny) {
                                continue;
                            }
                            let t = (nx as u8, ny as u8);
                            if walkable(t) && !occupied.contains_key(&t) && !reserved.contains(&t)
                            {
                                cand.push(t);
                            }
                        }
                    }
                    cand.sort_by_key(|&t| (road(t), t.1, t.0));
                    if let Some(&t) = cand.first() {
                        shoves.push((to, t));
                        reserved.insert(t);
                        break Verdict::Granted;
                    }
                    break Verdict::Denied;
                }
                Some(Occupant::Moving(next_id)) => {
                    let Some(&next) = index.get(next_id) else {
                        // 意図の登録が無い id (別部屋など)。動くか不明 → 待つ。
                        break Verdict::Denied;
                    };
                    match verdicts[next] {
                        Verdict::Granted => break Verdict::Granted, // 隊列: 明け渡し追従
                        Verdict::Denied => break Verdict::Denied,
                        Verdict::Unknown => {
                            if visited.contains(&next) {
                                // 循環 (相互 swap を含む) → チェーンごと成立
                                break Verdict::Granted;
                            }
                            cur = next;
                        }
                    }
                }
            }
        };

        // チェーンを末端側から確定する。途中で行き先が予約済みなら
        // そこから手前 (追従側) は全部却下 (投げ縄形の同一マス競合対策)。
        match outcome {
            Verdict::Granted => {
                let mut ok = true;
                for &cid in chain.iter().rev() {
                    let to = intents[cid].to;
                    if ok && !reserved.contains(&to) {
                        verdicts[cid] = Verdict::Granted;
                        reserved.insert(to);
                    } else {
                        verdicts[cid] = Verdict::Denied;
                        ok = false;
                    }
                }
            }
            _ => {
                for &cid in chain.iter() {
                    verdicts[cid] = Verdict::Denied;
                }
            }
        }
    }

    let mut result = Resolution::default();
    for (i, v) in verdicts.iter().enumerate() {
        match v {
            Verdict::Granted => result.granted.push(intents[i].id),
            _ => result.denied.push(intents[i].id),
        }
    }
    result.shoves = shoves;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn intent(id: usize, _from: (u8, u8), to: (u8, u8), priority: u8) -> IntentData {
        IntentData { id, to, priority }
    }

    /// 全マス歩行可・道路なしの解決。
    fn solve(
        intents: &[IntentData],
        occupied: &HashMap<(u8, u8), Occupant>,
    ) -> Resolution {
        resolve_intents(intents, occupied, &|_| true, &|_| false)
    }

    #[test]
    fn 空きマスへの単独移動は許可される() {
        let intents = [intent(0, (1, 1), (2, 1), 30)];
        let occupied = HashMap::from([((1, 1), Occupant::Moving(0))]);
        let res = solve(&intents, &occupied);
        assert_eq!(res.granted, vec![0]);
        assert!(res.denied.is_empty());
    }

    #[test]
    fn 対向2体のswapは両方許可される() {
        let intents = [intent(0, (1, 1), (2, 1), 30), intent(1, (2, 1), (1, 1), 30)];
        let occupied = HashMap::from([
            ((1, 1), Occupant::Moving(0)),
            ((2, 1), Occupant::Moving(1)),
        ]);
        let res = solve(&intents, &occupied);
        assert_eq!(res.granted.len(), 2);
        assert!(res.denied.is_empty());
    }

    #[test]
    fn 三体の循環は全員許可される() {
        let intents = [
            intent(0, (1, 1), (2, 1), 30),
            intent(1, (2, 1), (2, 2), 30),
            intent(2, (2, 2), (1, 1), 30),
        ];
        let occupied = HashMap::from([
            ((1, 1), Occupant::Moving(0)),
            ((2, 1), Occupant::Moving(1)),
            ((2, 2), Occupant::Moving(2)),
        ]);
        let res = solve(&intents, &occupied);
        assert_eq!(res.granted.len(), 3);
    }

    #[test]
    fn 隊列は全員許可される() {
        let intents = [
            intent(0, (1, 1), (2, 1), 30),
            intent(1, (2, 1), (3, 1), 30),
            intent(2, (3, 1), (4, 1), 30),
        ];
        let occupied = HashMap::from([
            ((1, 1), Occupant::Moving(0)),
            ((2, 1), Occupant::Moving(1)),
            ((3, 1), Occupant::Moving(2)),
        ]);
        let res = solve(&intents, &occupied);
        assert_eq!(res.granted.len(), 3);
    }

    #[test]
    fn 先頭が着席creepなら隊列全員が却下される() {
        let intents = [
            intent(0, (1, 1), (2, 1), 30),
            intent(1, (2, 1), (3, 1), 30),
        ];
        let occupied = HashMap::from([
            ((1, 1), Occupant::Moving(0)),
            ((2, 1), Occupant::Moving(1)),
            ((3, 1), Occupant::Parked),
        ]);
        let res = solve(&intents, &occupied);
        assert!(res.granted.is_empty());
        assert_eq!(res.denied.len(), 2);
    }

    #[test]
    fn 遊休creepは押し出され_元の意図は許可される() {
        let intents = [intent(0, (1, 1), (2, 1), 30)];
        let occupied = HashMap::from([
            ((1, 1), Occupant::Moving(0)),
            ((2, 1), Occupant::Idle),
        ]);
        let res = solve(&intents, &occupied);
        assert_eq!(res.granted, vec![0]);
        assert_eq!(res.shoves.len(), 1);
        assert_eq!(res.shoves[0].0, (2, 1));
        // 退避先は空きマス (自分の元いたマスや依頼者のマスではない)。
        assert_ne!(res.shoves[0].1, (2, 1));
        assert_ne!(res.shoves[0].1, (1, 1));
    }

    #[test]
    fn 押し出し先が無ければ却下される() {
        let intents = [intent(0, (1, 1), (2, 1), 30)];
        let occupied = HashMap::from([
            ((1, 1), Occupant::Moving(0)),
            ((2, 1), Occupant::Idle),
        ]);
        // (1,1) と (2,1) 以外は全部壁。
        let walkable = |t: (u8, u8)| t == (1, 1) || t == (2, 1);
        let res = resolve_intents(&intents, &occupied, &walkable, &|_| false);
        assert!(res.granted.is_empty());
        assert_eq!(res.denied, vec![0]);
        assert!(res.shoves.is_empty());
    }

    #[test]
    fn 同一マスを取り合ったら優先度の高い方が勝つ() {
        let intents = [
            intent(0, (1, 1), (2, 2), 30), // worker
            intent(1, (3, 3), (2, 2), 60), // hauler
        ];
        let occupied = HashMap::from([
            ((1, 1), Occupant::Moving(0)),
            ((3, 3), Occupant::Moving(1)),
        ]);
        let res = solve(&intents, &occupied);
        assert_eq!(res.granted, vec![1]);
        assert_eq!(res.denied, vec![0]);
    }

    #[test]
    fn 押し出し先は許可済み意図の行き先と重複しない() {
        // hauler が (5,5) へ移動予約。worker が (4,5) の遊休を押すが、
        // 退避先の候補は (5,5) しか歩けない → 予約済みなので押せず却下。
        let intents = [
            intent(0, (3, 5), (4, 5), 30),
            intent(1, (6, 5), (5, 5), 60),
        ];
        let occupied = HashMap::from([
            ((3, 5), Occupant::Moving(0)),
            ((6, 5), Occupant::Moving(1)),
            ((4, 5), Occupant::Idle),
        ]);
        let walkable = |t: (u8, u8)| matches!(t, (3, 5) | (4, 5) | (5, 5) | (6, 5));
        let res = resolve_intents(&intents, &occupied, &walkable, &|_| false);
        assert_eq!(res.granted, vec![1]);
        assert_eq!(res.denied, vec![0]);
        assert!(res.shoves.is_empty());
    }

    #[test]
    fn fatigue中のcreepは押し出せず隊列も止まる() {
        let intents = [intent(0, (1, 1), (2, 1), 30)];
        let occupied = HashMap::from([
            ((1, 1), Occupant::Moving(0)),
            ((2, 1), Occupant::Fatigued),
        ]);
        let res = solve(&intents, &occupied);
        assert!(res.granted.is_empty());
        assert_eq!(res.denied, vec![0]);
    }

    #[test]
    fn 押し出しは非道路マスを優先する() {
        let intents = [intent(0, (1, 1), (2, 1), 30)];
        let occupied = HashMap::from([
            ((1, 1), Occupant::Moving(0)),
            ((2, 1), Occupant::Idle),
        ]);
        // (2,1) の隣は (3,1) と (2,2) だけ歩ける。(3,1) は道路。
        let walkable = |t: (u8, u8)| matches!(t, (1, 1) | (2, 1) | (3, 1) | (2, 2));
        let road = |t: (u8, u8)| t == (3, 1);
        let res = resolve_intents(&intents, &occupied, &walkable, &road);
        assert_eq!(res.shoves, vec![((2, 1), (2, 2))]);
    }
}
