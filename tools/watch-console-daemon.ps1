# Screeps コンソール watcher (js_tools/watch-console.js) を常駐させるラッパー。
#
# タスクスケジューラの "screeps-watch-console" タスクからログオン時に起動される。
# watch-console 自体は WebSocket の切断には自動再接続で耐えるが、プロセス自体が
# 死んだ場合 (例外・強制終了) はここで拾って 15 秒後に再起動する。
#
# 手動での起動/停止:
#   Start-ScheduledTask -TaskName screeps-watch-console
#   Stop-ScheduledTask  -TaskName screeps-watch-console
#
# このスクリプト自身のライフサイクルは logs/watcher-daemon.log に記録する
# (watch-console の中身のログは従来どおり logs/mmo.log)。

$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo

# 二重起動防止 (タスクの手動実行とログオン起動が重なった場合など)
$created = $false
$mutex = New-Object System.Threading.Mutex($true, 'screeps-watch-console-daemon', [ref]$created)
if (-not $created) { exit 0 }

$daemon_log = Join-Path $repo 'logs\watcher-daemon.log'
New-Item -ItemType Directory -Force (Split-Path $daemon_log) | Out-Null

$node = (Get-Command node -ErrorAction SilentlyContinue).Source
if (-not $node) {
    Add-Content $daemon_log "$(Get-Date -Format o) fatal: node not found in PATH"
    exit 1
}

while ($true) {
    Add-Content $daemon_log "$(Get-Date -Format o) starting watch-console (node: $node)"
    # stdout は watch-console が logs/mmo.log にも書くので捨てる。stderr は起動失敗の
    # 手がかりになるので daemon ログへ。
    & $node js_tools\watch-console.js --server mmo --cpu --out logs\mmo.log 2>> $daemon_log | Out-Null
    Add-Content $daemon_log "$(Get-Date -Format o) watch-console exited (code $LASTEXITCODE); restarting in 15s"
    Start-Sleep -Seconds 15
}
