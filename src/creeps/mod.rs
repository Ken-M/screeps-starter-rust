mod builder;
mod harvester;
mod repairer;
mod upgrader;

use crate::constants::*;
use crate::mem::{self, MemoryExt};
use crate::util::*;
use log::*;
use screeps::action_error_codes::{CreepMoveByPathErrorCode, HarvestErrorCode};
use screeps::enums::StructureObject;
use screeps::local::Position;
use screeps::pathfinder::SearchResults;
use screeps::prelude::*;
use screeps::{find, game, look, Creep, Part};

#[derive(PartialEq, Debug)]
enum AttackerKind {
    SHORT,
    RANGED,
    NONE,
}

/// 採取先が1つも見つからなかったとき、次に再探索するまで待つ tick 数。
const HARVEST_RETRY_BACKOFF: u32 = 10;

/// ターゲットの有効期限を経路長から決める。
///
/// 旧実装は 5〜20 の固定値だった。カウントダウンは移動中に毎tick減るが、
/// 基本 body は MOVE 比 1/2 なので平地1マスに2tick かかる。storage 用の初期値 10 だと
/// 5マス進むごとにフル再探索を払う計算になり、部屋の対角 (最大約70マス) には
/// いつまでも到達できなかった。
/// 経路長 × 2 に余裕を足した値にする。
fn path_ttl(res: &SearchResults) -> i32 {
    let steps = res.path().len() as i32;
    (steps * 2 + 5).clamp(5, 200)
}

fn reset_source_target(
    creep: &Creep,
    is_harvester: bool,
    harvest_kind: &ResourceKind,
) -> (SearchResults, Position) {
    debug!("harvesting : reset_source_target");

    if is_harvester == true {
        // active sourceをチェック.
        let res = find_nearest_active_source(&creep, harvest_kind, false);
        debug!(
            "harvesting : find_nearest_active_source result:{:?}",
            res.path()
        );

        if res.path().len() > 0 && res.incomplete() == false {
            let last_pos = *(res.path().last().unwrap());
            let json_str = serde_json::to_string(&last_pos).unwrap();
            creep.memory().set("target_pos", json_str);
            creep.memory().set("target_pos_count", path_ttl(&res));
            creep.memory().set("will_harvest_from_storage", false);
            creep.memory().del("nothing_to_harvest");

            debug!(
                "harvesting : target_pos:{:?}",
                creep.memory().string("target_pos")
            );

            return (res, last_pos);
        }

        // storageをチェック.
        if *harvest_kind == ResourceKind::ENERGY {
            let res = find_nearest_stored_source(&creep, harvest_kind, true);

            if res.path().len() > 0 && res.incomplete() == false {
                let last_pos = *(res.path().last().unwrap());
                let json_str = serde_json::to_string(&last_pos).unwrap();
                creep.memory().set("target_pos", json_str);
                creep.memory().set("target_pos_count", path_ttl(&res));
                creep.memory().set("will_harvest_from_storage", true);
                creep.memory().del("nothing_to_harvest");

                debug!(
                    "harvesting : target_pos:{:?}",
                    creep.memory().string("target_pos")
                );

                return (res, last_pos);
            }
        }
    } else {
        // storageをチェック.
        let res = find_nearest_stored_source(&creep, harvest_kind, false);

        if res.path().len() > 0 && res.incomplete() == false {
            let last_pos = *(res.path().last().unwrap());
            let json_str = serde_json::to_string(&last_pos).unwrap();
            creep.memory().set("target_pos", json_str);
            creep.memory().set("target_pos_count", path_ttl(&res));
            creep.memory().set("will_harvest_from_storage", true);
            creep.memory().del("nothing_to_harvest");

            debug!(
                "harvesting : target_pos:{:?}",
                creep.memory().string("target_pos")
            );

            return (res, last_pos);
        }

        // active sourceをチェック.
        let res = find_nearest_active_source(&creep, harvest_kind, true);
        debug!(
            "harvesting : find_nearest_active_source result:{:?}",
            res.path()
        );

        if res.path().len() > 0 && res.incomplete() == false {
            let last_pos = *(res.path().last().unwrap());
            let json_str = serde_json::to_string(&last_pos).unwrap();
            creep.memory().set("target_pos", json_str);
            creep.memory().set("target_pos_count", path_ttl(&res));
            creep.memory().set("will_harvest_from_storage", false);
            creep.memory().del("nothing_to_harvest");

            debug!(
                "harvesting : target_pos:{:?}",
                creep.memory().string("target_pos")
            );

            return (res, last_pos);
        }
    }

    //　やむなく枯渇sourceを選ぶ.
    let res = find_nearest_exhausted_source(&creep, harvest_kind);

    if res.path().len() > 0 {
        let last_pos = *(res.path().last().unwrap());
        let json_str = serde_json::to_string(&last_pos).unwrap();
        creep.memory().set("target_pos", json_str);
        creep.memory().set("target_pos_count", path_ttl(&res));
        creep.memory().set("will_harvest_from_storage", true);
        creep.memory().del("nothing_to_harvest");

        debug!(
            "harvesting : target_pos:{:?}",
            creep.memory().string("target_pos")
        );

        return (res, last_pos);
    }

    // 全部ダメならその場待機。
    //
    // 旧実装はここで「自分の現在地への経路探索」という無意味なコストを払い、
    // 次tickも同じフル再探索 (全室スキャン6回 + 経路探索3回) を無限に繰り返して
    // いた。採取先が無い状況は数tickで変わらないので、しばらく探索を止める。
    creep.memory().set("nothing_to_harvest", true);
    creep.memory().set(
        "harvest_retry_at",
        (game::time() + HARVEST_RETRY_BACKOFF) as i32,
    );
    return (empty_search_for(&creep), creep.pos());
}

