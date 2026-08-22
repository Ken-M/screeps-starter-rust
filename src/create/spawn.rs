use std::usize;

use crate::mem::{self, MemoryExt};
use log::*;
use screeps::action_error_codes::SpawnCreepErrorCode;
use screeps::prelude::*;
use screeps::objects::SpawnOptions;
use screeps::{find, game, Part, StructureType};


/// この体数を割ったら、body の大きさを待たずに最小構成でも即座に生産する。
/// 「大きい creep を待つ」判断は、艦隊に余裕があるときだけ許される。
const EMERGENCY_CREEP_FLOOR: i32 = 4;

/// 回復段階 (人口が目標の 2/3 未満) の設計予算の下限。
/// これ以上貯まっていれば手持ちで即生産する。基本ユニット1個 (250) より
/// 少し上、spawn の自己回復上限 (300) と同じ。
const RECOVERY_MIN_BUDGET: u32 = 300;

/// セーフモードを張るかを判定して、必要なら発動する。
///
/// 旧実装は「spawnのHPが満タンでない」「creep数が上限の1/3未満」「攻撃パーツ持ちが0」
/// のOR条件だったが、どれも被攻撃を意味しない。実際に敵0体の状態で発動し、部屋唯一の
/// セーフモードを浪費した (コスト1000ゴジウム / 効果20000tick / クールダウン50000tick)。
///
/// 判定を「実害の検知」に置き換える:
///   - 攻撃能力を持つ敵が実在し、かつ
///   - 重要建造物のHPがこのtickで実際に減った
/// の AND。HPは前tick値をMemoryに持って差分を取る。
fn check_safe_mode(spawn: &screeps::objects::StructureSpawn) {
    let Some(room) = spawn.room() else {
        return;
    };
    let Some(controller) = room.controller() else {
        return;
    };

    // 重要建造物 (spawn / storage / terminal / tower) の現在HP合計。
    let mut total_hits: u32 = 0;
    for structure in room.find(find::STRUCTURES, None) {
        let is_critical = matches!(
            structure.structure_type(),
            StructureType::Spawn
                | StructureType::Storage
                | StructureType::Terminal
                | StructureType::Tower
        );
        if !is_critical || !crate::util::check_my_structure(&structure) {
            continue;
        }
        if let Some(attackable) = structure.as_attackable() {
            total_hits += attackable.hits();
        }
    }

    // 前tickとの差分を取る。キーは部屋ごと。
    let root = mem::root();
    let key = format!("critical_hits_{}", room.name());
    let prev_hits = root.i32(&key).unwrap_or(None);
    root.set(&key, total_hits as i32);

    // 初回 (前tick値なし) は比較できないので判定しない。
    let Some(prev_hits) = prev_hits else {
        return;
    };
    let is_damaged_now = (total_hits as i32) < prev_hits;

    if !is_damaged_now {
        return;
    }

    // 攻撃能力を持つ敵が実在するか。単なる偵察creepでは発動しない。
    let has_armed_hostile = room.find(find::HOSTILE_CREEPS, None).iter().any(|enemy| {
        enemy.body().iter().any(|part| {
            part.hits() > 0
                && matches!(
                    part.part(),
                    Part::Attack | Part::RangedAttack | Part::Work | Part::Heal
                )
        })
    });

    if !has_armed_hostile {
        // 敵がいないのにHPが減るのは自然崩壊 (rampart等)。修理の仕事であって
        // セーフモードの出番ではない。
        info!(
            "critical structures lost {} hits without armed hostiles; not a raid",
            prev_hits - total_hits as i32
        );
        return;
    }

    // 発動可能な状態か。すでに発動中/クールダウン中/在庫切れなら呼ぶだけ無駄。
    if controller.safe_mode().is_some() {
        warn!("under attack, safe mode already active");
        return;
    }
    if controller.safe_mode_cooldown().is_some() {
        warn!("under attack, but safe mode is on cooldown");
        return;
    }
    if controller.safe_mode_available() == 0 {
        warn!("under attack, but no safe mode available");
        return;
    }

    warn!(
        "under attack! lost {} hits. activating safe mode ({} available)",
        prev_hits - total_hits as i32,
        controller.safe_mode_available()
    );

    match controller.activate_safe_mode() {
        Ok(()) => warn!("safe mode activated"),
        Err(e) => warn!("couldn't activate safe mode: {:?}", e),
    }
}


