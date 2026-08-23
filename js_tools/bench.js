// デプロイ後自動ベンチ。
//
// mmo.log の summary 行 (100 tick ごとに Rust 側が出力) を使い、直近デプロイ
// (= "loading complete" マーカー) の前後で進捗レート・CPU・エネルギー滞留を
// 比較する。回帰 (進捗低下・CPU増・実行時エラー) を人手なしで検知するのが目的。
//
// 使い方:
//   node js_tools/bench.js                 # 今あるログで即座に比較
//   node js_tools/bench.js --wait          # デプロイ後の summary が揃うまで待つ
//   npm run deploy                         # postdeploy が --wait を裏で起動し、
//                                          # 結果を logs/bench.log に追記する
//
// 注意: "loading complete" はデプロイ以外 (グローバルリセット) でも出る。
// その場合は「最後のリセット前後の比較」になるが、回帰検知の用途では同じ。

'use strict';

const fs = require('fs');
const path = require('path');

const args = process.argv.slice(2);
const opt = (name, def) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : def;
};
const has = (name) => args.includes(name);

const LOG_PATH = opt('--log', path.join(__dirname, '..', 'logs', 'mmo.log'));
const OUT_PATH = opt('--out', null);
const INTERVALS = parseInt(opt('--intervals', '3'), 10);
const TIMEOUT_MIN = parseInt(opt('--timeout', '45'), 10);
// autolaunch が渡す「この時刻より後のマーカーだけを見る」境界 (ms epoch)。
// 無指定ならログ中の最後のマーカーを使う。
const MARKER_AFTER = opt('--marker-after', null);

const SUMMARY_RE =
  /^(\S+) .*summary (\w+\d+\w+\d+): rcl:(\d+) progress:(\d+)\/(\d+) downgrade:(\d+) energy:(\d+)\/(\d+)/;
const COLONY_RE =
  /^(\S+) .*summary colony: creeps\[([^\]]*)\] backlog:(-?\d+) cpu:([\d.]+) bucket:(\d+)/;
const MARKER_RE = /^(\S+) .*loading complete/;
const ERROR_RE = /ERROR|panicked/;

function parseLog(text) {
  const rooms = []; // {ts, room, rcl, progress, total}
  const colony = []; // {ts, roles, backlog, cpu, bucket}
  let markerIdx = -1; // rooms/colony 配列に対する「マーカー時点」の位置
  let markerTs = null;
  const errorsAfter = [];

  const markerAfterMs = MARKER_AFTER ? Number(MARKER_AFTER) : 0;

  for (const line of text.split('\n')) {
    let m;
    if ((m = MARKER_RE.exec(line))) {
      const ts = Date.parse(m[1]);
      if (!markerAfterMs || ts >= markerAfterMs) {
        markerIdx = rooms.length;
        markerTs = m[1];
        errorsAfter.length = 0;
      }
      continue;
    }
    if ((m = SUMMARY_RE.exec(line))) {
      rooms.push({
        ts: m[1],
        room: m[2],
        rcl: +m[3],
        progress: +m[4],
        total: +m[5],
      });
      continue;
    }
    if ((m = COLONY_RE.exec(line))) {
      colony.push({ ts: m[1], roles: m[2], backlog: +m[3], cpu: +m[4], bucket: +m[5] });
      continue;
    }
    if (markerIdx >= 0 && ERROR_RE.test(line)) {
      errorsAfter.push(line.trim());
    }
  }
  return { rooms, colony, markerIdx, markerTs, errorsAfter };
}

// summary は 100 tick ごとなので、隣接する2点の progress 差 / 100 が
// progress/tick。RCL が変わった区間 (progress がリセットされる) は除外。
function progressRates(points) {
  const rates = [];
  for (let i = 1; i < points.length; i++) {
    if (points[i].rcl !== points[i - 1].rcl) continue;
    const d = points[i].progress - points[i - 1].progress;
    if (d < 0) continue;
    rates.push(d / 100);
  }
  return rates;
}

const avg = (xs) => (xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : null);
const fmt = (x, digits = 2) => (x == null ? 'n/a' : x.toFixed(digits));

