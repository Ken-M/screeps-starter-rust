use crate::util::*;
use log::*;

use screeps::enums::StructureObject;
use screeps::objects::StructureLink;
use screeps::prelude::*;
use screeps::{game, ResourceType};
use std::collections::HashMap;

/// 送信元とみなす残量。これ未満の link からは送らない。
/// 中途半端な量を送っても 3% の損失とクールダウン24tickに見合わない。
const LINK_SEND_THRESHOLD: u32 = 400;

/// 送信先に最低限これだけ空きがあること。
const LINK_RECEIVE_THRESHOLD: i32 = 200;

pub fn run_link() {
    // 部屋ごとに link をまとめる。
    //
    // 旧実装は game::structures() の全部屋横断で残量の最大・最小を1組だけ選んでいた。
    // 部屋Aの link が最大で部屋Bが最小になると transfer_energy は必ず失敗するため、
    // 複数部屋ではリンクが全く機能しない tick が常態化する。
    // また役割を区別せず残量だけで方向を決めるので、運搬 creep が補充した直後の
    // ストレージ側 link がソース側へ送り返す逆流が起き、往復ごとに 3% を失っていた。
    let mut by_room: HashMap<_, Vec<StructureLink>> = HashMap::new();

    for game_structure in game::structures().values() {
        if !check_my_structure(&game_structure) {
            continue;
        }
        if let StructureObject::StructureLink(my_link) = game_structure {
            let Some(room) = my_link.room() else {
                continue;
            };
            by_room.entry(room.name()).or_default().push(my_link);
        }
    }

    for (room_name, links) in by_room.iter() {
        if links.len() < 2 {
            continue;
        }

        // 送信元候補: 十分に溜まっていて、クールダウンが明けているもの。
        // 送信先候補: 受け入れ余地があるもの。
        let mut senders: Vec<&StructureLink> = links
            .iter()
            .filter(|l| {
                l.cooldown() == 0
                    && l.store().get_used_capacity(Some(ResourceType::Energy))
                        >= LINK_SEND_THRESHOLD
            })
            .collect();

        let mut receivers: Vec<&StructureLink> = links
            .iter()
            .filter(|l| {
                l.store().get_free_capacity(Some(ResourceType::Energy))
                    >= LINK_RECEIVE_THRESHOLD
            })
            .collect();

        // 溜まっている順に送り、空いている順に受ける。
        senders.sort_by_key(|l| {
            std::cmp::Reverse(l.store().get_used_capacity(Some(ResourceType::Energy)))
        });
        receivers.sort_by_key(|l| {
            std::cmp::Reverse(l.store().get_free_capacity(Some(ResourceType::Energy)))
        });

        // クールダウンが明けている送信元は全て同 tick で処理する。
        // 旧実装は1 tick に1組しか扱わず、link が4基あってもスループットは1本分だった。
        let mut used_receivers = Vec::new();

        for sender in senders.iter() {
            let sender_id = sender.id();

            let Some(receiver) = receivers.iter().find(|r| {
                r.id() != sender_id && !used_receivers.contains(&r.id())
            }) else {
                break;
            };

            let available = sender.store().get_used_capacity(Some(ResourceType::Energy));
            let space = receiver
                .store()
                .get_free_capacity(Some(ResourceType::Energy))
                .max(0) as u32;
            let amount = std::cmp::min(available, space);

            if amount == 0 {
                continue;
            }

            match sender.transfer_energy(*receiver, Some(amount)) {
                Ok(()) => {
                    info!("link {}: sent {} energy", room_name, amount);
                    used_receivers.push(receiver.id());
                }
                Err(e) => {
                    warn!("couldn't transfer to another link:{:?}", e);
                }
            }
        }
    }
}
