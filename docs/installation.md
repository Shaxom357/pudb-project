# インストール（本番運用）

一般的な OS サービスとして KAGURA DB を常駐運用するための手順です。開発時の起動方法は
[../README.md](../README.md#クイックスタート) を参照してください。

- [Linux へのインストール（systemd連携）](#linux-へのインストールsystemd連携)
- [Windows へのインストール](#windows-へのインストール工事中)

---

## Linux へのインストール（systemd連携）

一般的な Linux パッケージ（nginx・PostgreSQL 等）と同様に、専用の非rootシステムユーザーで
動作する systemd サービスとして導入できます。`systemctl start/stop/restart/enable` による
プロセス管理、`journalctl` によるログ確認、FHS準拠のディレクトリ配置に対応しています。

導入方法は3種類用意しています（詳細は [../packaging/README.md](../packaging/README.md)）。

| 方法 | コマンド | 備考 |
|------|---------|------|
| インストールスクリプト（推奨） | `sudo ./packaging/scripts/install.sh` | 追加ツール不要。ソースから即ビルド＆導入 |
| .deb パッケージ | `cargo deb -p db_client --no-build` → `dpkg -i` | Debian/Ubuntu系。要 `cargo-deb` |
| .rpm パッケージ | `cargo generate-rpm -p db_client` → `rpm -i` | RHEL/Fedora/Amazon Linux系。要 `cargo-generate-rpm` |

### 配置内容

| 項目 | パス |
|------|------|
| バイナリ | `/usr/bin/kagura-db` |
| CLIクライアント | `/usr/bin/kdb`（`kdb login` 等。3種類の導入方法すべてで自動配置される。詳細は [cli.md](cli.md) を参照） |
| systemdユニット | `kagura-db.service` |
| 環境設定ファイル | `/etc/kagura-db/kagura.env`（`DB_ADDR` / `DB_FILE` / `KAGURA_MASTER_KEY` / `KAGURA_LOG_FILE` / `KAGURA_AUTH_FILE`） |
| データディレクトリ | `/var/lib/kagura-db` |
| ログディレクトリ | `/var/log/kagura-db` |
| 実行ユーザー | `kagura`（システムアカウント・ログインシェルなし） |

`KAGURA_MASTER_KEY`（KDB暗号化マスターキー）はインストール時に自動でランダム生成され、
`/etc/kagura-db/kagura.env` に `root:kagura` 640 権限で保存されます（既存ファイルがある場合は上書きしません）。

管理者ログイン用の認証情報（`KAGURA_AUTH_FILE`、既定 `/var/lib/kagura-db/auth.json`）は
初回起動時にサーバー自身が自動作成します（username=`kagura` / 初期パスワード=`root`）。
起動後は必ず `kdb login` の後 `curl -X PUT .../auth/password` またはWeb UIの設定画面から
パスワードを変更してください（詳細は [authentication.md](authentication.md) を参照）。

インストール後は `kdb login -u kagura -p root -a localhost:3000` でログインを確認できます
（`sudo ./packaging/scripts/install.sh` は完了メッセージにもこのコマンド例を表示します）。

### 運用コマンド

```bash
sudo systemctl start kagura-db      # 起動
sudo systemctl stop kagura-db       # 停止
sudo systemctl restart kagura-db    # 再起動
sudo systemctl status kagura-db     # 状態確認
sudo systemctl enable kagura-db     # OS起動時に自動起動
journalctl -u kagura-db -f          # systemdログ（標準出力）を追跡
tail -f /var/log/kagura-db/kagura.log  # アプリケーションログ（JSON Lines）
```

### アンインストール

```bash
sudo ./packaging/scripts/uninstall.sh          # サービス・バイナリのみ削除（データ/設定は保持）
sudo ./packaging/scripts/uninstall.sh --purge   # データ・設定・専用ユーザーも含め完全削除
```

---

## Windows へのインストール（工事中）

Windows では、PC 起動時に自動的に開始され異常終了時は自動再起動される常駐アプリとして
導入できます（Windows のサービス制御マネージャーに正規登録するには実行ファイル側の対応が
必要なため、代わりに**タスクスケジューラ**で同等の常駐運用を実現しています）。

導入方法は2種類用意しています（詳細は [../packaging/README.md](../packaging/README.md)）。

| 方法 | コマンド | 備考 |
|------|---------|------|
| インストールスクリプト（推奨） | `powershell -ExecutionPolicy Bypass -File .\packaging\windows\install.ps1` | 追加ツール不要。ソースから即ビルド＆導入。管理者権限のPowerShellで実行 |
| `.exe` インストーラー | `iscc packaging\windows\installer.iss` → 生成された `KaguraDB-Setup-*.exe` を実行 | GUIウィザード形式。要 [Inno Setup](https://jrsoftware.org/isinfo.php)。アンインストーラーは自動生成され「プログラムと機能」に登録される |

### 配置内容

| 項目 | パス |
|------|------|
| バイナリ | `%ProgramFiles%\KaguraDB\kagura-db.exe` |
| 常駐方式 | タスクスケジューラ タスク `KaguraDB`（PC起動時に自動実行、異常終了時は自動再起動） |
| 環境設定ファイル | `%ProgramData%\KaguraDB\config\kagura.env`（`DB_ADDR` / `DB_FILE` / `KAGURA_MASTER_KEY` / `KAGURA_LOG_FILE` / `KAGURA_AUTH_FILE`） |
| データディレクトリ | `%ProgramData%\KaguraDB\data` |
| ログディレクトリ | `%ProgramData%\KaguraDB\logs` |

`KAGURA_MASTER_KEY`（KDB暗号化マスターキー）はインストール時に自動でランダム生成され、
`kagura.env` に保存されます（既存ファイルがある場合は上書きしません）。

管理者ログイン用の認証情報（`KAGURA_AUTH_FILE`、既定 `%ProgramData%\KaguraDB\data\auth.json`）は
初回起動時にサーバー自身が自動作成します（username=`kagura` / 初期パスワード=`root`）。
起動後は必ず `PUT /auth/password` またはWeb UIの設定画面からパスワードを変更してください
（詳細は [authentication.md](authentication.md) を参照）。

### 運用コマンド

```powershell
Start-ScheduledTask -TaskName KaguraDB                                    # 起動
Stop-ScheduledTask -TaskName KaguraDB                                     # 停止
Stop-ScheduledTask -TaskName KaguraDB; Start-ScheduledTask -TaskName KaguraDB   # 再起動
Get-ScheduledTask -TaskName KaguraDB | Get-ScheduledTaskInfo              # 状態確認
Get-Content -Tail 50 -Wait $env:ProgramData\KaguraDB\logs\kagura.log      # アプリログ追跡
```

### アンインストール

```powershell
powershell -ExecutionPolicy Bypass -File .\packaging\windows\uninstall.ps1          # タスク・バイナリのみ削除（データ/設定は保持）
powershell -ExecutionPolicy Bypass -File .\packaging\windows\uninstall.ps1 -Purge   # データ・設定・ログも含め完全削除
```

`.exe` インストーラーで導入した場合は「プログラムと機能」からアンインストール（タスク登録解除のみ。
データ/設定/ログは既定で保持されるので、完全削除する場合は `uninstall.ps1 -Purge` を利用してください）。

> **要検証**: 本リポジトリのサンドボックス環境は Linux のため、これらのスクリプトの
> Windows 実機での動作確認ができていません。導入前に検証環境で一度お試しください。
