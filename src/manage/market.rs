use crate::constants::*;
use crate::mem::{self, MemoryExt};
use log::*;

use js_sys::{Object, Reflect};
use screeps::constants::OrderType;
use screeps::game::market;
use screeps::local::{LodashFilter, RoomName};
use screeps::game::market::Order;
use screeps::{game, MarketResourceType, ResourceType};
use wasm_bindgen::JsValue;

/// 0.23 の `market::create_order` は JS オブジェクトを直接受け取る形に変わったため、
/// 旧 API 相当の引数からパラメータオブジェクトを組み立てる。
fn make_order_params(
    order_type: OrderType,
    resource: ResourceType,
    price: f64,
    total_amount: u32,
    room_name: RoomName,
) -> Object {
    let obj = Object::new();
    let _ = Reflect::set(&obj, &"type".into(), &JsValue::from(order_type));
    let _ = Reflect::set(&obj, &"resourceType".into(), &JsValue::from(resource));
    let _ = Reflect::set(&obj, &"price".into(), &JsValue::from_f64(price));
    let _ = Reflect::set(&obj, &"totalAmount".into(), &JsValue::from(total_amount));
    let _ = Reflect::set(
        &obj,
        &"roomName".into(),
        &JsValue::from(room_name.to_string()),
    );
    obj
}

/// 売ってはいけない資源か。
///
/// セーフモードの生成には Ghodium が 1000 必要。旧実装はエネルギー以外を無条件で
/// 全部売っていたため、防衛の弾を売り払ってしまう構図になっていた。
/// Ghodium とその原料 (Zynthium / Keanium とその中間体) は手元に残す。
fn is_protected(resource: ResourceType) -> bool {
    matches!(
        resource,
        ResourceType::Ghodium
            | ResourceType::ZynthiumKeanite
            | ResourceType::UtriumLemergite
            | ResourceType::Zynthium
            | ResourceType::Keanium
    )
}

/// 14日分の履歴から代表価格 (中央値) と散らばりを出す。
///
/// 旧実装は単純平均だった。マーケットの薄い資源では、意図的な安値取引を数日
/// ぶつけられるだけで平均が引き下がり、こちらの出品価格と買い受入閾値が同時に
/// 下がる。コミットログにある「Marketでカモられていた」の根本はここにある。
/// 中央値なら数日ぶんの外れ値では動かない。あわせて出来高がゼロの日は
/// 価格情報として意味がないので捨てる。
fn price_stats(resource: ResourceType) -> Option<(f64, f64)> {
    let mut prices: Vec<f64> = market::get_history(Some(resource))
        .iter()
        .filter(|h| h.volume() > 0)
        .map(|h| h.avg_price())
        .collect();

    if prices.is_empty() {
        return None;
    }

    prices.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = prices[prices.len() / 2];

    // 散らばりは四分位範囲で見る。標準偏差より外れ値に強い。
    let q1 = prices[prices.len() / 4];
    let q3 = prices[prices.len() * 3 / 4];
    let spread = (q3 - q1).max(0.0);

    if !median.is_finite() || median <= 0.0 {
        return None;
    }

    Some((median, spread))
}

