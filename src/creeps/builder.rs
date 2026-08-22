use crate::util::*;
use log::*;
use screeps::prelude::*;
use screeps::{find, Creep};

/// 建設する。仕事が無いときの委譲は呼び出し側 (worker) が持つ。
pub fn run_builder_task(creep: &Creep) {
    let name = creep.name();
    info!("building {}", creep.name());

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

}
