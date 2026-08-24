# タスクスケジューラに日次サプライチェーンチェック "screeps-supply-chain-check" を登録する。
#
# 使い方 (管理者権限は不要):
#   pwsh -File tools\install-supply-chain-check-task.ps1            # 登録して即時テスト実行
#   pwsh -File tools\install-supply-chain-check-task.ps1 -NoStart   # 登録のみ
#   pwsh -File tools\install-supply-chain-check-task.ps1 -Uninstall # 削除
#
# 登録内容 (install-watch-console-task.ps1 と同じ流儀):
#   - 毎日 12:00 に tools/supply-chain-check.ps1 を非表示ウィンドウで実行
#   - 実行時刻を逃したら次の機会に実行 / バッテリー駆動でも実行・継続
#   - 常駐ではないため実行時間制限は既定のまま (暴走時は自動終了)

param(
    [switch] $Uninstall,
    [switch] $NoStart
)

$ErrorActionPreference = 'Stop'

$task_name = 'screeps-supply-chain-check'
$script = Join-Path $PSScriptRoot 'supply-chain-check.ps1'

if ($Uninstall) {
    Stop-ScheduledTask -TaskName $task_name -ErrorAction SilentlyContinue
    Unregister-ScheduledTask -TaskName $task_name -Confirm:$false
    Write-Host "removed task '$task_name'"
    return
}

if (-not (Test-Path $script)) {
    throw "check script not found: $script"
}

$pwsh_path = (Get-Command pwsh).Source
$action = New-ScheduledTaskAction -Execute $pwsh_path `
    -Argument "-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File `"$script`""
$trigger = New-ScheduledTaskTrigger -Daily -At '12:00'
$settings = New-ScheduledTaskSettingsSet `
    -StartWhenAvailable `
    -AllowStartIfOnBatteries `
    -DontStopIfGoingOnBatteries

Register-ScheduledTask -TaskName $task_name `
    -Action $action -Trigger $trigger -Settings $settings `
    -Description 'サプライチェーン日次チェック (凍結crates.io-index前進 + cargo deny)' `
    -Force | Out-Null
Write-Host "registered task '$task_name' (daily 12:00, script: $script)"

if (-not $NoStart) {
    Start-ScheduledTask -TaskName $task_name
    Write-Host "started task '$task_name' (テスト実行。結果は logs\supply-chain-check.log)"
}
