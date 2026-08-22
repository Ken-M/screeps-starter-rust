use crate::constants::*;
use std::usize;

use crate::mem::{self, MemoryExt};
use log::*;
use screeps::action_error_codes::SpawnCreepErrorCode;
use screeps::enums::StructureObject;
use screeps::prelude::*;
use screeps::objects::SpawnOptions;
use screeps::{find, game, Part, ResourceType, StructureType};

const MAX_NUM_OF_CREEPS: u32 = 14;

/// この体数を割ったら、body の大きさを待たずに最小構成でも即座に生産する。
/// 「大きい creep を待つ」判断は、艦隊に余裕があるときだけ許される。
const EMERGENCY_CREEP_FLOOR: i32 = 4;

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
/// 旧実装は全 creep に同じ汎用構成 (MOVE/MOVE/CARRY/WORK の繰り返し) を配っていた。
/// 静的採掘者は動かないので MOVE は1個で足り、その分を WORK に回せる。運搬者は
/// 掘らないので WORK が要らず、CARRY と MOVE だけ積める。同じエネルギーで
/// 仕事量が大きく変わる。
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
            let unit = [Part::Carry, Part::Move];
            let unit_cost: u32 = unit.iter().map(|p| p.cost()).sum();
            let mut body = Vec::new();
            let mut left = energy;
            while body.len() + unit.len() <= screeps::constants::MAX_CREEP_SIZE as usize
                && left >= unit_cost
            {
                body.extend(unit.iter().cloned());
                left -= unit_cost;
            }
            body
        }

        // それ以外は従来どおりの汎用構成。呼び出し側が組む。
        _ => Vec::new(),
    }
}

