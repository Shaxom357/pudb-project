# KAGURA DB - Windows アンインストールスクリプト
#
# 使い方:
#   powershell -ExecutionPolicy Bypass -File .\packaging\windows\uninstall.ps1          # タスク・バイナリのみ削除（データ/設定は残す）
#   powershell -ExecutionPolicy Bypass -File .\packaging\windows\uninstall.ps1 -Purge   # データ/設定/ログも含めて完全削除

[CmdletBinding()]
param(
    [switch]$Purge
)

$ErrorActionPreference = "Stop"

$InstallDir = "$env:ProgramFiles\KaguraDB"
$ConfDir    = "$env:ProgramData\KaguraDB\config"
$DataDir    = "$env:ProgramData\KaguraDB\data"
$LogDir     = "$env:ProgramData\KaguraDB\logs"

function Log($msg) { Write-Host "[uninstall] $msg" }

$currentPrincipal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $currentPrincipal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Write-Error "管理者権限の PowerShell で実行してください（右クリック→「管理者として実行」）。"
    exit 1
}

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
& (Join-Path $ScriptDir "unregister-task.ps1")

if (Test-Path $InstallDir) {
    Log "バイナリを削除します: $InstallDir"
    Remove-Item -Path $InstallDir -Recurse -Force
}

if (-not $Purge) {
    @"
[uninstall] サービス本体を削除しました。
以下は保持されています（完全に削除する場合は -Purge を付けて再実行してください）:
  設定ファイル  : $ConfDir
  データファイル: $DataDir
  ログファイル  : $LogDir
"@ | Write-Host
    exit 0
}

Log "-Purge が指定されたため、設定・データ・ログも削除します"
foreach ($dir in @($ConfDir, $DataDir, $LogDir)) {
    if (Test-Path $dir) { Remove-Item -Path $dir -Recurse -Force }
}

Log "完全に削除しました"