/// 転送に必要なエネルギーを踏まえて、送れる最大量を求める。
///
/// 旧実装は `calc_transaction_cost(1, from, to)` で「1単位あたりのコスト」を得て
/// 割っていたが、ゲーム側は `ceil(amount * (1 - exp(-range/30)))` を返すため
/// amount=1 では距離に関係なく常に 1 になり、距離依存性が完全に消えていた。
/// 近距離では本来の 1/6〜1/30 しか取引しない過小取引になり、同室 (コスト0) だと
/// ゼロ除算で無限大に飛ぶ。
///
/// 候補量に対して実際のコストを評価し、予算に収まる最大量を二分探索する。
fn max_affordable_amount(
    upper_bound: u32,
    energy_budget: u32,
    from: RoomName,
    to: RoomName,
) -> u32 {
    if upper_bound == 0 {
        return 0;
    }

    // 同室ならターミナル転送のコストはかからない。
    if from == to {
        return upper_bound;
    }

    let cost_of = |amount: u32| -> u32 {
        market::calc_transaction_cost(amount, &from.into(), &to.into())
    };

    if cost_of(upper_bound) <= energy_budget {
        return upper_bound;
    }

    let (mut lo, mut hi) = (0u32, upper_bound);
    while lo < hi {
        let mid = lo + (hi - lo + 1) / 2;
        if cost_of(mid) <= energy_budget {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    lo
}

/// 自分の注文の保守。値下げと、腐った注文の入れ替え。
fn maintain_orders(market_count: i32) -> (i32, i32) {
    let mut my_sell_orders = 0;
    let mut my_buy_orders = 0;

    for my_order in market::orders().values() {
        debug!("my order:{:?}", my_order.id());

        if my_order.remaining_amount() == 0 {
            // 残量ゼロの注文はゲーム側で消えるので、手数料も戻らない以上
            // わざわざキャンセルを呼ぶ意味はない。数だけ数えない。
            continue;
        }

        if my_order.order_type() == OrderType::Buy {
            my_buy_orders += 1;
            continue;
        }

        my_sell_orders += 1;

        // 値下げは 9000 tick ごと。
        if market_count % 9000 != 0 {
            continue;
        }

        // created() が無い注文は経過時間を判定できない。旧実装は unwrap_or(0) で
        // 「無限に古い」と扱って値下げしていたが、判定不能なら触らない方が安全。
        let Some(created) = my_order.created() else {
            continue;
        };
        if game::time().saturating_sub(created) <= 9000 {
            continue;
        }

        let MarketResourceType::Resource(resource) = my_order.resource_type() else {
            continue;
        };

        // 値下げには下限を設ける。
        //
        // 旧実装は下限なしで 0.95 倍を繰り返していたため、長期稼働では
        // 「時間が経つほど無条件に安く売る」注文になり、市場が高騰しても
        // 安値のまま約定していた。しかも同一資源の注文が1件でもあると新規注文を
        // 作らない条件があるため、一度作った注文が永久に居座って値下げだけが進む。
        let floor = match price_stats(resource) {
            Some((median, _)) => median * SELL_PRICE_FLOOR_RATIO,
            None => MARKET_MIN_PRICE,
        };

        let new_price = my_order.price() * 0.95;

        if new_price >= floor {
            info!(
                "discount order {:?}: {:.3} -> {:.3}",
                resource,
                my_order.price(),
                new_price
            );
            if let Err(e) = market::change_order_price(&my_order.id(), new_price) {
                warn!("couldn't change order price: {:?}", e);
            }
        } else {
            // 下限を割るところまで下げても売れないなら、価格が市況に合っていない。
            // 値下げを続けるのではなく畳んで、次の巡回で適正価格で作り直す。
            info!(
                "cancel stale order {:?} at {:.3} (floor {:.3})",
                resource,
                my_order.price(),
                floor
            );
            if let Err(e) = market::cancel_order(&my_order.id()) {
                warn!("couldn't cancel order: {:?}", e);
            }
            my_sell_orders -= 1;
        }
    }

    (my_sell_orders, my_buy_orders)
}

/// 自分の注文を資源別に数える。
///
/// 旧実装は資源ループの内側で毎回 `market::orders()` を全走査していたため
/// O(資源数 × 自注文数) になっていた。1回だけ集計する。
fn count_my_orders(order_type: OrderType) -> std::collections::HashMap<ResourceType, u32> {
    let mut counts = std::collections::HashMap::new();
    for my_order in market::orders().values() {
        if my_order.order_type() != order_type {
            continue;
        }
        if let MarketResourceType::Resource(r) = my_order.resource_type() {
            *counts.entry(r).or_insert(0) += 1;
        }
    }
    counts
}

/// 指定資源の全注文を取ってくる。
fn orders_for(resource: ResourceType) -> Vec<Order> {
    let filter = LodashFilter::new();
    filter.resource_type(MarketResourceType::Resource(resource));
    market::get_all_orders(Some(&filter))
}

pub fn run_market() {
    info!("running market");

    let root = mem::root();

    let mut market_count = root.i32("market_counter").unwrap_or(Some(0)).unwrap_or(0);
    market_count += 1;
    if market_count >= 18000 {
        market_count = 0;
    }
    root.set("market_counter", market_count);

    // クレジットは全部屋で共有する1つの財布。
    //
    // 旧実装は tick 先頭で1回読んだ値を各部屋のループで使い回していたため、
    // 部屋が3つあれば理論上「クレジットの70%」が3重に発行され得た。
    // 使った分を引いていく予算変数として扱う。
    let mut credit_budget = market::credits();
    info!("current credits:{:?}", credit_budget);

    let (my_sell_orders, my_buy_orders) = maintain_orders(market_count);

    if market_count % 100 == 0 {
        run_sell_side(&mut credit_budget, my_sell_orders);
    } else if market_count % 100 == 50 {
        run_buy_side(&mut credit_budget, my_buy_orders);
    }
}

fn run_sell_side(credit_budget: &mut f64, my_sell_orders: i32) {
    let existing = count_my_orders(OrderType::Sell);

    for room in game::rooms().values() {
        let Some(my_terminal) = room.terminal() else {
            continue;
        };
        if my_terminal.cooldown() > 0 {
            continue;
        }

        let terminal_energy = my_terminal
            .store()
            .get_used_capacity(Some(ResourceType::Energy));

        for resource in my_terminal.store().store_types() {
            // エネルギーは売らない。防衛の原資も売らない。
            if resource == ResourceType::Energy || is_protected(resource) {
                continue;
            }

            let stored_amount = my_terminal.store().get_used_capacity(Some(resource));
            if stored_amount == 0 {
                continue;
            }

            // 値付けの根拠が無い資源には手を出さない。
            let Some((median, spread)) = price_stats(resource) else {
                info!("no usable market history for {:?}; skipping", resource);
                continue;
            };

            // 値引きは割合で行う。
            //
            // 旧実装は `avg - min(stddev, 0.5)` という絶対値の引き算だったため、
            // 平均 0.6 の資源は下限 0.1 まで落ちて 83% 引き、平均 30 の資源は
            // 1.7% 引きにしかならず、価格帯に対して全く一貫していなかった。
            let discount = (spread / median).min(MARKET_MAX_DISCOUNT_RATIO);
            let ask_price = (median * (1.0 - discount)).max(median * SELL_PRICE_FLOOR_RATIO);
            // 相手の買い注文を受ける最低ライン。散らばりのぶん上に置く。
            let accept_price = median * (1.0 + discount);

            // 約定は価格の良い順に1件だけ。
            //
            // ターミナルは約定後 10 tick のクールダウンに入るので、1 tick に
            // 成立するのは実質1件。旧実装は返却順で走査して閾値を超えた最初の
            // 注文に売っていたため、もっと高く買う注文があっても取り逃がしていた。
            let mut candidates: Vec<Order> = orders_for(resource)
                .into_iter()
                .filter(|o| o.order_type() == OrderType::Buy && o.price() >= accept_price)
                .collect();

            candidates.sort_by(|a, b| {
                b.price()
                    .partial_cmp(&a.price())
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            for order in candidates.iter() {
                // アカウント資源 (pixel 等) の注文は roomName を持たない。
                // 旧実装は expect() で落としており、market は tick の先頭で走るため
                // creep もタワーも動かないまま tick 全体が失われていた。
                let Some(order_room) = order.room_name() else {
                    continue;
                };
                let Ok(order_room) = RoomName::new(&String::from(order_room)) else {
                    continue;
                };

                let energy_budget = (terminal_energy as f64 * 0.7) as u32;
                let upper = std::cmp::min(stored_amount, order.remaining_amount());
                let amount =
                    max_affordable_amount(upper, energy_budget, room.name(), order_room);

                if amount == 0 {
                    continue;
                }

                info!(
                    "deal {:?}: {} @ {:.3} (median {:.3})",
                    resource,
                    amount,
                    order.price(),
                    median
                );

                match market::deal(&order.id(), amount, Some(room.name())) {
                    Ok(()) => {
                        *credit_budget += amount as f64 * order.price();
                    }
                    Err(e) => warn!("deal failed: {:?}", e),
                }
                // 成否によらずクールダウンに入るので、この部屋はここで終わり。
                break;
            }

            // 売り注文を出す。
            if my_sell_orders >= 40 {
                continue;
            }
            if existing.get(&resource).copied().unwrap_or(0) > 0 {
                continue;
            }

            let amount = stored_amount / 2;
            let fee = ask_price * amount as f64 * MARKET_FEE;
            let fee_budget = *credit_budget * MARKET_FEE_BUDGET_RATIO;

            let amount = if fee > fee_budget {
                let affordable = fee_budget / (ask_price * MARKET_FEE);
                if affordable.is_finite() && affordable >= 1.0 {
                    std::cmp::min(amount, affordable as u32)
                } else {
                    0
                }
            } else {
                amount
            };

            if amount == 0 {
                continue;
            }

            info!(
                "create sell order {:?}: {} @ {:.3}",
                resource, amount, ask_price
            );
            match market::create_order(&make_order_params(
                OrderType::Sell,
                resource,
                ask_price,
                amount,
                room.name(),
            )) {
                Ok(()) => {
                    *credit_budget -= ask_price * amount as f64 * MARKET_FEE;
                }
                Err(e) => warn!("create_order failed: {:?}", e),
            }
        }
    }
}

fn run_buy_side(credit_budget: &mut f64, my_buy_orders: i32) {
    let existing = count_my_orders(OrderType::Buy);

    let Some((median, spread)) = price_stats(ResourceType::Energy) else {
        info!("no usable market history for energy; skipping buy side");
        return;
    };

    let discount = (spread / median).min(MARKET_MAX_DISCOUNT_RATIO);
    // 買い叩ける上限。これ以下でしか買わない。
    let accept_price = median * (1.0 - discount);
    // 自分で出す買い注文の提示価格。
    let bid_price = median * (1.0 + discount);

    for room in game::rooms().values() {
        let Some(my_terminal) = room.terminal() else {
            continue;
        };
        if my_terminal.cooldown() > 0 {
            continue;
        }

        let terminal_energy_capacity = my_terminal
            .store()
            .get_free_capacity(Some(ResourceType::Energy));
        if terminal_energy_capacity <= 0 {
            continue;
        }
        let terminal_energy_capacity = terminal_energy_capacity as u32;

        let terminal_energy = my_terminal
            .store()
            .get_used_capacity(Some(ResourceType::Energy));

        let mut candidates: Vec<Order> = orders_for(ResourceType::Energy)
            .into_iter()
            .filter(|o| o.order_type() == OrderType::Sell && o.price() <= accept_price)
            .collect();

        // 安い順。
        candidates.sort_by(|a, b| {
            a.price()
                .partial_cmp(&b.price())
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        for order in candidates.iter() {
            let Some(order_room) = order.room_name() else {
                continue;
            };
            let Ok(order_room) = RoomName::new(&String::from(order_room)) else {
                continue;
            };

            let price = order.price();
            if price <= 0.0 {
                continue;
            }

            let affordable_by_credits = (*credit_budget * BUY_CREDIT_BUDGET_RATIO) / price;
            if !affordable_by_credits.is_finite() || affordable_by_credits < 1.0 {
                continue;
            }

            let upper = std::cmp::min(
                std::cmp::min(terminal_energy_capacity, order.remaining_amount()),
                affordable_by_credits as u32,
            );
            let energy_budget = (terminal_energy as f64 * 0.7) as u32;
            let amount = max_affordable_amount(upper, energy_budget, room.name(), order_room);

            if amount == 0 {
                continue;
            }

            info!("buy energy: {} @ {:.3} (median {:.3})", amount, price, median);

            match market::deal(&order.id(), amount, Some(room.name())) {
                Ok(()) => {
                    *credit_budget -= amount as f64 * price;
                }
                Err(e) => warn!("deal failed: {:?}", e),
            }
            break;
        }

        // 買い注文を出す。
        if my_buy_orders >= 10 {
            continue;
        }
        if existing.get(&ResourceType::Energy).copied().unwrap_or(0) > 0 {
            continue;
        }

        let amount = terminal_energy_capacity / 2;
        let affordable = (*credit_budget * BUY_CREDIT_BUDGET_RATIO) / (bid_price * 1.05);
        let amount = if affordable.is_finite() && affordable >= 1.0 {
            std::cmp::min(amount, affordable as u32)
        } else {
            0
        };

        if amount == 0 {
            continue;
        }

        info!("create buy order energy: {} @ {:.3}", amount, bid_price);
        match market::create_order(&make_order_params(
            OrderType::Buy,
            ResourceType::Energy,
            bid_price,
            amount,
            room.name(),
        )) {
            Ok(()) => {
                *credit_budget -= bid_price * amount as f64 * MARKET_FEE;
            }
            Err(e) => warn!("create_order failed: {:?}", e),
        }
    }
}
