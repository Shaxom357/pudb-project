# KAGURA DB - Windows インストールスクリプト
#
# ソースからリリースビルドし、一般的な Windows アプリと同様に
#   - Program Files 配下へのバイナリ配置
#   - ProgramData 配下への設定/データ/ログディレクトリ配置
#   - タスクスケジューラへの登録（systemd サービスに相当する自動起動・自動再起動）
# を行う。管理者権限の PowerShell で実行すること。再実行しても安全（冪等）。
#
# 使い方:
#   powershell -ExecutionPolicy Bypass -File .\packaging\windows\install.ps1 [-NoStart]
#
#   -NoStart  タスク登録のみ行い、即時起動は行わない

[CmdletBinding()]
param(
    [switch]$NoStart
)

$ErrorActionPreference = "Stop"

$InstallDir = "$env:ProgramFiles\KaguraDB"

function Log($msg) { Write-Host "[install] $msg" }

# ---- 0. 管理者権限チェック ----
$currentPrincipal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $currentPrincipal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Write-Error "管理者権限の PowerShell で実行してください（右クリック→「管理者として実行」）。"
    exit 1
}

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Error "cargo が見つかりません。Rust (https://rustup.rs) をインストールしてください。"
    exit 1
}

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot  = (Resolve-Path (Join-Path $ScriptDir "..\..")).Path

# ---- 1. リリースビルド ----
Log "リリースビルド中... (cargo build --release -p db_client)"
Push-Location $RepoRoot
try {
    cargo build --release -p db_client
    if ($LASTEXITCODE -ne 0) { throw "cargo build に失敗しました" }
} finally {
    Pop-Location
}

$BuiltBin = Join-Path $RepoRoot "target\release\db_client.exe"
if (-not (Test-Path $BuiltBin)) {
    Write-Error "ビルド成果物が見つかりません: $BuiltBin"
    exit 1
}

# ---- 2. バイナリ / 起動ラッパー配置 ----
Log "インストール先ディレクトリを作成します: $InstallDir"
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null

Log "バイナリを配置します: $InstallDir\kagura-db.exe"
Copy-Item -Path $BuiltBin -Destination (Join-Path $InstallDir "kagura-db.exe") -Force
Copy-Item -Path (Join-Path $ScriptDir "run-kagura.ps1") -Destination (Join-Path $InstallDir "run-kagura.ps1") -Force

# ---- 3. 環境設定ファイル生成 + タスクスケジューラ登録 ----
$registerArgs = @{}
if ($NoStart) { $registerArgs["NoStart"] = $true }
& (Join-Path $ScriptDir "register-task.ps1") @registerArgs

$ConfDir = "$env:ProgramData\KaguraDB\config"
$DataDir = "$env:ProgramData\KaguraDB\data"
$LogDir  = "$env:ProgramData\KaguraDB\logs"

@"

インストールが完了しました。

  起動状態確認 : Get-ScheduledTask -TaskName KaguraDB | Get-ScheduledTaskInfo
  起動         : Start-ScheduledTask -TaskName KaguraDB
  停止         : Stop-ScheduledTask -TaskName KaguraDB
  再起動       : Stop-ScheduledTask -TaskName KaguraDB; Start-ScheduledTask -TaskName KaguraDB
  自動起動     : PC起動時に自動的に開始されます（タスクスケジューラ登録済み）
  アプリログ    : $LogDir\kagura.log
  設定ファイル  : $ConfDir\kagura.env
  データファイル: $DataDir\db_data.kdb

"@ | Write-Host
