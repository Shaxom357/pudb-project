# KAGURA DB - タスクスケジューラ登録スクリプト
#
# 前提: kagura-db.exe と run-kagura.ps1 が既に $InstallDir に配置済みであること。
# install.ps1 と installer.iss (Inno Setup) の両方から共通で呼び出される。
# 環境設定ファイル (初回のみランダムなマスターキー付きで生成) と、
# systemd ユニットに相当するタスクスケジューラ登録を行う。冪等（再実行しても安全）。

[CmdletBinding()]
param(
    [switch]$NoStart
)

$ErrorActionPreference = "Stop"

$TaskName     = "KaguraDB"
$InstallDir   = "$env:ProgramFiles\KaguraDB"
$ConfDir      = "$env:ProgramData\KaguraDB\config"
$DataDir      = "$env:ProgramData\KaguraDB\data"
$LogDir       = "$env:ProgramData\KaguraDB\logs"
$ExampleEnv   = Join-Path $PSScriptRoot "kagura.env.example"

function Log($msg) { Write-Host "[register-task] $msg" }

New-Item -ItemType Directory -Force -Path $ConfDir | Out-Null
New-Item -ItemType Directory -Force -Path $DataDir | Out-Null
New-Item -ItemType Directory -Force -Path $LogDir  | Out-Null

# ---- 環境設定ファイル (初回のみ生成、マスターキーをランダム生成) ----
$EnvFile = Join-Path $ConfDir "kagura.env"
if (Test-Path $EnvFile) {
    Log "既存の設定ファイルを保持します: $EnvFile"
} else {
    Log "設定ファイルを生成します: $EnvFile (マスターキーをランダム生成)"
    $bytes = New-Object byte[] 32
    [Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
    $masterKey = -join ($bytes | ForEach-Object { $_.ToString("x2") })

    $dbFile  = Join-Path $DataDir "db_data.kdb"
    $logFile = Join-Path $LogDir  "kagura.log"

    @"
# KAGURA DB 環境設定ファイル（自動生成: $(Get-Date -Format o)）
DB_FILE=$dbFile
DB_ADDR=0.0.0.0:3000
KAGURA_MASTER_KEY=$masterKey
KAGURA_LOG_FILE=$logFile
"@ | Set-Content -Path $EnvFile -Encoding utf8
}

if (Test-Path $ExampleEnv) {
    Copy-Item -Path $ExampleEnv -Destination (Join-Path $ConfDir "kagura.env.example") -Force
}

# ---- タスクスケジューラ登録 (systemdサービスに相当) ----
Log "タスクスケジューラへ登録します: $TaskName"
Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue

$action = New-ScheduledTaskAction -Execute "powershell.exe" `
    -Argument "-NoProfile -ExecutionPolicy Bypass -File `"$InstallDir\run-kagura.ps1`"" `
    -WorkingDirectory $DataDir
$trigger = New-ScheduledTaskTrigger -AtStartup
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries `
    -StartWhenAvailable -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1) `
    -ExecutionTimeLimit ([TimeSpan]::Zero)
$principal = New-ScheduledTaskPrincipal -UserId "NT AUTHORITY\SYSTEM" -LogonType ServiceAccount -RunLevel Highest

Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $trigger `
    -Settings $settings -Principal $principal -Description "KAGURA DB - Rust製データベースエンジン" | Out-Null

if (-not $NoStart) {
    Log "サービスを起動します: $TaskName"
    Start-ScheduledTask -TaskName $TaskName
} else {
    Log "-NoStart が指定されたため、即時起動はスキップしました"
}

Log "完了: 設定ファイル $EnvFile / データ $DataDir / ログ $LogDir"
