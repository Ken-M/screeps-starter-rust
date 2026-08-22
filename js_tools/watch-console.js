// Screeps のコンソール出力を WebSocket で購読し、ログファイルに追記し続ける常駐スクリプト。
//
// 使い方:
//   node js_tools/watch-console.js --server mmo
//   node js_tools/watch-console.js --server mmo --cpu --out logs/mmo.log
//
// 認証は deploy.js と同じく .screeps.yaml から読む (トークンをここには書かない)。
// 自動再接続・再購読は screeps-api 側のデフォルトで有効なので、切断されても復帰する。
//
// 出力形式 (grep しやすいよう 1 行 1 レコードの固定カラム):
//   2026-08-22T10:15:03.123Z [shard3] LOG (INFO) screeps_starter_rust: running creeps cpu:2.5
//   2026-08-22T10:15:03.456Z [shard3] ERR TypeError: ...
//   2026-08-22T10:15:04.000Z [shard3] CPU cpu=8 memory=126435
//   2026-08-22T10:15:05.000Z [-]      SYS disconnected
//
// ERR はゲーム側の実行時例外 (スタックトレース含む)、LOG は console.log 出力
// (bot の log クレート経由の出力もここに来る)、SYS はこのスクリプト自身の
// 接続ライフサイクル。CPU は --cpu 指定時のみ。

const fs = require('fs');
const path = require('path');

const { ScreepsHttpClient, ScreepsSocketClient } = require('screeps-api');
const argv = require('yargs')
  .option('server', {
    describe: 'server to connect to; must be defined in .screeps.yaml servers section',
  })
  .demandOption('server')
  .option('out', {
    describe: 'log file to append to',
    type: 'string',
  })
  .option('cpu', {
    describe: 'also record per-tick CPU and memory usage',
    type: 'boolean',
    default: false,
  })
  .argv;

const out_path = argv.out || path.join('logs', `${argv.server}.log`);

fs.mkdirSync(path.dirname(out_path), { recursive: true });

// ログのサイズ上限。超えたら .log.1 へ退避して書き直す (2世代保持)。
// _startup のスーパーバイザーログと同じポリシー。常駐化に伴い、
// 放っておくと際限なく育つようになったため。
const MAX_LOG_BYTES = 5 * 1024 * 1024;
let log_bytes = fs.existsSync(out_path) ? fs.statSync(out_path).size : 0;

function write_line(entry) {
  if (log_bytes + Buffer.byteLength(entry) > MAX_LOG_BYTES) {
    try {
      fs.renameSync(out_path, `${out_path}.1`);
      log_bytes = 0;
    } catch (err) {
      // ローテート失敗 (他プロセスが掴んでいる等) なら諦めて追記を続ける。
      // 次の書き込みで再挑戦する。
    }
  }
  fs.appendFileSync(out_path, entry);
  log_bytes += Buffer.byteLength(entry);
}

// Screeps サーバーはコンソール出力を HTML エスケープして送ってくる
// (例: `role:"harvester"` が `role:&#x22;harvester&#x22;` になる)。
// そのままだと grep も目視も辛いので元に戻す。
function decode_entities(text) {
  return text
    .replace(/&#x([0-9a-fA-F]+);/g, (_, hex) => String.fromCodePoint(parseInt(hex, 16)))
    .replace(/&#(\d+);/g, (_, dec) => String.fromCodePoint(parseInt(dec, 10)))
    .replace(/&quot;/g, '"')
    .replace(/&apos;/g, "'")
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    // &amp; は最後に処理する (先にやると `&amp;lt;` が `<` に化ける)
    .replace(/&amp;/g, '&');
}

function record(kind, shard, message) {
  // 複数行 (スタックトレース) は行ごとに分解し、どの行も同じ書式になるようにする。
  // こうしておくと grep でエラー行だけ拾っても文脈が失われない。
  const stamp = new Date().toISOString();
  const shard_tag = `[${shard || '-'}]`;
  for (const line of decode_entities(String(message)).split('\n')) {
    if (line.length === 0) continue;
    const entry = `${stamp} ${shard_tag} ${kind} ${line}\n`;
    write_line(entry);
    process.stdout.write(entry);
  }
}

async function main() {
  const api = await ScreepsHttpClient.fromConfig(argv.server);
  const socket = api.socket;

  socket.on(ScreepsSocketClient.CONNECTED, () => record('SYS', null, 'connected'));
  socket.on(ScreepsSocketClient.AUTHED, () => record('SYS', null, 'authenticated'));
  socket.on(ScreepsSocketClient.DISCONNECTED, () => record('SYS', null, 'disconnected'));
  socket.on(ScreepsSocketClient.ERROR, (err) => {
    record('SYS', null, `socket error: ${err && err.message ? err.message : err}`);
  });

  await socket.connect();

  await socket.subscribeUserConsole((event) => {
    const data = event.data || {};

    if (data.messages) {
      for (const line of data.messages.log || []) {
        record('LOG', data.shard, line);
      }
      // ゲーム内コンソールへ手で打ったコマンドの戻り値。
      for (const line of data.messages.results || []) {
        record('RES', data.shard, line);
      }
    }

    // tick 中に投げられた例外。js_src/main.js がこれを拾って VM をリセットする。
    if (data.error) {
      record('ERR', data.shard, data.error);
    }
  });

  if (argv.cpu) {
    await socket.subscribeUserCpu((event) => {
      const data = event.data || {};
      record('CPU', data.shard, `cpu=${data.cpu} memory=${data.memory}`);
    });
  }

  record('SYS', null, `watching console of server '${argv.server}' -> ${out_path}`);

  const shutdown = () => {
    record('SYS', null, 'shutting down');
    socket.disconnect();
    process.exit(0);
  };
  process.on('SIGINT', shutdown);
  process.on('SIGTERM', shutdown);
}

main().catch((err) => {
  record('SYS', null, `fatal: ${err && err.stack ? err.stack : err}`);
  process.exit(1);
});
