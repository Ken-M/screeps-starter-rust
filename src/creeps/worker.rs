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
//! - 平時も worker の約半数をアップグレードに固定する (進捗を止めない)

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
            // worker は消費側なので controller 脇の補給 container からも引く。
            super::hauler::collect_energy(creep, &room, true);
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

    // controller 脇の補給 container がまだ無い部屋では、worker の約半数を
    // アップグレードに回す (RCL3 到達で傾斜が切れて進捗がほぼ止まった実測
    // 1.2 progress/tick の再発防止)。container が立てば専任 upgrader ロール
    // (WORK 全振り body) がこの傾斜を引き継ぐので、worker は建設・修理に戻る。
    // 名前のハッシュで決めるので、指名は creep の生涯を通じて安定する。
    if !super::ColonyState::observe().has_controller_stock && name_parity(creep) {
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
