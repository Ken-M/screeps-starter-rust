//! 汎用作業者。建設 → 修理 → アップグレードを優先順で処理する。
//!
//! 旧実装は builder / repairer / upgrader を別々のロールとして持ち、それぞれが
//! 「自分の仕事が無ければ次のロールとして振る舞う」形で委譲し合っていた。
//! しかし委譲先が互いに繋がっている以上、3つは実質1つの役割である。定常状態
//! (建設現場も修理対象も無い) では3ロールとも結局アップグレードに落ち着く。
//!
//! 分けておく意味があるのは目標体数を別々に決めたい場合だが、実際には
//! 「余剰労働力をどれだけ持つか」が決まればよく、内訳はその時の仕事の有無で
//! 自動的に決まる。1つのロールに統合して、仕事の有無で分岐させる。
//!
//! 委譲チェーンが消えるので、各段が探索で仕事の有無を確かめる必要もなくなる。

use crate::util::*;
use log::*;
use screeps::prelude::*;
use screeps::Creep;

pub fn run_worker(creep: &Creep) {
    let summary = work_summary();

    if summary.has_construction {
        super::builder::run_builder_task(creep);
        return;
    }

    if summary.has_repair_target {
        super::repairer::run_repairer_task(creep);
        return;
    }

    debug!("{} has no build/repair work; upgrading", creep.name());
    super::upgrader::run_upgrader(creep);
}
