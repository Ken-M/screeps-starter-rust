//! 汎用作業者。建設・修理・アップグレードを状況に応じて処理する。
//!
//! 旧実装は builder / repairer / upgrader を別々のロールとして持ち、それぞれが
//! 「自分の仕事が無ければ次のロールとして振る舞う」形で委譲し合っていた。
//! しかし委譲先が互いに繋がっている以上、3つは実質1つの役割である。
//! 1つのロールに統合して、仕事の有無で分岐させる。
//!
//! 統合当初は「建設 > 修理 > アップグレード」の固定優先順にしていたが、
//! 建設サイトが続く限りアップグレードが飢餓する欠陥があった。controller の
//! downgrade タイマーは upgrade しない限り毎tick減り続け、0 になると RCL が
//! 下がって進捗も失われ、セーフモードも使えなくなる。実際にタイマーが
//! 8143/10000 まで減っている状態が観測された。
//!
//! 対策は2段:
//! - タイマーが半分を切ったら、全 worker がアップグレードを最優先する
//! - 平時も worker のうち1体は必ずアップグレードに就く (進捗を止めない)

use crate::util::*;
use log::*;
use screeps::prelude::*;
use screeps::Creep;

/// downgrade タイマーがこの割合を切ったら、建設より先にアップグレードする。
const DOWNGRADE_GUARD_RATIO: f64 = 0.5;

pub fn run_worker(creep: &Creep, force_upgrade: bool) {
    // まずエネルギー。空荷では建設もアップグレードもできない。
    //
    // 旧世代の採取ステートマシンは worker 系のエネルギー調達も担っていたが、
    // 退役時にその代替を用意し忘れ、worker が空荷のままタスクを試みて失敗し
    // 立ち尽くす回帰が起きた (実測: 部屋のエネルギー14/450、progress ほぼ停止)。
    // 調達先は container / storage / 地面。spawn と extension は生産用の
    // 備蓄なので絶対に引き出さない。
    if creep
        .store()
        .get_used_capacity(Some(screeps::ResourceType::Energy))
        == 0
    {
        if let Some(room) = creep.room() {
            super::hauler::collect_energy(creep, &room);
        }
        return;
    }

    // 安全弁: downgrade が近いなら何を置いてもアップグレード。
    if controller_needs_rescue(creep) {
        debug!("{} rescuing controller from downgrade", creep.name());
        super::upgrader::run_upgrader(creep);
        return;
    }

    // 平時でも1体はアップグレードに固定する。建設サイトが続く限り誰も
    // アップグレードしない、という飢餓を構造的に防ぐ。
    if force_upgrade {
        super::upgrader::run_upgrader(creep);
        return;
    }

    // RCL 3 への傾斜。タワーが解禁されるまで、このコロニーの安全は
    // 「誰も攻めてこないこと」だけに依存している (セーフモード残ゼロ)。
    // RCL 3 未満の間は worker の約半数をアップグレードへ回す。
    // 名前のハッシュで決めるので、指名は creep の生涯を通じて安定する。
    if rcl_below_3(creep) && name_parity(creep) {
        super::upgrader::run_upgrader(creep);
        return;
    }

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

fn rcl_below_3(creep: &Creep) -> bool {
    creep
        .room()
        .and_then(|r| r.controller())
        .map(|c| c.my() && c.level() < 3)
        .unwrap_or(false)
}

/// creep 名から安定した2値を得る。worker の半数を選ぶのに使う。
fn name_parity(creep: &Creep) -> bool {
    creep.name().bytes().map(|b| b as u32).sum::<u32>() % 2 == 0
}

/// controller の downgrade タイマーが危険域にあるか。
fn controller_needs_rescue(creep: &Creep) -> bool {
    let Some(room) = creep.room() else {
        return false;
    };
    let Some(controller) = room.controller() else {
        return false;
    };
    if !controller.my() {
        return false;
    }

    let Some(ticks_left) = controller.ticks_to_downgrade() else {
        return false;
    };
    let Some(max) = screeps::constants::controller_downgrade(controller.level()) else {
        return false;
    };

    (ticks_left as f64) < (max as f64) * DOWNGRADE_GUARD_RATIO
}
