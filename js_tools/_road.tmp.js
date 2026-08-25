const { ScreepsHttpClient } = require('screeps-api');
(async () => {
  const api = await ScreepsHttpClient.fromConfig('mmo');
  const res = await api.gameRoomObjects('E23N15', 'shard2');
  const o = Object.values(res.objects || res).filter((x) => x && x.type);
  const roads = o.filter((x) => x.type === 'road');
  const sites = o.filter((x) => x.type === 'constructionSite');
  const set = new Set(roads.map(r => `${r.x},${r.y}`));
  console.log(`road ${roads.length}本 / constructionSite ${sites.length}件`);
  for (const s of sites) console.log(`  site (${s.x},${s.y}) ${s.structureType} ${s.progress}/${s.progressTotal}`);
  // 隣接する道路が1本以下 = 経路として途切れている端点
  const ends = [];
  for (const r of roads) {
    let n = 0;
    for (let dx=-1; dx<=1; dx++) for (let dy=-1; dy<=1; dy++) {
      if (dx===0&&dy===0) continue;
      if (set.has(`${r.x+dx},${r.y+dy}`)) n++;
    }
    if (n <= 1) ends.push(`(${r.x},${r.y}):隣接${n}`);
  }
  console.log(`端点/孤立: ${ends.length}箇所  ${ends.join(' ')}`);
  const hp = roads.filter(r => r.hits < r.hitsMax * 0.5);
  console.log(`HP半分以下の道路: ${hp.length}本 ${hp.slice(0,10).map(r=>`(${r.x},${r.y}):${r.hits}/${r.hitsMax}`).join(' ')}`);
})().catch((e) => { console.error('ERR', e.message); process.exit(1); });
