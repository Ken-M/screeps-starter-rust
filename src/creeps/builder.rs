use crate::util::*;
use log::*;
use screeps::prelude::*;
use screeps::{find, Creep};

use crate::creeps::repairer::*;

pub fn run_builder(creep: &Creep) {
    let name = creep.name();
    info!("running builder {}", creep.name());

    // 建設現場が1つも無いなら、探索する前に次のロールへ委譲する。
    // 旧実装は find_nearest_construction_site (PathFinder探索) を呼んで
    // 「無い」と分かってから委譲していた。
    if !work_summary().has_construction {
        run_repairer(creep);
        return;
    }

    debug!("check construction sites {}", name);
    let construction_sites = &creep
        .room()
        .expect("room is not visible to you")
        .find(find::MY_CONSTRUCTION_SITES, None);

    let room_name = creep.room().expect("room is not visible to you").name();

    let stats = get_construction_progress_average(&room_name);
    let threshold = (stats.0 + stats.1) / 2;

    for construction_site in construction_sites.iter() {
        if (construction_site.progress_total() - construction_site.progress())
            <= (threshold + 1) as u32
        {
            let r = creep.build(construction_site);
            if r.is_ok() {
                info!("build to my_construction_sites!!");
                return;
            }
        }
    }

    let res = find_nearest_construction_site(&creep, (threshold + 1) as u32);
    debug!("go to:{:?}", res.path());

    if res.path().len() > 0 {
        let res = move_by_search_result(&creep, &res);
        if let Err(e) = res {
            info!("couldn't move to build: {:?}", e);
        }

        return;
    }

    // if nothing to do, act like repairer.
    run_repairer(creep);
}
