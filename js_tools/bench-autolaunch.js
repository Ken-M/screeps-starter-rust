// npm の postdeploy フックから呼ばれ、bench.js --wait を切り離して起動する。
//
// デプロイ後の summary が揃うまで 20 分程度かかるため、deploy コマンド自体は
// ブロックせず、待機はバックグラウンドに任せる。結果は logs/bench.log に
// 追記される (確認: `Get-Content logs/bench.log -Tail 15`)。

'use strict';

const { spawn } = require('child_process');
const path = require('path');

const root = path.join(__dirname, '..');
const child = spawn(
  process.execPath,
  [
    path.join(__dirname, 'bench.js'),
    '--wait',
    '--out',
    path.join(root, 'logs', 'bench.log'),
    // 今回のデプロイより前の "loading complete" を拾わないよう境界を渡す。
    '--marker-after',
    String(Date.now()),
  ],
  { cwd: root, detached: true, stdio: 'ignore' }
);
child.unref();

console.log('bench: scheduled (result will be appended to logs/bench.log in ~20 min)');
