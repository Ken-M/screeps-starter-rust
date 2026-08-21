# 凍結 crates.io-index の更新スクリプト (サプライチェーン攻撃対策)
#
# 方針: https://zenn.dev/nojima/articles/338d153cb5dd35
#   crates.io-index のクローンを「N 日前時点」のコミットに固定し、
#   .cargo/config.toml のソース置換 (frozen-index) で参照させることで、
#   公開から N 日未満のクレートバージョンを依存解決から見えなくする (cooldown)。
#   侵害パッケージは公開後数時間〜数日で検知・削除されるため、これで大半を回避できる。
#
# 注意: crates.io-index (GitHub ミラー) は定期的に履歴が squash される
#   ("Collapse index into one commit")。squash が N 日以内に起きた場合、
#   N 日前のコミットは master には存在せず、squash 前の履歴を保持する
#   snapshot-YYYY-MM-DD ブランチ側にあるため、そちらへフォールバックする。
#
# 運用: `cargo update` や依存追加の前に本スクリプトを実行してインデックスを進める。
#   緊急のセキュリティパッチを即入れたいときだけ .cargo/config.toml の
#   [source.crates-io] を一時的にコメントアウトして個別更新する。
#
# 将来: cargo の -Zmin-publish-age (RFC 3923) が stable 化したらそちらへ移行し、
#   この仕組みは廃止する。
param(
    [int]$Days = 7,
    [string]$IndexPath = "D:\GitHub\_tools\crates.io-index"
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path (Join-Path $IndexPath ".git"))) {
    Write-Error "凍結インデックスが見つかりません: $IndexPath`n次で作成してください: git clone --depth=100000 https://github.com/rust-lang/crates.io-index `"$IndexPath`""
}

Write-Host "fetching crates.io-index (master)..."
git -C $IndexPath fetch origin master --depth=100000 --quiet

$rev = git -C $IndexPath rev-list -1 --before "$Days days ago" origin/master

if (-not $rev) {
    # master が N 日以内に squash されている → 最新の snapshot ブランチから探す
    $snapshot = git -C $IndexPath ls-remote --heads origin "snapshot-*" |
        ForEach-Object { ($_ -split "`t")[1] -replace "^refs/heads/", "" } |
        Sort-Object -Descending | Select-Object -First 1

    if (-not $snapshot) {
        Write-Error "$Days 日前のコミットが master に無く、snapshot ブランチも見つかりません"
    }

    Write-Host "master は squash 済みのため $snapshot からコミットを探します..."
    git -C $IndexPath fetch origin ${snapshot}:refs/remotes/origin/$snapshot --depth=100000 --quiet
    $rev = git -C $IndexPath rev-list -1 --before "$Days days ago" "origin/$snapshot"

    if (-not $rev) {
        Write-Error "$Days 日前のコミットが $snapshot にも見つかりません (fetch depth を増やしてください)"
    }
}

git -C $IndexPath checkout --quiet $rev
$when = git -C $IndexPath log -1 --format="%ci" HEAD
Write-Host "frozen-index を $when ($rev) に固定しました (cooldown: $Days 日)"
