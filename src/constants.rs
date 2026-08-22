pub const REPAIRER_DYING_THRESHOLD: u128 = 3000;
pub const TERMINAL_KEEP_ENERGY: u32 = 500;
pub const MARKET_CUT_VALUE: f64 = 0.5 as f64;
pub const MARKET_MIN_PRICE: f64 = 0.1 as f64;
pub const CAP_WORKER_CARRY_COEFF: f64 = 1.5 as f64;

/// 注文作成時に前払いする手数料の率 (価格 × 数量 の 5%、返金なし)。
/// ゲーム定数 MARKET_FEE と同値。
pub const MARKET_FEE: f64 = 0.05;

/// 1回の売り注文で手数料に充ててよいクレジットの割合。
/// 旧実装は 50% を一度の出品に投じていたので、保守的な値に下げる。
pub const MARKET_FEE_BUDGET_RATIO: f64 = 0.1;

/// 1回の買い注文に充ててよいクレジットの割合。
pub const BUY_CREDIT_BUDGET_RATIO: f64 = 0.7;
