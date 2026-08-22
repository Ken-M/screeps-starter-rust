# タスクスケジューラに watch-console 常駐タスク "screeps-watch-console" を登録する。
#
# 使い方 (管理者権限は不要):
#   pwsh -File tools\install-watch-console-task.ps1            # 登録して即起動
#   pwsh -File tools\install-watch-console-task.ps1 -NoStart   # 登録のみ
#   pwsh -File tools\install-watch-console-task.ps1 -Uninstall # 削除
#
# 登録内容 (2026-08-23 に手動で構築した設定の再現):
#   - ログオン時に tools/watch-console-daemon.ps1 を非表示ウィンドウで起動
#   - 実行時間制限なし (既定の「3日で強制終了」を無効化。常駐なので必須)
#   - タスク自体が失敗したら1分間隔で最大99回再起動
#   - 起動時刻を逃したら次の機会に開始 / バッテリー駆動でも起動・継続

param(
    [switch] $Uninstall,
    [switch] $NoStart
)

$ErrorActionPreference = 'Stop'

$task_name = 'screeps-watch-console'
$daemon = Join-Path $PSScriptRoot 'watch-console-daemon.ps1'

if ($Uninstall) {
    Stop-ScheduledTask -TaskName $task_name -ErrorAction SilentlyContinue
    Unregister-ScheduledTask -TaskName $task_name -Confirm:$false
    Write-Host "removed task '$task_name'"
    return
}

if (-not (Test-Path $daemon)) {
    throw "daemon script not found: $daemon"
}

$pwsh_path = (Get-Command pwsh).Source
$action = New-ScheduledTaskAction -Execute $pwsh_path `
    -Argument "-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File `"$daemon`""
$trigger = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME
$settings = New-ScheduledTaskSettingsSet `
    -ExecutionTimeLimit ([TimeSpan]::Zero) `
    -RestartCount 99 `
    -RestartInterval (New-TimeSpan -Minutes 1) `
    -StartWhenAvailable `
    -AllowStartIfOnBatteries `
    -DontStopIfGoingOnBatteries

Register-ScheduledTask -TaskName $task_name `
    -Action $action -Trigger $trigger -Settings $settings `
    -Description 'Screeps console watcher (logs/mmo.log) を常駐させる' `
    -Force | Out-Null
Write-Host "registered task '$task_name' (logon trigger, daemon: $daemon)"

if (-not $NoStart) {
    Start-ScheduledTask -TaskName $task_name
    Write-Host "started task '$task_name'"
}