/// ロールに合わせた body を組む。
///
/// 旧実装は全 creep に同じ汎用構成 (MOVE/MOVE/CARRY/WORK の繰り返し) を配り、
/// さらに総数の 1/3 に攻撃パーツを混ぜていた。ハイブリッド body は攻撃も経済も
/// 中途半端で高くつく。ロールごとに最適な構成を組み、戦闘 body は defender
/// 専用にする。
fn build_body(role: &str, energy: u32) -> Vec<Part> {
    match role {
        crate::creeps::ROLE_MINER => {
            // 動かないので MOVE は1個。CARRY 1個は container へ移すため。
            // WORK 5個で毎tick 10エネルギー = source の再生速度と一致するので、
            // それ以上積んでも無駄になる。
            let mut body = vec![Part::Move, Part::Carry];
            let mut left = energy.saturating_sub(Part::Move.cost() + Part::Carry.cost());
            while body.len() < screeps::constants::MAX_CREEP_SIZE as usize
                && body.iter().filter(|p| **p == Part::Work).count() < 5
                && left >= Part::Work.cost()
            {
                body.push(Part::Work);
                left -= Part::Work.cost();
            }
            // WORK が1個も積めないなら採掘者として成立しない。
            if body.iter().any(|p| *p == Part::Work) {
                body
            } else {
                Vec::new()
            }
        }

        crate::creeps::ROLE_HAULER | crate::creeps::ROLE_CARRIER_MINERAL => {
            // 掘らないので CARRY と MOVE だけ。平地で満載でも全速が出る 1:1。
            unit_body(&[Part::Carry, Part::Move], energy)
        }

        crate::creeps::ROLE_DEFENDER => {
            // 戦闘専用。MOVE を前に並べ、被弾はまず MOVE が受ける
            // (ATTACK が残っていれば反撃は続けられる)。
            let unit_cost = Part::Move.cost() + Part::Attack.cost();
            let n = (energy / unit_cost) as usize;
            let n = n.min(screeps::constants::MAX_CREEP_SIZE as usize / 2);
            if n == 0 {
                return Vec::new();
            }
            let mut body = vec![Part::Move; n];
            body.extend(std::iter::repeat(Part::Attack).take(n));
            body
        }

        // worker とその他は汎用構成。平地で満載でも全速の 2:1:1。
        _ => unit_body(&[Part::Move, Part::Move, Part::Carry, Part::Work], energy),
    }
}

/// 単位構成を予算いっぱいまで繰り返す。
fn unit_body(unit: &[Part], energy: u32) -> Vec<Part> {
    let unit_cost: u32 = unit.iter().map(|p| p.cost()).sum();
    let mut body = Vec::new();
    let mut left = energy;
    while body.len() + unit.len() <= screeps::constants::MAX_CREEP_SIZE as usize
        && left >= unit_cost
    {
        body.extend_from_slice(unit);
        left -= unit_cost;
    }
    body
}