/// 攻撃パーツを持つ creep の戦闘処理。
///
/// 戻り値は「この tick の移動を消費したか」。Screeps は 1 tick に attack と transfer と
/// move を同時に発行できるので、攻撃しただけなら経済の仕事も続けてよい。
///
/// 旧実装は攻撃が成功した時点で true を返し、呼び出し側が `continue` で creep の仕事を
/// 丸ごと飛ばしていた。spawn は creep 総数の 1/3 に攻撃パーツを配るので、偵察目的の敵が
/// 1体入ってくるだけで艦隊の大半が採取も建設も止めていた。
/// さらに採取ステートマシンが使う `target_pos` に敵座標を、`harvesting` に true を
/// 書き込んで状態を壊していたため、敵が消えた次の tick に「敵がいた座標を採取ターゲット
/// として扱う」無駄な往復が起きていた。攻撃用のキーは分離する。
fn attacker_routine(creep: &Creep, kind: &AttackerKind) -> bool {
    debug!("check enemies {}", creep.name());
    let room = creep.room().expect("room is not visible to you");
    let enemies = room_hostiles(&room);

    if enemies.is_empty() {
        creep.memory().del("attack_target_pos");
        return false;
    }

    // 射程内の敵がいれば撃つ。移動は消費しない。
    let mut attacked = false;
    for enemy in enemies.iter() {
        let r = match kind {
            AttackerKind::SHORT => creep.attack(enemy).is_ok(),
            AttackerKind::RANGED => creep.ranged_attack(enemy).is_ok(),
            AttackerKind::NONE => false,
        };

        if r {
            info!("attack to enemy!!");
            attacked = true;
            break;
        }
    }

    if attacked {
        // 撃てたなら近づく必要はない。移動は経済側に譲る。
        return false;
    }

    // 射程外なので接近する。ここで初めて移動を消費する。
    // RANGED の射程は 3。旧実装は 2 を指定しており、わざわざ近接圏まで
    // 踏み込みに行っていた。
    let range: u32 = match kind {
        AttackerKind::SHORT => 1,
        AttackerKind::RANGED => 3,
        AttackerKind::NONE => 1,
    };

    let res = find_nearest_enemy(&creep, range);
    debug!("go to:{:?}", res.path());

    if res.path().len() > 0 {
        let last_pos = *(res.path().last().unwrap());
        let json_str = serde_json::to_string(&last_pos).unwrap();
        // 採取用の target_pos とは別のキーに書く。
        creep.memory().set("attack_target_pos", json_str);

        let move_result = move_by_search_result(&creep, &res);
        if move_result.is_ok() {
            info!("move to enemy: {:?}", move_result);
            return true;
        }
    }

    return false;
}

