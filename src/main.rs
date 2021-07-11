use std::collections::HashSet;

use log::*;

use stdweb::js;

//mod attack;
//mod defence;
mod create;
mod creeps;
mod defence;
mod manage;
mod util;

mod logging;

fn main() {
    logging::setup_logging(logging::Info);

    js! {
        var game_loop = @{game_loop};

        module.exports.loop = function() {
            // Provide actual error traces.
            try {
                game_loop();
            } catch (error) {
                // console_error function provided by 'screeps-game-api'
                console_error("caught exception:", error);
                if (error.stack) {
                    console_error("stack trace:", error.stack);
                }
                console_error("resetting VM next tick.");
                // reset the VM since we don't know if everything was cleaned up and don't
                // want an inconsistent state.
                module.exports.loop = wasm_initialize;
            }
        }
    }
}

fn game_loop() {
    info!(
        "loop starting! CPU: {}, Bucket:{}",
        screeps::game::cpu::get_used(),
        screeps::game::cpu::bucket()
    );

    util::clear_init_flag();

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
        cleanup_memory().expect("expected Memory.creeps format to be a regular memory object");
    }

    info!("done! cpu: {}", screeps::game::cpu::get_used())
}

fn cleanup_memory() -> Result<(), Box<dyn std::error::Error>> {
    let alive_creeps: HashSet<String> = screeps::game::creeps::keys().into_iter().collect();

    let screeps_memory = match screeps::memory::root().dict("creeps")? {
        Some(v) => v,
        None => {
            warn!("not cleaning game creep memory: no Memory.creeps dict");
            return Ok(());
        }
    };

    for mem_name in screeps_memory.keys() {
        if !alive_creeps.contains(&mem_name) {
            debug!("cleaning up creep memory of dead creep {}", mem_name);
            screeps_memory.del(&mem_name);
        }
    }

    Ok(())
}
