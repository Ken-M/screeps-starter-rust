use crate::util::*;
use log::*;

use screeps::action_error_codes::UpgradeControllerErrorCode;
use screeps::prelude::*;
use screeps::Creep;

pub fn run_upgrader(creep: &Creep) {
    let name = creep.name();
    info!("running upgrader {}", creep.name());

    debug!("check controller {}", name);

    if let Some(c) = creep
        .room()
        .expect("room is not visible to you")
        .controller()
    {
        if c.my() == true {
            let r = creep.upgrade_controller(&c);

            match r {
                Err(UpgradeControllerErrorCode::NotInRange) => {
                    let res = find_path(&creep, &c.pos(), 3);

                    if res.path().len() > 0 {
                        let res = move_by_search_result(&creep, &res);
                        if let Err(e) = res {
                            info!("couldn't move to upgrade: {:?}", e);
                        } else {
                            return;
                        }
                    }
                }

                Err(e) => {
                    warn!(
                        "couldn't upgrade: {:?},{:?}",
                        e,
                        creep.store().get_used_capacity(None)
                    );
                }

                Ok(()) => {
                    return;
                }
            }
        }
    }

    let res = find_nearest_room_controler(&creep);
    debug!("go to:{:?}", res.path());

    if res.path().len() > 0 {
        let res = move_by_search_result(&creep, &res);
        if let Err(e) = res {
            info!("couldn't move to build: {:?}", e);
        }

        return;
    }
}