pub fn do_spawn() {
    let colony = crate::creeps::ColonyState::observe();
    let num_total_creep = colony.total_creeps;

    // 人口の上限はロール目標の合計。
    //
    // 旧実装は MAX_NUM_OF_CREEPS = 14 の固定値だった。ロール目標が構造から
    // 決まるようになった今、目標の合計が「必要な人口」そのものであり、
    // 固定上限は目標との間で矛盾を起こすだけになった (実際に目標合計16に対し
    // 上限14で、優先度下位の worker が恒常的に2体不足していた)。
    let population_target = crate::creeps::total_role_target(&colony);

    for spawn in game::spawns().values() {
        debug!("running spawn {}", spawn.name());

        check_safe_mode(&spawn);

        if num_total_creep >= population_target {
            continue;
        }

        // 生産中の spawn は body 計算をしても無駄。
        if spawn.spawning().is_some() {
            continue;
        }

        let Some(room) = spawn.room() else {
            continue;
        };

        // この tick に使えるエネルギーと、部屋の理論上限 (spawn + extension)。
        let available = room.energy_available();
        let capacity = room.energy_capacity_available();

        let role = crate::creeps::most_needed_role(&colony);

        // 原則: 部屋の上限いっぱいの body を設計し、貯まるまで待つ。
        // 小さい creep の量産より、同じエネルギーで大きい creep を作るほうが
        // body あたりの効率 (寿命1500tickに対する生産時間の比も含め) が良い。
        //
        // ただし緊急時は待たず、今あるぶんで即座に出す。緊急とは:
        // - 人口が安全下限を割った
        // - miner または hauler がゼロ (ロジスティクス崩壊)
        //
        // 特に hauler ゼロは危険なデッドロックを作る。extension に補給する者が
        // いないため使えるエネルギーは spawn の自己回復分 300 が上限になるが、
        // 「容量 450 で設計した hauler (400)」はそれを超えていて永遠に買えない。
        // 実際に旧世代 creep の一斉寿命切れで hauler が全滅し、人口 11 → 4 の
        // 崩壊と生産停止が起きた。
        let logistics_down = colony.num_miners == 0 || colony.num_haulers == 0;
        let emergency = num_total_creep < EMERGENCY_CREEP_FLOOR || logistics_down;

        // 回復段階: 人口が目標の 2/3 未満。
        //
        // 「緊急=即時生産」と「平時=容量いっぱいで待つ」の二値だと、崩壊からの
        // 回復が這う。緊急モードが最小個体を1体作って解除された後、
        // 「容量500の body が買えるまで待つ」に戻るが、最小個体の細い収入では
        // そこまでなかなか届かない (実測: エネルギー364/500で横ばい、生産なし)。
        // 回復中は完璧な body を待たず、そこそこの body を数で揃えて
        // 収入自体を立て直す。
        let recovery = num_total_creep * 3 < population_target * 2;

        let budget = if emergency {
            available
        } else if recovery {
            // 300 以上貯まっていれば手持ちで即生産。未満なら 300 まで待つ。
            available.max(RECOVERY_MIN_BUDGET).min(capacity)
        } else {
            capacity
        };

        if emergency {
            warn!(
                "emergency spawn mode: total={} miners={} haulers={} available={}",
                num_total_creep, colony.num_miners, colony.num_haulers, available
            );
        } else if recovery {
            info!(
                "recovery spawn mode: total={}/{} available={}",
                num_total_creep, population_target, available
            );
        }

        let body = build_body(role, budget);
        if body.is_empty() {
            continue;
        }

        let cost: u32 = body.iter().map(|p| p.cost()).sum();
        if cost > available {
            debug!(
                "waiting for {} energy to spawn {} ({} available)",
                cost, role, available
            );
            continue;
        }

        // create a unique name, spawn.
        let name_base = game::time();
        let mut additional = 0;
        let res = loop {
            let name = format!("{}-{}", name_base, additional);
            debug!("try spawn {:?} as {}", body, role);

            // ロールを生成時に書き込む。creep_loop 側の割り当てを待たずに
            // 最初の tick から意図した仕事に就ける。
            let mem_obj = js_sys::Object::new();
            let _ = js_sys::Reflect::set(
                &mem_obj,
                &wasm_bindgen::JsValue::from_str(crate::mem::keys::ROLE),
                &wasm_bindgen::JsValue::from_str(role),
            );
            let opts = SpawnOptions::new().memory(mem_obj.into());

            let res = spawn.spawn_creep_with_options(&body, &name, &opts);

            if res == Err(SpawnCreepErrorCode::NameExists) {
                additional += 1;
            } else {
                break res;
            }
        };

        match res {
            Err(e) => {
                info!("couldn't spawn: {:?}", e);
            }
            Ok(()) => {
                info!("spawn {} as {:?}", role, body);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cost_of(body: &[Part]) -> u32 {
        body.iter().map(|p| p.cost()).sum()
    }

    #[test]
    fn minerはworkを5個で頭打ちにする() {
        // 予算が潤沢でも WORK は source 再生速度と釣り合う5個まで。
        let body = build_body(crate::creeps::ROLE_MINER, 10_000);
        let works = body.iter().filter(|p| **p == Part::Work).count();
        assert_eq!(works, 5);
        assert_eq!(body.iter().filter(|p| **p == Part::Move).count(), 1);
        assert_eq!(body.iter().filter(|p| **p == Part::Carry).count(), 1);
    }

    #[test]
    fn minerはworkが積めない予算では成立しない() {
        // MOVE+CARRY = 100。WORK (100) に届かない予算では空を返す。
        assert!(build_body(crate::creeps::ROLE_MINER, 150).is_empty());
    }

    #[test]
    fn haulerはcarryとmoveだけで構成される() {
        let body = build_body(crate::creeps::ROLE_HAULER, 550);
        assert!(!body.is_empty());
        assert!(body.iter().all(|p| *p == Part::Carry || *p == Part::Move));
        // 1:1 比率。
        let carries = body.iter().filter(|p| **p == Part::Carry).count();
        let moves = body.iter().filter(|p| **p == Part::Move).count();
        assert_eq!(carries, moves);
    }

    #[test]
    fn defenderはmoveが前に並ぶ() {
        // 被弾は body の先頭から。MOVE を先に失っても反撃は続けられる。
        let body = build_body(crate::creeps::ROLE_DEFENDER, 520);
        assert!(!body.is_empty());
        let first_attack = body.iter().position(|p| *p == Part::Attack).unwrap();
        assert!(body[..first_attack].iter().all(|p| *p == Part::Move));
    }

    #[test]
    fn どのbodyも予算とサイズ上限を守る() {
        for role in [
            crate::creeps::ROLE_MINER,
            crate::creeps::ROLE_HAULER,
            crate::creeps::ROLE_DEFENDER,
            crate::creeps::ROLE_WORKER,
        ] {
            for budget in [0, 100, 300, 1000, 12_900] {
                let body = build_body(role, budget);
                assert!(cost_of(&body) <= budget, "{} over budget {}", role, budget);
                assert!(body.len() <= screeps::constants::MAX_CREEP_SIZE as usize);
            }
        }
    }
}