pub fn do_spawn() {
    let colony = crate::creeps::ColonyState::observe();
    let num_total_creep = colony.total_creeps;

    if num_total_creep >= MAX_NUM_OF_CREEPS as i32 {
        return;
    }

    let root = mem::root();

    let _num_upgrader: i32 = root.i32("num_upgrader").unwrap_or(Some(0)).unwrap_or(0);
    let _num_builder: i32 = root.i32("num_builder").unwrap_or(Some(0)).unwrap_or(0);
    let _num_harvester: i32 = root.i32("num_harvester").unwrap_or(Some(0)).unwrap_or(0);
    let _num_harvester_spawn: i32 = root
        .i32("num_harvester_spawn")
        .unwrap_or(Some(0))
        .unwrap_or(0);
    let _num_harvester_mineral: i32 = root
        .i32("num_harvester_mineral")
        .unwrap_or(Some(0))
        .unwrap_or(0);
    let _num_carrier_mineral: i32 = root
        .i32("num_carrier_mineral")
        .unwrap_or(Some(0))
        .unwrap_or(0);
    let _num_repairer: i32 = root.i32("num_repairer").unwrap_or(Some(0)).unwrap_or(0);

    let opt_num_attackable_short: i32 = root
        .i32("opt_num_attackable_short")
        .unwrap_or(Some(0))
        .unwrap_or(0);
    let opt_num_attackable_long: i32 = root
        .i32("opt_num_attackable_long")
        .unwrap_or(Some(0))
        .unwrap_or(0);

    let cap_worker_carry: i32 = root.i32("cap_worker_carry").unwrap_or(Some(0)).unwrap_or(0);

    for spawn in game::spawns().values() {
        info!("running spawn {}", spawn.name());

        check_safe_mode(&spawn);

        //check energy can be used.
        let all_structures = spawn
            .room()
            .expect("room is not visible to you")
            .find(find::STRUCTURES, None);

        let mut sum_energy = spawn.store().get_used_capacity(Some(ResourceType::Energy));
        let mut extention_cap = spawn.store().get_capacity(Some(ResourceType::Energy));

        for structure in all_structures {
            match structure {
                StructureObject::StructureExtension(extention) => {
                    if extention.my() == true {
                        sum_energy += extention
                            .store()
                            .get_used_capacity(Some(ResourceType::Energy));
                        extention_cap +=
                            extention.store().get_capacity(Some(ResourceType::Energy));
                    }
                }
                _ => {
                    // other structure
                }
            }
        }

        let body_unit = [Part::Move, Part::Move, Part::Carry, Part::Work];
        let body_short_atk_unit = [Part::Move, Part::Attack];
        let body_long_atk_unit = [Part::Move, Part::RangedAttack];

        let body_cost: u32 = body_unit.iter().map(|p| p.cost()).sum();
        let body_short_atk_cost: u32 = body_short_atk_unit.iter().map(|p| p.cost()).sum();
        let body_long_atk_cost: u32 = body_long_atk_unit.iter().map(|p| p.cost()).sum();

        let body_cost_vec = vec![body_cost, body_short_atk_cost, body_long_atk_cost];
        let _min_cost = body_cost_vec.iter().min().unwrap();

        let mut body = Vec::new();

        let available_energy = sum_energy;
        debug!("spawn calc sum_energy:{:?}", sum_energy);
        let min_basic_body_set = std::cmp::min(
            ((cap_worker_carry as f64 * CAP_WORKER_CARRY_COEFF) / (body_cost as f64)) as u32,
            ((extention_cap as f64) / (body_cost as f64)) as u32,
        );

        info!("min basic body set:{:?}", min_basic_body_set);

        // 「大きい body が組めるまでエネルギーを貯めて待つ」ゲート。
        //
        // 旧実装は外側に恒真の if を被せていた (切り捨てた商を掛け戻すので必ず両辺以下)
        // ため、実質「sum_energy が閾値未満なら生産しない」だけだった。定常状態では
        // cap_worker_carry が 1000 を超えるので 1500 エネルギー貯まるまで生産が止まり、
        // creep が死んで数が減っている最中ほど収入が落ちて閾値に届かなくなる、という
        // 負のスパイラルに入り得た。
        //
        // creep が安全下限を割っているときは待たず、最小構成でもすぐ出す。
        if (num_total_creep >= EMERGENCY_CREEP_FLOOR)
            && (sum_energy < body_cost * min_basic_body_set)
        {
            info!(
                "waiting for {} energy to build a full-size creep",
                body_cost * min_basic_body_set
            );
            continue;
        }

        // とりあえず基本セットをつける.
        if sum_energy >= body_cost {
            body.extend(body_unit.iter().cloned());
            sum_energy -= body_cost;
        } else {
            // 基本セット分だけEnergyがたまってなければまた次回.
            continue;
        }

        // 長距離攻撃がたりなければ装備.
        if opt_num_attackable_long < std::cmp::max(1, num_total_creep / 3) {
            if sum_energy >= body_long_atk_cost {
                let mut count = 0;

                loop {
                    count += 1;
                    // 次に足すユニットを先に決めてから、そのサイズで上限を検査する。
                    let next_is_basic = count % 3 == 0;
                    let (next_len, next_cost) = if next_is_basic {
                        (body_unit.len(), body_cost)
                    } else {
                        (body_long_atk_unit.len(), body_long_atk_cost)
                    };
                    if sum_energy < next_cost
                        || body.len() + next_len > screeps::constants::MAX_CREEP_SIZE as usize
                    {
                        break;
                    }
                    if next_is_basic {
                        body.extend(body_unit.iter().cloned());
                    } else {
                        body.extend(body_long_atk_unit.iter().cloned());
                    }
                    sum_energy -= next_cost;
                }
            } else {
                if ((opt_num_attackable_long + opt_num_attackable_short) < (num_total_creep / 3))
                    && (extention_cap > (body_long_atk_cost + body_cost))
                {
                    continue;
                }
            }

        // 短距離攻撃が足りなければ装備.
        } else if opt_num_attackable_short < std::cmp::max(1, num_total_creep / 3) {
            if sum_energy >= body_short_atk_cost {
                let mut count = 0;

                loop {
                    count += 1;
                    // 次に足すユニットを先に決めてから、そのサイズで上限を検査する。
                    let next_is_basic = count % 3 == 0;
                    let (next_len, next_cost) = if next_is_basic {
                        (body_unit.len(), body_cost)
                    } else {
                        (body_short_atk_unit.len(), body_short_atk_cost)
                    };
                    if sum_energy < next_cost
                        || body.len() + next_len > screeps::constants::MAX_CREEP_SIZE as usize
                    {
                        break;
                    }

                    if next_is_basic {
                        body.extend(body_unit.iter().cloned());
                    } else {
                        body.extend(body_short_atk_unit.iter().cloned());
                    }
                    sum_energy -= next_cost;
                }
            } else {
                if ((opt_num_attackable_long + opt_num_attackable_short) < (num_total_creep / 3))
                    && (extention_cap > (body_short_atk_cost + body_cost))
                {
                    continue;
                }
            }
        }

        // あとは可能な限り基本セット.
        let mut set_num = sum_energy / body_cost;

        while (set_num > 0)
            && ((body.len() + body_unit.len()) <= screeps::constants::MAX_CREEP_SIZE as usize)
        {
            body.extend(body_unit.iter().cloned());
            set_num -= 1;
        }

        // 最も不足しているロールを先に決め、それに合う body を組む。
        // 専用 body が無いロール (汎用作業者など) は上で組んだ body を使う。
        let role = crate::creeps::most_needed_role(&colony);
        // 専用 body の予算は、今この tick に実際に使えるエネルギー。
        let specialized = build_body(role, available_energy);
        let body = if specialized.is_empty() { body } else { specialized };

        if !body.is_empty() {
            // create a unique name, spawn.
            let name_base = game::time();
            let mut additional = 0;
            let res = loop {
                let name = format!("{}-{}", name_base, additional);
                debug!("try spawn {:?} as {}", body, role);

                // ロールを生成時に書き込む。creep_loop 側の割り当てを待たずに
                // 最初の tick から意図した仕事に就ける。
                let mem = js_sys::Object::new();
                let _ = js_sys::Reflect::set(
                    &mem,
                    &wasm_bindgen::JsValue::from_str("role"),
                    &wasm_bindgen::JsValue::from_str(role),
                );
                let opts = SpawnOptions::new().memory(mem.into());

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
}