function report(parsed) {
  const { rooms, colony, markerIdx, markerTs, errorsAfter } = parsed;
  const before = rooms.slice(Math.max(0, markerIdx - (INTERVALS + 1)), markerIdx);
  // デプロイ直後の1点はリロードのノイズを含むので、区間としてはそのまま使う
  // (最初の区間だけ低めに出るが、平均 INTERVALS 区間で均される)。
  const after = rooms.slice(markerIdx, markerIdx + INTERVALS + 1);

  const beforeRate = avg(progressRates(before));
  const afterRate = avg(progressRates(after));

  const colonyBefore = colony.slice(Math.max(0, markerIdx - (INTERVALS + 1)), markerIdx);
  const colonyAfter = colony.slice(markerIdx, markerIdx + INTERVALS + 1);
  const beforeCpu = avg(colonyBefore.map((c) => c.cpu));
  const afterCpu = avg(colonyAfter.map((c) => c.cpu));
  const beforeBacklog = avg(colonyBefore.map((c) => c.backlog));
  const afterBacklog = avg(colonyAfter.map((c) => c.backlog));

  const lines = [];
  lines.push(`=== bench @ ${new Date().toISOString()} (deploy marker: ${markerTs}) ===`);
  lines.push(
    `progress/tick : ${fmt(beforeRate)} -> ${fmt(afterRate)}` +
      (beforeRate && afterRate
        ? ` (${((afterRate / beforeRate - 1) * 100).toFixed(0)}%)`
        : '')
  );
  lines.push(
    `cpu/tick      : ${fmt(beforeCpu)} -> ${fmt(afterCpu)}` +
      (beforeCpu && afterCpu ? ` (${((afterCpu / beforeCpu - 1) * 100).toFixed(0)}%)` : '')
  );
  lines.push(`backlog       : ${fmt(beforeBacklog, 0)} -> ${fmt(afterBacklog, 0)}`);
  if (colonyAfter.length) {
    lines.push(`creeps        : [${colonyAfter[colonyAfter.length - 1].roles}]`);
  }
  lines.push(`errors after  : ${errorsAfter.length}`);
  for (const e of errorsAfter.slice(0, 5)) {
    lines.push(`  ${e}`);
  }

  // 回帰判定。閾値はノイズ (spawn タイミング等で ±15% は普通に揺れる) を
  // 踏まえて 25%。
  const flags = [];
  if (errorsAfter.length > 0) flags.push('実行時エラーあり');
  if (beforeCpu && afterCpu && afterCpu > beforeCpu * 1.25)
    flags.push(`CPU +${((afterCpu / beforeCpu - 1) * 100).toFixed(0)}%`);
  if (beforeRate && afterRate && afterRate < beforeRate * 0.75)
    flags.push(`進捗 ${((afterRate / beforeRate - 1) * 100).toFixed(0)}%`);
  lines.push(flags.length ? `verdict       : ⚠ 要確認 (${flags.join(' / ')})` : 'verdict       : ✅ 回帰なし');

  return lines.join('\n');
}

function enoughData(parsed) {
  if (parsed.markerIdx < 0) return false;
  // after 側に INTERVALS 区間ぶん (= INTERVALS+1 点) 揃ったか。
  return parsed.rooms.length - parsed.markerIdx >= INTERVALS + 1;
}

async function main() {
  const deadline = Date.now() + TIMEOUT_MIN * 60 * 1000;

  for (;;) {
    let text = '';
    try {
      text = fs.readFileSync(LOG_PATH, 'utf-8');
    } catch (e) {
      // ログ未生成・ローテート直後など。--wait なら待つ。
    }
    const parsed = parseLog(text);

    if (enoughData(parsed) || !has('--wait')) {
      if (parsed.markerIdx < 0) {
        console.error('bench: no deploy marker (loading complete) found in log');
        process.exit(1);
      }
      const out = report(parsed);
      console.log(out);
      if (OUT_PATH) {
        fs.appendFileSync(OUT_PATH, out + '\n\n');
      }
      return;
    }

    if (Date.now() > deadline) {
      console.error(`bench: timed out after ${TIMEOUT_MIN} min waiting for summaries`);
      process.exit(1);
    }
    await new Promise((r) => setTimeout(r, 15000));
  }
}

main();
