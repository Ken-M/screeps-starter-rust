# 日次サプライチェーンチェック (サプライチェーン攻撃対策)
#
# タスクスケジューラ "screeps-supply-chain-check" から毎日実行される。
# 手動実行も可: pwsh -File tools\supply-chain-check.ps1
#
# やること:
#   1. tools/update-frozen-index.ps1 で凍結 crates.io-index を「7日前時点」へ前進
#      (グローバル ~/.cargo/config.toml からも参照されるため、放置すると
#       クールダウンが7日を超えて伸び、全プロジェクトの依存解決が古くなりすぎる)
#   2. cargo deny check advisories sources bans (設定は deny.toml)
#      → RustSec advisory / yanked / 不明ソース / ワイルドカード依存を検知する事後検知網
#
# 結果は logs/supply-chain-check.log に追記。いずれかが失敗したら
# logs/SUPPLY-CHAIN-ALERT.txt を書き出し、msg.exe でポップアップ通知する。

$ErrorActionPreference = 'Continue'  # 1つ失敗しても後続チェックは実行し、まとめて報告する

$repo = Split-Path $PSScriptRoot -Parent
$log = Join-Path $repo 'logs\supply-chain-check.log'
$alert = Join-Path $repo 'logs\SUPPLY-CHAIN-ALERT.txt'
New-Item -ItemType Directory -Force (Split-Path $log) | Out-Null

# ローテート (watcher-daemon.log と同じ方針)
if ((Test-Path $log) -and ((Get-Item $log).Length -gt 5MB)) {
    Move-Item $log "$log.1" -Force -ErrorAction SilentlyContinue
}

function Log($msg) { Add-Content $log "$(Get-Date -Format o) $msg" }

$failures = @()
Log "=== supply-chain check start ==="

# ---- 1) 凍結インデックスの前進 (クールダウンを常に7日に保つ) ----
$out = & pwsh -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'update-frozen-index.ps1') 2>&1
$out | ForEach-Object { Log "  [index] $_" }
if ($LASTEXITCODE -ne 0) {
    # インデックスが古いまま止まるのは安全側 (新しいクレートが見えないだけ) だが、
    # 放置すると開発に支障が出るので通知対象にする。
    $failures += "update-frozen-index 失敗 (exit $LASTEXITCODE)"
}

# ---- 1b) 凍結点の鮮度確認 ----
# update-frozen-index.ps1 は git fetch が失敗してもローカル参照から凍結点を
# 固定できれば成功扱いになる (実測: unpack-objects failed でも exit 0)。
# fetch 失敗が続くと凍結点が古いまま気づけないため、クールダウン7日+猶予2日を
# 超えて古い場合はアラート対象にする。
$index_path = 'D:\GitHub\_tools\crates.io-index'
$head_unix = [long](git -C $index_path log -1 --format=%ct 2>$null)
if ($head_unix) {
    $age_days = ((Get-Date).ToUniversalTime() - [DateTimeOffset]::FromUnixTimeSeconds($head_unix).UtcDateTime).TotalDays
    Log ("  [index] 凍結点の古さ: {0:N1} 日" -f $age_days)
    if ($age_days -gt 9) {
        $failures += ("凍結インデックスが古すぎる ({0:N1} 日前) — fetch 失敗が続いている可能性" -f $age_days)
    }
} else {
    $failures += "凍結インデックスの HEAD 日時を取得できない"
}

# ---- 2) cargo deny (deny.toml: advisories sources bans) ----
Push-Location $repo
$out = cargo deny check advisories sources bans 2>&1
$code = $LASTEXITCODE
Pop-Location
$out | ForEach-Object { Log "  [deny] $_" }
if ($code -ne 0) {
    $failures += "cargo deny check 失敗 (exit $code) — RustSec advisory 等を検知した可能性"
}

# ---- 結果報告 ----
if ($failures.Count -gt 0) {
    $failures | ForEach-Object { Log "ALERT: $_" }
    Set-Content $alert ("{0}`n{1}`n詳細: {2}" -f (Get-Date -Format o), ($failures -join "`n"), $log)
    # ログオン中ならポップアップで即時通知 (msg.exe が使えない環境でも他は完走させる)
    msg.exe $env:USERNAME "[screeps] サプライチェーンチェック失敗: $($failures -join ' / ') 詳細は logs\supply-chain-check.log" 2>$null
    exit 1
}

# 全チェック通過 → 過去のアラートマーカーは消す
if (Test-Path $alert) { Remove-Item $alert -Force }
Log "OK: 全チェック通過"
exit 0
