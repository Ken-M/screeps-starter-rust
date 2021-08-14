
use log::*;


use screeps::{
    find, game, game::market, game::market::*, local::ObjectId, objects::StructureLink,
    pathfinder::SearchResults, prelude::*, Attackable, Creep, Part, ResourceType, ReturnCode,
    RoomObjectProperties, StructureType,
};

pub fn run_market() {
    info!("running market");

    let market_count = screeps::memory::root()
        .i32("market_counter")
        .unwrap_or(Some(0))
        .unwrap_or(0);
    screeps::memory::root().set("market_counter", market_count + 1);

    let cur_credits = game::market::credits();
    info!("current credits:{:?}", cur_credits);

    for my_order in game::market::orders().values() {
        info!("my order:{:?}", my_order);
    }

    if market_count % 10 == 0 {
        for room in game::rooms::values() {
            if let Some(my_terminal) = room.terminal() {
                //sell orders.
                let resource_type_list = my_terminal.store_types();
                let terminal_energy = my_terminal.store_of(ResourceType::Energy);

                if my_terminal.cooldown() > 0 {
                    continue;
                }

                for resource in resource_type_list {
                    // don't sell energy.
                    if resource == ResourceType::Energy {
                        continue;
                    }

                    let mut stored_amount = my_terminal.store_of(resource);
                    if stored_amount <= 0 {
                        continue;
                    }

                    // calc target price.
                    let market_history = game::market::get_history(Some(
                        screeps::MarketResourceType::Resource(resource),
                    ));

                    let mut target_price: f64 = 0 as f64;
                    let mut num_data: u128 = 0;

                    for history in market_history {
                        target_price += history.avg_price + history.stddev_price;
                        num_data += 1;
                    }

                    if num_data > 0 {
                        target_price = target_price / num_data as f64;
                    }

                    // check buy orders.
                    let all_orders = game::market::get_all_orders(Some(
                        screeps::MarketResourceType::Resource(resource),
                    ));
                    for order in all_orders {
                        if order.order_type == OrderType::Buy {
                            if order.price >= target_price {
                                let amount = terminal_energy as f64
                                    / game::market::calc_transaction_cost(
                                        1,
                                        room.name(),
                                        order.room_name.expect("not resource order"),
                                    );
                                let amount = std::cmp::min(amount as u32, stored_amount);
                                let amount = std::cmp::min(amount as u32, order.remaining_amount);
                                if amount > 0 {
                                    info!("deal: {:?}, amount:{:?}", order, amount);

                                    let ret = game::market::deal(
                                        order.id.as_str(),
                                        amount,
                                        Some(room.name()),
                                    );

                                    if ret == ReturnCode::Ok {
                                        stored_amount -= amount;
                                    } else {
                                        warn!("ret:{:?}", ret);
                                    }
                                }
                            }
                        }
                    }

                    // make sell orders.
                    let amount = (((cur_credits as f64 * 0.5) / 0.05) / target_price) as u32;
                    let amount = std::cmp::min(amount, stored_amount / 2);
                    info!(
                        "create a Sell deal: resource type:{:?}, amount:{:?}, price:{:?}",
                        resource, amount, target_price
                    );
                    let ret = game::market::create_order(
                        OrderType::Sell,
                        screeps::MarketResourceType::Resource(resource),
                        target_price,
                        amount,
                        Some(room.name()),
                    );

                    if ret != ReturnCode::Ok {
                        warn!("ret:{:?}", ret);
                    }
                }
            }
        }
    } else if market_count % 10 == 5 {
        for room in game::rooms::values() {
            if let Some(my_terminal) = room.terminal() {
                //buy energy orders.
                let mut terminal_energy_capacity =
                    my_terminal.store_free_capacity(Some(ResourceType::Energy));
                let terminal_energy = my_terminal.store_of(ResourceType::Energy);

                if my_terminal.cooldown() > 0 {
                    continue;
                }

                if terminal_energy_capacity <= 0 {
                    continue;
                }

                // calc target price.
                let market_history = game::market::get_history(Some(
                    screeps::MarketResourceType::Resource(ResourceType::Energy),
                ));

                let mut target_price: f64 = 0 as f64;
                let mut num_data: u128 = 0;

                for history in market_history {
                    target_price += history.avg_price - history.stddev_price;
                    num_data += 1;
                }

                if num_data > 0 {
                    target_price = target_price / num_data as f64;
                }

                // check buy orders.
                let all_orders = game::market::get_all_orders(Some(
                    screeps::MarketResourceType::Resource(ResourceType::Energy),
                ));
                for order in all_orders {
                    if order.order_type == OrderType::Sell {
                        if order.price <= target_price {
                            let amount = terminal_energy as f64
                                / game::market::calc_transaction_cost(
                                    1,
                                    room.name(),
                                    order.room_name.expect("not resource order"),
                                );
                            let amount =
                                std::cmp::min(amount as u32, terminal_energy_capacity as u32);
                            let amount = std::cmp::min(amount as u32, order.remaining_amount);
                            if amount > 0 {
                                info!("make a deal: {:?}, amount:{:?}", order, amount);

                                let ret = game::market::deal(
                                    order.id.as_str(),
                                    amount,
                                    Some(room.name()),
                                );

                                if ret == ReturnCode::Ok {
                                    terminal_energy_capacity -= amount as i32;
                                } else {
                                    warn!("ret:{:?}", ret);
                                }
                            }
                        }
                    }
                }

                // make buy orders.
                let amount = (((cur_credits as f64 * 0.5) / 0.05) / target_price) as u32;
                let amount = std::cmp::min(amount, terminal_energy_capacity as u32 / 2);
                info!(
                    "create a Buy deal: resource type:{:?}, amount:{:?}, price:{:?}",
                    ResourceType::Energy,
                    amount,
                    target_price
                );
                let ret = game::market::create_order(
                    OrderType::Buy,
                    screeps::MarketResourceType::Resource(ResourceType::Energy),
                    target_price,
                    amount,
                    Some(room.name()),
                );

                if ret != ReturnCode::Ok {
                    warn!("ret:{:?}", ret);
                }
            }
        }
    }
}
