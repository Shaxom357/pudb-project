# KAGURA DB - タスクスケジューラ登録解除スクリプト
#
# uninstall.ps1 と installer.iss (Inno Setup, [UninstallRun]) の両方から共通で呼び出される。
# タスクの停止・削除のみを行う。設定/データ/ログファイルには触れない。

$ErrorActionPreference = "SilentlyContinue"

$TaskName = "KaguraDB"

Write-Host "[unregister-task] タスクを停止・削除します: $TaskName"
Stop-ScheduledTask -TaskName $TaskName
Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false
