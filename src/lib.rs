use std::collections::HashSet;
use std::sync::Once;

use js_sys::{JsString, Object, Reflect};
use log::*;
use wasm_bindgen::prelude::*;

mod constants;
mod create;
mod creeps;
mod defence;
mod manage;
mod mem;
mod util;

mod logging;

static INIT_LOGGING: Once = Once::new();

// 例外捕捉と VM リセットは js_src/main.js のローダーが担当する
// (旧 stdweb 版で js! マクロがやっていた処理の置き換え).
#[wasm_bindgen(js_name = loop)]
pub fn game_loop() {
    INIT_LOGGING.call_once(|| {
        logging::setup_logging(logging::Info);
    });

    debug!(
        "loop starting! CPU: {}, Bucket:{}",
        screeps::game::cpu::get_used(),
        screeps::game::cpu::bucket()
    );

    util::clear_init_flag();

    debug!("running market cpu:{}", screeps::game::cpu::get_used());
    manage::market::run_market();

    debug!("running links cpu:{}", screeps::game::cpu::get_used());
    manage::link::run_link();

    debug!("running spawns cpu:{}", screeps::game::cpu::get_used());
    create::spawn::do_spawn();

    debug!("running build planner cpu:{}", screeps::game::cpu::get_used());
    create::build::run_planner();

    debug!("running creeps cpu:{}", screeps::game::cpu::get_used());
    creeps::creep_loop();

    debug!("running towers cpu:{}", screeps::game::cpu::get_used());
    defence::tower::run_tower();

    let time = screeps::game::time();

    if time % 32 == 3 {
        debug!("running memory cleanup");
        cleanup_memory();
    }

    if time.is_multiple_of(SUMMARY_INTERVAL) {
        log_summary();
    }

    debug!("done! cpu: {}", screeps::game::cpu::get_used())
}

/// 定点観測サマリの間隔 (tick)。
const SUMMARY_INTERVAL: u32 = 100;

/// 定点観測サマリ。tick 末尾 (CPU 計上がほぼ済んだ地点) で呼ぶ。
///
/// creep 単位・フェーズ単位の逐次ログは debug に落とした (INFO のままだと
/// 10時間で26MB 出る一方、成長したかどうかはどこにも残らなかった)。
/// INFO の定常出力はこのサマリと、spawn / planner / 戦闘などのイベントだけ。
/// 1行目が部屋ごとの成長と経済、2行目がコロニーの人口と CPU。
fn log_summary() {
    use crate::mem::MemoryExt;
    use screeps::prelude::*;

    for room in screeps::game::rooms().values() {
        let Some(controller) = room.controller() else {
            continue;
        };
        if !controller.my() {
            continue;
        }

        info!(
            "summary {}: rcl:{} progress:{}/{} downgrade:{} energy:{}/{}",
            room.name(),
            controller.level(),
            controller.progress().unwrap_or(0),
            controller.progress_total().unwrap_or(0),
            controller.ticks_to_downgrade().unwrap_or(0),
            room.energy_available(),
            room.energy_capacity_available(),
        );
    }

    // 人口構成。BTreeMap でロール名順に安定させる。
    let mut counts: std::collections::BTreeMap<String, u32> = Default::default();
    for creep in screeps::game::creeps().values() {
        let role = creep
            .memory()
            .string(mem::keys::ROLE)
            .ok()
            .flatten()
            .unwrap_or_else(|| "none".to_string());
        *counts.entry(role).or_insert(0) += 1;
    }
    let roles = counts
        .iter()
        .map(|(role, n)| format!("{}:{}", role, n))
        .collect::<Vec<_>>()
        .join(" ");

    let colony = creeps::ColonyState::observe();
    info!(
        "summary colony: creeps[{}] backlog:{} cpu:{:.2} bucket:{}",
        roles,
        colony.energy_backlog,
        screeps::game::cpu::get_used(),
        screeps::game::cpu::bucket(),
    );
}

fn cleanup_memory() {
    let alive_creeps: HashSet<String> = screeps::game::creeps().keys().collect();

    // `Memory.creeps` (あれば) を取得して、死んだ creep のエントリを削除する.
    if let Ok(memory_creeps) = Reflect::get(&mem::root(), &JsString::from("creeps")) {
        if memory_creeps.is_undefined() || memory_creeps.is_null() {
            warn!("not cleaning game creep memory: no Memory.creeps dict");
            return;
        }

        let memory_creeps: Object = memory_creeps.unchecked_into();
        for creep_name_js in Object::keys(&memory_creeps).iter() {
            let creep_name = String::from(creep_name_js.dyn_ref::<JsString>().unwrap());

            if !alive_creeps.contains(&creep_name) {
                debug!("cleaning up creep memory of dead creep {}", creep_name);
                let _ = Reflect::delete_property(&memory_creeps, &creep_name_js);
            }
        }
    }
}
