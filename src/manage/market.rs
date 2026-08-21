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

                    if num_data > 0 {
                        target_price = target_price / num_data as f64;
                        target_price_own = target_price_own / num_data as f64;
                    }

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
                            let amount =
                                (((cur_credits * 0.5) / 0.05) / target_price_own) as u32;
                            let amount = std::cmp::min(amount, stored_amount / 2);
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

                if num_data > 0 {
                    target_price = target_price / num_data as f64;
                    target_price_own = target_price_own / num_data as f64;
                }

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
                        let amount = ((cur_credits * 0.7) / (target_price_own * 1.05)) as u32;
                        let amount = std::cmp::min(amount, terminal_energy_capacity as u32 / 2);
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
