use crate::constants::*;
use crate::mem::{self, MemoryExt};
use log::*;

use js_sys::{Object, Reflect};
use screeps::constants::OrderType;
use screeps::game::market;
use screeps::local::{LodashFilter, RoomName};
use screeps::prelude::*;
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

pub fn run_market() {
    info!("running market");

    let root = mem::root();

    let mut market_count = root.i32("market_counter").unwrap_or(Some(0)).unwrap_or(0);
    market_count += 1;
    if market_count >= 18000 {
        market_count = 0;
    }
    root.set("market_counter", market_count);

    let cur_credits = market::credits();
    info!("current credits:{:?}", cur_credits);

    let mut my_sell_orders = 0;
    let mut my_buy_orders = 0;

    for my_order in market::orders().values() {
        debug!("my order:{:?}", my_order.id());

        if my_order.remaining_amount() <= 0 {
            let _ = market::cancel_order(&my_order.id());
        } else {
            if my_order.order_type() == OrderType::Buy {
                my_buy_orders += 1;
            } else {
                my_sell_orders += 1;

                if market_count % 9000 == 0 {
                    if game::time() - my_order.created().unwrap_or(0) > 9000 {
                        let _ = market::change_order_price(
                            &my_order.id(),
                            my_order.price() * 0.95,
                        );
                    }
                }
            }
        }
    }

    if market_count % 100 == 0 {
        for room in game::rooms().values() {
            if let Some(my_terminal) = room.terminal() {
                //sell orders.
                let resource_type_list = my_terminal.store().store_types();
                let terminal_energy = my_terminal
                    .store()
                    .get_used_capacity(Some(ResourceType::Energy));

                if my_terminal.cooldown() > 0 {
                    continue;
                }

                for resource in resource_type_list {
                    // don't sell energy.
                    if resource == ResourceType::Energy {
                        continue;
                    }

                    let mut stored_amount =
                        my_terminal.store().get_used_capacity(Some(resource));
                    if stored_amount <= 0 {
                        continue;
                    }

                    // calc target price.
                    let market_history = market::get_history(Some(resource));

                    let mut target_price: f64 = 0 as f64;
                    let mut target_price_own: f64 = 0 as f64;
                    let mut num_data: u128 = 0;

                    for history in market_history {
                        target_price += history.avg_price() + history.stddev_price();
                        target_price_own += f64::max(
                            history.avg_price()
                                - f64::min(history.stddev_price(), MARKET_CUT_VALUE),
                            MARKET_MIN_PRICE,
                        );
                        num_data += 1;
                    }

                    // 取引履歴が無い資源 (実績ゼロ、サーバリセット直後、取得失敗) では
                    // get_history() が空を返す。このとき target_price は 0.0 のままなので、
                    // 下の `order.price() >= target_price` が全注文で成立し、
                    // 価格0.001のような釣り注文に在庫を全量投げてしまう。
                    // 値付けの根拠が無い以上、この資源は売買しない。
                    if num_data == 0 {
                        info!("no market history for {:?}; skipping", resource);
                        continue;
                    }

                    target_price = target_price / num_data as f64;
                    target_price_own = target_price_own / num_data as f64;

                    // 平均が壊れていても最低ラインは割らない (二重の歯止め)。
                    target_price = f64::max(target_price, MARKET_MIN_PRICE);
                    target_price_own = f64::max(target_price_own, MARKET_MIN_PRICE);

                    // check buy orders.
                    let filter = LodashFilter::new();
                    filter.resource_type(MarketResourceType::Resource(resource));
                    let all_orders = market::get_all_orders(Some(&filter));

                    for order in all_orders {
                        if order.order_type() == OrderType::Buy {
                            if order.price() >= target_price {
                                let amount = (terminal_energy as f64 * 0.7)
                                    / market::calc_transaction_cost(
                                        1,
                                        &room.name().into(),
                                        &order.room_name().expect("not resource order"),
                                    ) as f64;
                                let amount = std::cmp::min(amount as u32, stored_amount);
                                let amount =
                                    std::cmp::min(amount as u32, order.remaining_amount());
                                if amount > 0 {
                                    info!("deal: {:?}, amount:{:?}", order.id(), amount);

                                    let ret =
                                        market::deal(&order.id(), amount, Some(room.name()));

                                    match ret {
                                        Ok(()) => {
                                            stored_amount -= amount;
                                        }
                                        Err(e) => {
                                            warn!("ret:{:?}", e);
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // make sell orders.
                    if my_sell_orders < 40 {
                        let mut found_count = 0;

                        for my_order in market::orders().values() {
                            if my_order.order_type() == OrderType::Sell {
                                if my_order.resource_type()
                                    == MarketResourceType::Resource(resource)
                                {
                                    found_count += 1;
                                }
                            }
                        }

                        if found_count < 1 {
                            // 旧実装は「クレジットの50%を手数料予算として、その予算で
                            // 何個売れるか」を先に計算していた。target_price_own が 0 だと
                            // +inf → as u32 が飽和して u32::MAX、cur_credits が 0 だと
                            // 0.0/0.0 = NaN → as u32 が 0 になり、価格0での大量出品か
                            // 「永久に出品しない」のどちらかに化けていた。
                            //
                            // 数量は在庫から決め、手数料が予算内かを後から検査する順序にする。
                            let amount = stored_amount / 2;
                            let fee = target_price_own * amount as f64 * MARKET_FEE;
                            let fee_budget = cur_credits * MARKET_FEE_BUDGET_RATIO;

                            // 手数料が予算を超えるなら、予算に収まる数量まで削る。
                            let amount = if fee > fee_budget {
                                let affordable =
                                    fee_budget / (target_price_own * MARKET_FEE);
                                if affordable.is_finite() && affordable >= 1.0 {
                                    std::cmp::min(amount, affordable as u32)
                                } else {
                                    0
                                }
                            } else {
                                amount
                            };

                            if amount > 0 {
                                info!(
                                    "create a Sell deal: resource type:{:?}, amount:{:?}, price:{:?}",
                                    resource, amount, target_price_own
                                );
                                let ret = market::create_order(&make_order_params(
                                    OrderType::Sell,
                                    resource,
                                    target_price_own,
                                    amount,
                                    room.name(),
                                ));

                                if let Err(e) = ret {
                                    warn!("ret:{:?}", e);
                                }
                            }
                        }
                    }
                }
            }
        }
    } else if market_count % 100 == 50 {
        for room in game::rooms().values() {
            if let Some(my_terminal) = room.terminal() {
                //buy energy orders.
                let mut terminal_energy_capacity = my_terminal
                    .store()
                    .get_free_capacity(Some(ResourceType::Energy));
                let terminal_energy = my_terminal
                    .store()
                    .get_used_capacity(Some(ResourceType::Energy));

                if my_terminal.cooldown() > 0 {
                    continue;
                }

                if terminal_energy_capacity <= 0 {
                    continue;
                }

                // calc target price.
                let market_history = market::get_history(Some(ResourceType::Energy));

                let mut target_price: f64 = 0 as f64;
                let mut target_price_own: f64 = 0 as f64;
                let mut num_data: u128 = 0;

                for history in market_history {
                    target_price +=
                        f64::max(history.avg_price() - history.stddev_price(), 0 as f64);
                    target_price_own +=
                        history.avg_price() + f64::min(history.stddev_price(), MARKET_CUT_VALUE);
                    num_data += 1;
                }

                // 売り側と同じく、履歴が無ければ値付けの根拠が無いので手を出さない。
                // (買い側の受入条件は `price <= target_price` なので、target_price が
                //  0 のままなら約定はしない。ただし target_price_own が 0 だと
                //  価格0の買い注文を作りにいくため、ここで止める。)
                if num_data == 0 {
                    info!("no market history for energy; skipping buy side");
                    continue;
                }

                target_price = target_price / num_data as f64;
                target_price_own = target_price_own / num_data as f64;

                // check sell orders.
                let filter = LodashFilter::new();
                filter.resource_type(MarketResourceType::Resource(ResourceType::Energy));
                let all_orders = market::get_all_orders(Some(&filter));

                for order in all_orders {
                    if order.order_type() == OrderType::Sell {
                        if order.price() <= target_price {
                            let amount = (terminal_energy as f64 * 0.7)
                                / market::calc_transaction_cost(
                                    1,
                                    &room.name().into(),
                                    &order.room_name().expect("not resource order"),
                                ) as f64;
                            let amount = std::cmp::min(
                                amount as u32,
                                ((cur_credits * 0.7) / (order.price() as f64)) as u32,
                            );
                            let amount =
                                std::cmp::min(amount as u32, terminal_energy_capacity as u32);
                            let amount = std::cmp::min(amount as u32, order.remaining_amount());
                            if amount > 0 {
                                info!("make a deal: {:?}, amount:{:?}", order.id(), amount);

                                let ret = market::deal(&order.id(), amount, Some(room.name()));

                                match ret {
                                    Ok(()) => {
                                        terminal_energy_capacity -= amount as i32;
                                    }
                                    Err(e) => {
                                        warn!("ret:{:?}", e);
                                    }
                                }
                            }
                        }
                    }
                }

                // make buy orders.
                if my_buy_orders < 10 {
                    let mut found_count = 0;

                    for my_order in market::orders().values() {
                        if my_order.order_type() == OrderType::Buy {
                            if my_order.resource_type()
                                == MarketResourceType::Resource(ResourceType::Energy)
                            {
                                found_count += 1;
                            }
                        }
                    }

                    if found_count < 1 {
                        // 買いは「ターミナルの空き容量の半分」を基準にし、
                        // 支払える範囲 (クレジットの70%) まで削る。除算の分母が
                        // 0 や NaN になる経路は上の num_data ガードで潰してあるが、
                        // 念のため有限値だけを採用する。
                        let amount = terminal_energy_capacity.max(0) as u32 / 2;
                        let affordable = (cur_credits * BUY_CREDIT_BUDGET_RATIO)
                            / (target_price_own * 1.05);
                        let amount = if affordable.is_finite() && affordable >= 1.0 {
                            std::cmp::min(amount, affordable as u32)
                        } else {
                            0
                        };

                        if amount > 0 {
                            info!(
                                "create a Buy deal: resource type:{:?}, amount:{:?}, price:{:?}",
                                ResourceType::Energy,
                                amount,
                                target_price_own
                            );
                            let ret = market::create_order(&make_order_params(
                                OrderType::Buy,
                                ResourceType::Energy,
                                target_price_own,
                                amount,
                                room.name(),
                            ));

                            if let Err(e) = ret {
                                warn!("ret:{:?}", e);
                            }
                        }
                    }
                }
            }
        }
    }
}
