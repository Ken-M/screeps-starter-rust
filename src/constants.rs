pub const REPAIRER_DYING_THRESHOLD: u128 = 3000;
pub const TERMINAL_KEEP_ENERGY: u32 = 500;
pub const MARKET_MIN_PRICE: f64 = 0.1 as f64;

/// 注文作成時に前払いする手数料の率 (価格 × 数量 の 5%、返金なし)。
/// ゲーム定数 MARKET_FEE と同値。
pub const MARKET_FEE: f64 = 0.05;

/// 1回の売り注文で手数料に充ててよいクレジットの割合。
/// 旧実装は 50% を一度の出品に投じていたので、保守的な値に下げる。
pub const MARKET_FEE_BUDGET_RATIO: f64 = 0.1;

/// 1回の買い注文に充ててよいクレジットの割合。
pub const BUY_CREDIT_BUDGET_RATIO: f64 = 0.7;

/// 値引きの上限 (割合)。散らばりが大きい資源でもこれ以上は引かない。
/// 旧実装の「絶対値 0.5 引き」は安い資源で 80% 引きになっていた。
pub const MARKET_MAX_DISCOUNT_RATIO: f64 = 0.15;

/// 売り価格の下限。中央値に対するこの割合を下回る値付けはしない。
/// 値下げを繰り返しても最終的にここで止まる。
pub const SELL_PRICE_FLOOR_RATIO: f64 = 0.7;

/// Rampart / Wall の修理目標 HP (RCL 連動)。
///
/// この2種は hits_max が桁違い (Rampart 1M〜、Wall 300M) で、hits_max を
/// 目標にすると修理労働が全部吸い込まれて終わらない。実測 (RCL3, 10.7h) では
/// repairer の修理成立 3924 回の 100% が Rampart 行きで、worker の
/// 「建設・修理が無ければ upgrade」の委譲が一度も発火していなかった。
/// 目標に達した Rampart / Wall は check_repairable が対象外にする
/// (減衰で目標を下回れば、また対象に戻る)。
pub fn barrier_target_hp(rcl: u8) -> u32 {
    match rcl {
        0..=2 => 10_000,
        3 => 30_000,
        4 => 100_000,
        5 => 300_000,
        6 => 1_000_000,
        7 => 3_000_000,
        _ => 10_000_000,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 修理目標hpはrclに対して単調増加() {
        for rcl in 0..=7u8 {
            assert!(barrier_target_hp(rcl) <= barrier_target_hp(rcl + 1));
        }
    }

    #[test]
    fn rcl3の修理目標はrampart上限より十分小さい() {
        // RCL3 の Rampart hits_max は 1M。目標が上限に達していると
        // 頭打ちの意味がない。
        assert!(barrier_target_hp(3) < 1_000_000);
    }
}
