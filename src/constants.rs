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

/// 値引きの上限 (割合)。散らばりが大きい資源でもこれ以上は引かない。
/// 旧実装の「絶対値 0.5 引き」は安い資源で 80% 引きになっていた。
pub const MARKET_MAX_DISCOUNT_RATIO: f64 = 0.15;

/// 売り価格の下限。中央値に対するこの割合を下回る値付けはしない。
/// 値下げを繰り返しても最終的にここで止まる。
pub const SELL_PRICE_FLOOR_RATIO: f64 = 0.7;
