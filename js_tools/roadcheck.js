// 幹線道路の連結性チェック (読み取りのみ)。
// spawn から各 source / controller / 補給 container まで、道路だけを辿って
// 到達できるかを BFS で確かめる。途切れがあれば分断点を報告する。
'use strict';
const { ScreepsHttpClient } = require('screeps-api');
const args = process.argv.slice(2);
const opt = (n, d) => { const i = args.indexOf(n); return i >= 0 ? args[i + 1] : d; };
const ROOM = opt('--room', 'E23N15'), SHARD = opt('--shard', 'shard2'), SERVER = opt('--server', 'mmo');

(async () => {
  const api = await ScreepsHttpClient.fromConfig(SERVER);
  const res = await api.gameRoomObjects(ROOM, SHARD);
  const o = Object.values(res.objects || res).filter((x) => x && x.type);
  const roads = new Set(o.filter((x) => x.type === 'road').map((r) => `${r.x},${r.y}`));
  const sites = o.filter((x) => x.type === 'constructionSite');
  const roadSites = new Set(sites.filter((s) => s.structureType === 'road').map((s) => `${s.x},${s.y}`));
  const spawn = o.find((x) => x.type === 'spawn');
  const goals = [];
  for (const x of o) {
    if (x.type === 'source') goals.push({ n: `source(${x.x},${x.y})`, x: x.x, y: x.y });
    if (x.type === 'controller') goals.push({ n: `controller(${x.x},${x.y})`, x: x.x, y: x.y });
    if (x.type === 'container') goals.push({ n: `container(${x.x},${x.y})`, x: x.x, y: x.y });
  }
  console.log(`road ${roads.size}本 / 建設中の道路 ${roadSites.size}件 / 全建設サイト ${sites.length}件`);
  for (const s of sites) console.log(`  site (${s.x},${s.y}) ${s.structureType} ${s.progress}/${s.progressTotal}`);

  // 道路(+建設中)を辿る BFS。起点は spawn 隣接の道路。
  const walk = (includeSites) => {
    const net = includeSites ? new Set([...roads, ...roadSites]) : roads;
    const seen = new Set(), q = [];
    for (let dx = -1; dx <= 1; dx++) for (let dy = -1; dy <= 1; dy++) {
      const k = `${spawn.x + dx},${spawn.y + dy}`;
      if (net.has(k)) { seen.add(k); q.push([spawn.x + dx, spawn.y + dy]); }
    }
    while (q.length) {
      const [x, y] = q.pop();
      for (let dx = -1; dx <= 1; dx++) for (let dy = -1; dy <= 1; dy++) {
        const k = `${x + dx},${y + dy}`;
        if (net.has(k) && !seen.has(k)) { seen.add(k); q.push([x + dx, y + dy]); }
      }
    }
    return { net, seen };
  };

  for (const withSites of [false, true]) {
    const { net, seen } = walk(withSites);
    const label = withSites ? '完成後の見込み' : '現在(完成した道路のみ)';
    const isolated = [...net].filter((k) => !seen.has(k));
    console.log(`\n[${label}] spawn から連結: ${seen.size}/${net.size}  孤立: ${isolated.length}`);
    if (isolated.length && isolated.length <= 20) console.log('  孤立マス:', isolated.join(' '));
    for (const g of goals) {
      let ok = false;
      for (let dx = -1; dx <= 1 && !ok; dx++) for (let dy = -1; dy <= 1 && !ok; dy++) {
        if (seen.has(`${g.x + dx},${g.y + dy}`)) ok = true;
      }
      console.log(`  ${ok ? '○' : '×'} ${g.n}`);
    }
  }
})().catch((e) => { console.error('ERR', e.message); process.exit(1); });
