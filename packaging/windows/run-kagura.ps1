# KAGURA DB - 起動ラッパー (タスクスケジューラから呼び出される)
#
# kagura.env を読み込んでプロセス環境変数に設定してから kagura-db.exe を起動する。
# Windows のタスクスケジューラのアクションには環境変数を直接渡す仕組みが無いため、
# systemd の EnvironmentFile= に相当する処理をこのラッパーで肩代わりする。

$ErrorActionPreference = "Stop"

$InstallDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$EnvFile    = "$env:ProgramData\KaguraDB\config\kagura.env"

if (Test-Path $EnvFile) {
    Get-Content -Path $EnvFile | ForEach-Object {
        $line = $_.Trim()
        if ($line -and -not $line.StartsWith("#") -and $line.Contains("=")) {
            $parts = $line.Split("=", 2)
            [Environment]::SetEnvironmentVariable($parts[0].Trim(), $parts[1].Trim(), "Process")
        }
    }
}

& "$InstallDir\kagura-db.exe"
exit $LASTEXITCODE