fn get_role_and_attacker_kind(creep: &Creep) -> (String, AttackerKind) {
    let mut attacker_kind: AttackerKind = AttackerKind::NONE;
    let role = creep.memory().string("role");
    let mut role_string = String::from("none");

    // attacker kind check.
    let body_list = creep.body();
    for body_part in body_list {
        if body_part.part() == Part::Attack {
            attacker_kind = AttackerKind::SHORT;
            break;
        } else if body_part.part() == Part::RangedAttack {
            attacker_kind = AttackerKind::RANGED;
            break;
        }
    }

    if let Ok(object) = role {
        if let Some(object) = object {
            role_string = object;
        } else {
            role_string = String::from("none");
        }
    }

    return (role_string, attacker_kind);
}

pub fn creep_loop() {
    let mut num_builder: i32 = 0;
    let mut num_harvester: i32 = 0;
    let mut num_upgrader: i32 = 0;
    let mut num_harvester_spawn: i32 = 0;
    let mut num_harvester_mineral: i32 = 0;
    let mut num_carrier_mineral: i32 = 0;
    let mut num_repairer: i32 = 0;

    let mut opt_num_attackable_short: i32 = 0;
    let mut opt_num_attackable_long: i32 = 0;

    let mut cap_worker_carry: u128 = 0;

    // creep 総数は tick 内で不変なので 1 回だけ数える (JS 境界越えの節約).
    let total_creeps = game::creeps().values().count();

    for creep in game::creeps().values() {
        let name = creep.name();
        debug!("checking creep {}", name);

        let role_and_attacker_kind = get_role_and_attacker_kind(&creep);

        let role_string = role_and_attacker_kind.0;
        let attacker_kind = role_and_attacker_kind.1;

        debug!("role:{:?}:atk:{:?}", role_string, attacker_kind);

        match attacker_kind {
            AttackerKind::SHORT => {
                opt_num_attackable_short += 1;
            }

            AttackerKind::RANGED => {
                opt_num_attackable_long += 1;
            }

            AttackerKind::NONE => {
                //nothing.
            }
        }

        match role_string.as_str() {
            "harvester" => {
                num_harvester += 1;
                cap_worker_carry += creep.store().get_capacity(None) as u128;
            }

            "harvester_spawn" => {
                num_harvester_spawn += 1;
                cap_worker_carry += creep.store().get_capacity(None) as u128;
            }

            "harvester_mineral" => {
                num_harvester_mineral += 1;
            }

            "carrier_mineral" => {
                num_carrier_mineral += 1;
            }

            "builder" => {
                num_builder += 1;
            }

            "upgrader" => {
                num_upgrader += 1;
            }

            "repairer" => {
                num_repairer += 1;
            }

            "none" => {
                // do nothing.
            }

            &_ => {
                error!("no role info");
            }
        }
    }

    // 採取係が枯渇したら全ロールを白紙に戻して再編成する。
    //
    // 旧実装はロールを消すだけでカウンタを据え置いていたため、直後の再割り当てが
    // 「creepのロールはゼロなのにカウンタは満室」という状態で走り、upgrader も
    // builder も repairer も充足済みと誤判定してスキップ、cap_worker_carry も
    // 古い大きな値のままで harvester が1体も作られず、残り全員が catch-all の
    // repairer に落ちていた。採取係が枯渇した瞬間に全creepが最も重い委譲チェーンを
    // 走るという、最悪のタイミングで最悪の挙動になっていた。
    // ロールを消すならカウンタも同じ地点まで巻き戻す。
    if (num_harvester + num_harvester_spawn <= 2)
        && (total_creeps > (num_harvester + num_harvester_spawn) as usize)
    {
        warn!("harvesters depleted; resetting all roles");
        for creep in game::creeps().values() {
            creep.memory().del("role");
        }

        num_builder = 0;
        num_harvester = 0;
        num_upgrader = 0;
        num_harvester_spawn = 0;
        num_harvester_mineral = 0;
        num_carrier_mineral = 0;
        num_repairer = 0;
        cap_worker_carry = 0;
    }

    for creep in game::creeps().values() {
        let name = creep.name();
        info!("running creep {}, cpu:{}", name, game::cpu::get_used());

        let role_and_attacker_kind = get_role_and_attacker_kind(&creep);
        let mut harvest_kind: ResourceKind = ResourceKind::ENERGY;

        let mut is_harvester = false;

        let mut role_string = role_and_attacker_kind.0;
        let attacker_kind = role_and_attacker_kind.1;

        match role_string.as_str() {
            "none" => {
                if num_harvester_spawn < 3 {
                    creep.memory().set("role", "harvester_spawn");
                    num_harvester_spawn += 1;
                    role_string = String::from("harvester_spawn");
                    cap_worker_carry += creep.store().get_capacity(None) as u128;
                } else if num_upgrader < (total_creeps as i32 / 10) + 1 {
                    creep.memory().set("role", "upgrader");
                    num_upgrader += 1;
                    role_string = String::from("upgrader");
                } else if num_builder < (total_creeps as i32 / 6) {
                    creep.memory().set("role", "builder");
                    num_builder += 1;
                    role_string = String::from("builder");
                } else if num_repairer < (total_creeps as i32 / 6) {
                    creep.memory().set("role", "repairer");
                    num_repairer += 1;
                    role_string = String::from("repairer");
                } else if (num_harvester_mineral <= 0) && (total_creeps as i32 > 13) {
                    creep.memory().set("role", "harvester_mineral");
                    num_harvester_mineral += 1;
                    harvest_kind = ResourceKind::MINELALS;
                    role_string = String::from("harvester_mineral");
                    is_harvester = true;
                } else if cap_worker_carry < 1000 {
                    creep.memory().set("role", "harvester");
                    num_harvester += 1;
                    role_string = String::from("harvester");
                    is_harvester = true;
                    cap_worker_carry += creep.store().get_capacity(None) as u128;
                } else if let Some(_my_terminal) = creep.room().expect("I can't see").terminal() {
                    if num_carrier_mineral <= 0 {
                        creep.memory().set("role", "carrier_mineral");
                        num_carrier_mineral += 1;
                        harvest_kind = ResourceKind::MINELALS;
                        role_string = String::from("carrier_mineral");
                        is_harvester = false;
                    } else {
                        creep.memory().set("role", "repairer");
                        num_repairer += 1;
                        role_string = String::from("repairer");
                    }
                } else {
                    creep.memory().set("role", "repairer");
                    num_repairer += 1;
                    role_string = String::from("repairer");
                }
            }

            "harvester" => {
                is_harvester = true;
            }

            "harvester_mineral" => {
                is_harvester = true;
            }

            &_ => {
                // do nothing.
            }
        }

        info!("role:{:?}:atk:{:?}", role_string, attacker_kind);

        if creep.spawning() {
            continue;
        }

        //// atacker check.
        // 戻り値は「移動を消費したか」。攻撃しただけなら経済の仕事も続ける。
        if attacker_kind != AttackerKind::NONE {
            let moved_to_enemy = attacker_routine(&creep, &attacker_kind);

            if moved_to_enemy {
                continue;
            }
        }

        //// harvest resrouce kind.
        if role_string == String::from("harvester_mineral")
            || role_string == String::from("carrier_mineral")
        {
            harvest_kind = ResourceKind::MINELALS;
        }

        // 配達できる荷 (今のロールが扱う資源) の量。
        //
        // 旧実装は採取フェーズへの復帰条件に get_used_capacity(None)、つまり
        // 「全資源が空か」を使っていた。ロール変更でミネラルを抱えたままエネルギー系の
        // ロールになったcreepは、配達先をエネルギーでしか探さないので必ず失敗し、
        // かつ荷が残っているので採取フェーズにも戻れず、寿命が尽きるまで毎tick
        // 全室スキャンを回し続けるデッドロックに陥っていた。
        // 判定を「今のロールで配達できる荷があるか」に変える。
        let store = creep.store();
        let deliverable: u32 = make_resoucetype_list(&harvest_kind)
            .iter()
            .map(|rt| store.get_used_capacity(Some(*rt)))
            .sum();
        let total_used = store.get_used_capacity(None);
        let free_capacity = store.get_free_capacity(None);

        if creep.memory().bool("harvesting") {
            if (free_capacity == 0)
                || ((creep.memory().bool("nothing_to_harvest")) && (deliverable > 0))
            {
                creep.memory().set("harvesting", false);
                creep.memory().del("target_pos");
                creep.memory().del("will_harvest_from_storage");
                creep.memory().del("nothing_to_harvest");
            }
        } else {
            if deliverable == 0 {
                // 配達できる荷が無いなら採取に戻る。ただし配達できない荷で満杯だと
                // 採取もできないので、その場合だけ捨てて詰まりを解消する。
                if total_used > 0 && free_capacity == 0 {
                    for resource in store.store_types() {
                        if !check_resouce_type_kind_matching(&resource, &harvest_kind) {
                            warn!(
                                "{} is stuck with undeliverable {:?}; dropping",
                                name, resource
                            );
                            let _ = creep.drop(resource, None);
                            break;
                        }
                    }
                }

                creep.memory().set("harvesting", true);
                creep.memory().del("target_pos");
                creep.memory().del("harvested_from_storage");
                creep.memory().del("harvested_from_terminal");
                creep.memory().del("harvested_from_link");
                creep.memory().del("nothing_to_harvest");
            }
        }

        // 採取先が無いと分かった直後は、再探索を数tick止める。
        // 探索はフルで走ると全室スキャン6回 + 経路探索3回に達するため、
        // 状況が変わらないうちに毎tick繰り返すのは純粋な浪費になる。
        if creep.memory().bool("harvesting") && creep.memory().bool("nothing_to_harvest") {
            let retry_at = creep
                .memory()
                .i32("harvest_retry_at")
                .unwrap_or(Some(0))
                .unwrap_or(0);
            if (game::time() as i32) < retry_at {
                debug!("{} waiting for harvest retry at {}", name, retry_at);
                continue;
            }
            creep.memory().del("harvest_retry_at");
        }

        if creep.memory().bool("harvesting") {
            debug!("harvesting {}", name);

            let check_string = creep.memory().string("target_pos");
            debug!("harvesting string{:?}", check_string);

            let mut defined_target_pos = creep.pos();
            let mut path_search_result;

            match check_string {
                Ok(v) => {
                    match v {
                        Some(v) => {
                            let defined_target_obj: Result<Position, serde_json::Error> =
                                serde_json::from_str(v.as_str());

                            match defined_target_obj {
                                Ok(object) => {
                                    defined_target_pos = object;
                                    debug!("harvesting decided:{}", defined_target_pos);
                                    path_search_result = find_path(&creep, &defined_target_pos, 0);
                                    debug!(
                                        "harvesting decided path:{:?}",
                                        path_search_result.path()
                                    );

                                    // ターゲット座標のある部屋を見る。旧実装は creep が
                                    // 今いる部屋を見ていたため、他室のターゲットに対して
                                    // 自室の同じ座標を調べてしまい、誤検知 (自室にたまたま
                                    // creep がいると「取られた」と誤判定) と検知漏れ
                                    // (他室の本当のターゲット上の creep を見逃す) の
                                    // 両方が起きていた。
                                    let look_result = game::rooms()
                                        .get(defined_target_pos.room_name())
                                        .map(|target_room| {
                                            target_room.look_for_at_xy(
                                                look::CREEPS,
                                                defined_target_pos.x().u8(),
                                                defined_target_pos.y().u8(),
                                            )
                                        })
                                        .unwrap_or_default();

                                    for one_result in look_result {
                                        if one_result.name() != creep.name() {
                                            debug!("re-check source :{}", defined_target_pos);
                                            creep.memory().del("target_pos");

                                            let reset_result = reset_source_target(
                                                &creep,
                                                is_harvester,
                                                &harvest_kind,
                                            );
                                            path_search_result = reset_result.0;
                                            defined_target_pos = reset_result.1;

                                            break;
                                        }
                                    }
                                }

                                Err(_err) => {
                                    //ロードに成功して値もあったけどDeSerializeできなかった.
                                    let reset_result =
                                        reset_source_target(&creep, is_harvester, &harvest_kind);
                                    path_search_result = reset_result.0;
                                    defined_target_pos = reset_result.1;
                                }
                            }
                        }

                        None => {
                            //ロードに成功したけど値がない.
                            let reset_result =
                                reset_source_target(&creep, is_harvester, &harvest_kind);
                            path_search_result = reset_result.0;
                            defined_target_pos = reset_result.1;
                        }
                    }
                }

                //ロードに失敗(key自体がない).
                Err(_err) => {
                    let reset_result = reset_source_target(&creep, is_harvester, &harvest_kind);
                    path_search_result = reset_result.0;
                    defined_target_pos = reset_result.1;
                }
            }

            let mut is_harvested = false;
            let resource_type_list = make_resoucetype_list(&harvest_kind);

            // check dropped source.
            let resources = &creep
                .room()
                .expect("room is not visible to you")
                .find(find::DROPPED_RESOURCES, None);

            for resource in resources.iter() {
                if creep.pos().is_near_to(resource.pos())
                    && check_resouce_type_kind_matching(&resource.resource_type(), &harvest_kind)
                {
                    if let Err(r) = creep.pickup(resource) {
                        warn!("couldn't pick-up dropped resrouces: {:?}", r);
                        continue;
                    }
                    is_harvested = true;
                    break;
                }
            }

            // check ruins.
            if is_harvested == false {
                let ruins = &creep
                    .room()
                    .expect("room is not visible to you")
                    .find(find::RUINS, None);

                for ruin in ruins.iter() {
                    if creep.pos().is_near_to(ruin.pos()) {
                        for resource_type in resource_type_list.iter() {
                            if ruin.store().get_used_capacity(Some(*resource_type)) > 0 {
                                if let Err(r) = creep.withdraw(ruin, *resource_type, None) {
                                    warn!("couldn't withdraw from RUINs: {:?}", r);
                                    break;
                                }
                                is_harvested = true;
                                break;
                            }
                        }
                    }

                    if is_harvested == true {
                        break;
                    }
                }
            }

            // check tombstones.
            if is_harvested == false {
                let tombstones = &creep
                    .room()
                    .expect("room is not visible to you")
                    .find(find::TOMBSTONES, None);

                for tombstone in tombstones.iter() {
                    if creep.pos().is_near_to(tombstone.pos()) {
                        for resource_type in resource_type_list.iter() {
                            if tombstone.store().get_used_capacity(Some(*resource_type)) > 0 {
                                if let Err(r) = creep.withdraw(tombstone, *resource_type, None) {
                                    warn!("couldn't withdraw from TOMBSTONES: {:?}", r);
                                    break;
                                }
                                is_harvested = true;
                                break;
                            }
                        }
                    }

                    if is_harvested == true {
                        break;
                    }
                }
            }

            //  check sources active.
            if is_harvested == false && harvest_kind == ResourceKind::ENERGY {
                let sources = &creep
                    .room()
                    .expect("room is not visible to you")
                    .find(find::SOURCES_ACTIVE, None);

                for source in sources.iter() {
                    if creep.pos().is_near_to(source.pos()) {
                        if let Err(r) = creep.harvest(source) {
                            warn!("couldn't harvest from ActiveSource: {:?}", r);
                            continue;
                        }
                        is_harvested = true;
                        break;
                    }
                }
            }

            if is_harvested == false && harvest_kind == ResourceKind::MINELALS {
                let sources = &creep
                    .room()
                    .expect("room is not visible to you")
                    .find(find::MINERALS, None);

                for source in sources.iter() {
                    if creep.pos().is_near_to(source.pos()) {
                        match creep.harvest(source) {
                            Ok(()) | Err(HarvestErrorCode::Tired) => {
                                // Tired は採掘クールダウン中なのでその場維持 (旧実装と同じ扱い).
                            }
                            Err(r) => {
                                info!("couldn't harvest from Minerals: {:?}", r);
                                continue;
                            }
                        }
                        is_harvested = true;
                        break;
                    }
                }
            }

            //  storage.
            if is_harvested == false && creep.memory().bool("will_harvest_from_storage") == true {
                let structures = &creep
                    .room()
                    .expect("room is not visible to you")
                    .find(find::STRUCTURES, None);

                for structure in structures.iter() {
                    if creep.pos().is_near_to(structure.pos()) {
                        for resource_type in resource_type_list.iter() {
                            if check_stored(structure, &resource_type, 0) {
                                match structure {
                                    StructureObject::StructureContainer(container) => {
                                        if let Err(r) =
                                            creep.withdraw(container, *resource_type, None)
                                        {
                                            warn!("couldn't withdraw from container: {:?}", r);
                                            break;
                                        }
                                        creep.memory().set("harvested_from_storage", true);
                                        is_harvested = true;
                                        break;
                                    }

                                    StructureObject::StructureStorage(storage) => {
                                        if let Err(r) =
                                            creep.withdraw(storage, *resource_type, None)
                                        {
                                            warn!("couldn't withdraw from storage: {:?}", r);
                                            break;
                                        }
                                        creep.memory().set("harvested_from_storage", true);
                                        is_harvested = true;
                                        break;
                                    }

                                    StructureObject::StructureTerminal(terminal) => {
                                        if harvest_kind == ResourceKind::ENERGY {
                                            if terminal
                                                .store()
                                                .get_used_capacity(Some(*resource_type))
                                                > TERMINAL_KEEP_ENERGY
                                            {
                                                if let Err(r) = creep.withdraw(
                                                    terminal,
                                                    *resource_type,
                                                    Some(std::cmp::min(
                                                        terminal
                                                            .store()
                                                            .get_used_capacity(Some(
                                                                *resource_type,
                                                            ))
                                                            - TERMINAL_KEEP_ENERGY,
                                                        creep.store().get_free_capacity(None)
                                                            as u32,
                                                    )),
                                                ) {
                                                    warn!(
                                                        "couldn't withdraw from terminal: {:?}",
                                                        r
                                                    );
                                                    break;
                                                }
                                                creep.memory().set("harvested_from_terminal", true);
                                                is_harvested = true;
                                                break;
                                            }
                                        }
                                    }

                                    StructureObject::StructureLink(link) => {
                                        if let Err(r) = creep.withdraw(link, *resource_type, None) {
                                            warn!("couldn't withdraw from link: {:?}", r);
                                            break;
                                        }
                                        creep.memory().set("harvested_from_link", true);
                                        is_harvested = true;
                                        break;
                                    }

                                    StructureObject::StructureLab(lab) => {
                                        if harvest_kind == ResourceKind::MINELALS {
                                            if let Err(r) =
                                                creep.withdraw(lab, *resource_type, None)
                                            {
                                                warn!("couldn't withdraw from lab: {:?}", r);
                                                break;
                                            }
                                            creep.memory().set("harvested_from_storage", true);
                                            is_harvested = true;
                                            break;
                                        }
                                    }

                                    _ => {
                                        //do nothing
                                    }
                                }
                            }
                        }

                        if is_harvested == true {
                            break;
                        }
                    }
                }
            }

            if is_harvested == false {
                if creep.pos() == defined_target_pos {
                    debug!("already arrived, but can't harvest!!!");
                    creep.memory().del("target_pos");
                } else {
                    let res = move_by_search_result(&creep, &path_search_result);

                    if let Err(e) = res {
                        info!("couldn't move to source: {:?}", e);
                        // move_by_path は経路が使えない (creep が経路上にいない等) とき
                        // NotFound を返す。旧 move_to の NoPath に相当するので、
                        // 同じくターゲットを捨てて次tickに選び直す。
                        if e == CreepMoveByPathErrorCode::NotFound {
                            creep.memory().del("target_pos");
                        }
                    }
                }

                let mut target_pos_count = creep
                    .memory()
                    .i32("target_pos_count")
                    .unwrap_or(Some(10))
                    .unwrap_or(10);
                target_pos_count -= 1;
                if target_pos_count <= 0 {
                    creep.memory().del("target_pos");
                    creep.memory().del("target_pos_count");
                } else {
                    creep.memory().set("target_pos_count", target_pos_count);
                }
            }
        } else {
            debug!("TASK role:{:?}", role_string);

            let sources = &creep
                .room()
                .expect("room is not visible to you")
                .find(find::SOURCES_ACTIVE, None);

            let mut is_finished = false;

            let flee_count = creep
                .memory()
                .i32("fleeing_count")
                .unwrap_or(Some(0))
                .unwrap_or(0);

            if flee_count <= 0 {
                for source in sources.iter() {
                    if creep.pos().is_near_to(source.pos()) {
                        info!("fleeing from source!!");

                        let result = find_flee_path_from_active_source(&creep);
                        debug!(
                            "fleeing from source!!:{},{},{:?}",
                            result.ops(),
                            result.cost(),
                            result.path()
                        );

                        let res = move_by_search_result(&creep, &result);
                        debug!("fleeing from source!!:{:?}", res);

                        if res.is_ok() {
                            creep.memory().set("fleeing_count", 5);
                            is_finished = true;
                        }

                        break;
                    }
                }
            } else {
                creep.memory().set("fleeing_count", flee_count - 1);
            }

            if is_finished {
                continue;
            }

            match role_string.as_str() {
                "harvester" => {
                    harvester::run_harvester(&creep);
                }

                "harvester_spawn" => {
                    harvester::run_harvester_spawn(&creep);
                }

                "harvester_mineral" => {
                    harvester::run_harvester_mineral(&creep);
                }

                "carrier_mineral" => {
                    harvester::run_carrier_mineral(&creep);
                }

                "builder" => {
                    builder::run_builder(&creep);
                }

                "upgrader" => {
                    upgrader::run_upgrader(&creep);
                }

                "repairer" => {
                    repairer::run_repairer(&creep);
                }

                "attacker" => {}

                "none" => {
                    error!("no role info");
                }

                &_ => {
                    error!("no role info");
                }
            }
        }
    }

    // check number of each type creeps.
    let root = mem::root();
    root.set("num_upgrader", num_upgrader);
    root.set("num_builder", num_builder);
    root.set("num_harvester", num_harvester);
    root.set("num_harvester_spawn", num_harvester_spawn);
    root.set("num_harvester_mineral", num_harvester_mineral);
    root.set("num_carrier_mineral", num_carrier_mineral);
    root.set("num_repairer", num_repairer);

    root.set("opt_num_attackable_short", opt_num_attackable_short);
    root.set("opt_num_attackable_long", opt_num_attackable_long);

    root.set("total_num", total_creeps as i32);
    root.set("cap_worker_carry", cap_worker_carry as i32);
}
