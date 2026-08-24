// 部屋の実状態スナップショット (読み取りのみ)。ログでは見えない
// 「今この瞬間の在庫と配置」を API から直接取る診断ツール。
//
// 使い方:
//   node js_tools/room-snapshot.js                    # 既定: mmo/shard2/E23N15
//   node js_tools/room-snapshot.js --room E23N15 --shard shard2 --server mmo
//
// 出力: container/storage の在庫、spawn、tower、全 creep の位置・body・積載。

'use strict';

const { ScreepsHttpClient } = require('screeps-api');

const args = process.argv.slice(2);
const opt = (name, def) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : def;
};
const SERVER = opt('--server', 'mmo');
const SHARD = opt('--shard', 'shard2');
const ROOM = opt('--room', 'E23N15');

(async () => {
  const api = await ScreepsHttpClient.fromConfig(SERVER);
  const res = await api.gameRoomObjects(ROOM, SHARD);
  const objs = Object.values(res.objects || res).filter((o) => o && o.type);

  const pos = (o) => `(${String(o.x).padStart(2)},${String(o.y).padStart(2)})`;
  const energy = (o) => (o.store && o.store.energy) || 0;

  for (const o of objs) {
    switch (o.type) {
      case 'container':
        console.log(`container ${pos(o)} energy:${energy(o)}/2000`);
        break;
      case 'storage':
        console.log(`storage   ${pos(o)} energy:${energy(o)}`);
        break;
      case 'spawn':
        console.log(`spawn     ${pos(o)} energy:${energy(o)}/300 ${o.spawning ? 'spawning' : ''}`);
        break;
      case 'tower':
        console.log(`tower     ${pos(o)} energy:${energy(o)}`);
        break;
      case 'controller':
        console.log(`controller${pos(o)} rcl:${o.level} progress:${o.progress}`);
        break;
    }
  }
  for (const o of objs) {
    if (o.type !== 'creep') continue;
    const parts = {};
    for (const b of o.body || []) parts[b.type] = (parts[b.type] || 0) + 1;
    const sig = Object.entries(parts)
      .map(([k, v]) => k[0] + v)
      .join('');
    console.log(
      `creep ${pos(o)} ${o.name} body:${sig} energy:${energy(o)} fatigue:${o.fatigue || 0}`
    );
  }
})().catch((e) => {
  console.error('ERR', e.message);
  process.exit(1);
});
