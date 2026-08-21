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

    info!(
        "loop starting! CPU: {}, Bucket:{}",
        screeps::game::cpu::get_used(),
        screeps::game::cpu::bucket()
    );

    util::clear_init_flag();

    info!("running market cpu:{}", screeps::game::cpu::get_used());
    manage::market::run_market();

    info!("running links cpu:{}", screeps::game::cpu::get_used());
    manage::link::run_link();

    info!("running spawns cpu:{}", screeps::game::cpu::get_used());
    create::spawn::do_spawn();

    info!("running creeps cpu:{}", screeps::game::cpu::get_used());
    creeps::creep_loop();

    info!("running towers cpu:{}", screeps::game::cpu::get_used());
    defence::tower::run_tower();

    let time = screeps::game::time();

    if time % 32 == 3 {
        info!("running memory cleanup");
        cleanup_memory();
    }

    info!("done! cpu: {}", screeps::game::cpu::get_used())
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
