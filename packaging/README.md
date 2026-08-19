# packaging/

KAGURA DB を Linux / Windows 上で一般的なパッケージと同様に導入・運用するための一式です。
利用者向けの手順はリポジトリルートの [README.md](../README.md) の「Linux へのインストール」
「Windows へのインストール」を参照してください。ここではメンテナー向けに、各成果物の生成方法をまとめます。

## 構成

```
packaging/
├── systemd/kagura-db.service   systemdユニット（Linux 3方式で共通）
├── env/kagura.env.example      環境設定ファイルの雛形（Linux用）
├── scripts/install.sh          ソースビルド + FHS配置 + systemd登録（推奨・動作確認済み）
├── scripts/uninstall.sh        install.shの対になるアンインストーラ
├── debian/{postinst,prerm,postrm}   dpkgメンテナスクリプト（cargo-deb用）
├── rpm/{post_install,pre_uninstall,post_uninstall}.sh   rpmスクリプトレット（cargo-generate-rpm用）
└── windows/                     Windows向け一式（下記「Windows向けパッケージ」参照）
    ├── kagura.env.example       環境設定ファイルの雛形（Windows用）
    ├── run-kagura.ps1           起動ラッパー（環境変数読み込み → kagura-db.exe 実行）
    ├── register-task.ps1        設定生成 + タスクスケジューラ登録（install.sh の後半に相当）
    ├── unregister-task.ps1      タスクスケジューラ登録解除
    ├── install.ps1              ソースビルド + 配置 + タスク登録（推奨）
    ├── uninstall.ps1            install.ps1の対になるアンインストーラ
    └── installer.iss            Inno Setup スクリプト（.exeインストーラー + 自動生成アンインストーラー）
```

## 1. install.sh / uninstall.sh（推奨）

追加ツール不要。`cargo build --release` できる環境ならそのまま使えます。

```bash
sudo ./packaging/scripts/install.sh
sudo ./packaging/scripts/uninstall.sh [--purge]
```

## 2. .deb パッケージ（cargo-deb）

`db_client/Cargo.toml` の `[package.metadata.deb]` にメタデータを定義済みです。

```bash
cargo install cargo-deb
cargo build --release -p db_client
cargo deb -p db_client --no-build
# 生成物: target/debian/kagura-db_<version>_<arch>.deb
sudo dpkg -i target/debian/kagura-db_*.deb
```

`maintainer-scripts = "../packaging/debian"` により `postinst`/`prerm`/`postrm` が
専用ユーザー作成・ディレクトリ配置・マスターキー生成・systemd登録を行います。

> **要検証**: `cargo-deb` はバージョンによって `assets` のパス解決やメタデータのキー名が
> 変わることがあります。本リポジトリのサンドボックス環境はネットワーク制限があり
> `cargo-deb` を実インストールしての生成確認ができていません。初回ビルド時は
> `cargo deb -p db_client --no-build` の出力ログを確認し、必要に応じて
> `db_client/Cargo.toml` の `[package.metadata.deb]` を手元の `cargo-deb` バージョンの
> ドキュメントに合わせて調整してください。

## 3. .rpm パッケージ（cargo-generate-rpm）

`db_client/Cargo.toml` の `[package.metadata.generate-rpm]` にメタデータを定義済みです。

```bash
cargo install cargo-generate-rpm
cargo build --release -p db_client
cargo generate-rpm -p db_client
# 生成物: target/generate-rpm/kagura-db-<version>-1.<arch>.rpm
sudo rpm -i target/generate-rpm/kagura-db-*.rpm
```

> **要検証**: 同様に `cargo-generate-rpm` も実行環境でのビルド確認ができていません。
> `post_install_script` 等がファイルパス参照かインラインスクリプトかは
> 手元のバージョンの仕様に合わせて確認してください。

## 共通仕様（Linux）

| 項目 | 値 |
|------|-----|
| バイナリ | `/usr/bin/kagura-db` |
| systemdユニット | `kagura-db.service` |
| 実行ユーザー/グループ | `kagura` / `kagura`（システムアカウント） |
| 設定ファイル | `/etc/kagura-db/kagura.env`（`root:kagura`, 640） |
| データディレクトリ | `/var/lib/kagura-db`（`kagura:kagura`, 750） |
| ログディレクトリ | `/var/log/kagura-db`（`kagura:kagura`, 750） |

`KAGURA_MASTER_KEY` はインストール時（初回のみ）にランダム生成され `/etc/kagura-db/kagura.env`
に書き込まれます。再インストール・アップグレード時は既存の値を保持します（鍵を再生成すると
既存の `.kdb` データが復号できなくなるため）。

---

## Windows向けパッケージ

追加ツール不要。`cargo build --release` できる環境ならそのまま使えます
（PowerShell を「管理者として実行」する必要があります）。

```powershell
powershell -ExecutionPolicy Bypass -File .\packaging\windows\install.ps1
powershell -ExecutionPolicy Bypass -File .\packaging\windows\uninstall.ps1 [-Purge]
```

Windows のサービス制御マネージャー(SCM)に正規登録するには実行ファイル側で
SCM プロトコルへの応答実装が必要（通常は NSSM 等のラッパーが要る）ため、本スクリプトでは
**タスクスケジューラ**（PC起動時に自動実行・異常終了時は自動再起動）で systemd 相当の
常駐運用を実現しています。`register-task.ps1` / `unregister-task.ps1` がその登録・解除処理の本体で、
`install.ps1` / `uninstall.ps1`（ソースビルド一式）と `installer.iss`（Inno Setup、下記）の
両方から共通で呼び出されます。

### .exe インストーラー（Inno Setup）

GUIウィザード形式の `.exe` インストーラーと、Inno Setup が自動生成するアンインストーラー
（コントロールパネル「プログラムと機能」に登録される `unins000.exe`）を作成します。

```powershell
# 事前に Windows 上でリリースビルドしておく
cargo build --release -p db_client

# Inno Setup Compiler (https://jrsoftware.org/isinfo.php) でビルド
iscc packaging\windows\installer.iss
# 生成物: dist\KaguraDB-Setup-<version>.exe
```

> **要検証**: 本リポジトリのサンドボックス環境は Linux のため、`iscc` による実ビルド・
> インストーラー実行・タスクスケジューラ登録の動作確認ができていません。Windows + Inno Setup
> 環境で一度ビルドして動作確認してください。`.msi` 形式が必要な場合は `cargo-wix`
> （WiX Toolset）でも同様の構成（`kagura-db.exe` 配置 + `register-task.ps1`/`unregister-task.ps1`
> をカスタムアクションから呼び出す）を組める見込みですが、本リポジトリには未同梱です。

### 共通仕様（Windows）

| 項目 | 値 |
|------|-----|
| バイナリ | `%ProgramFiles%\KaguraDB\kagura-db.exe` |
| 常駐方式 | タスクスケジューラ タスク `KaguraDB`（`AtStartup` トリガー、異常終了時は自動再起動） |
| 設定ファイル | `%ProgramData%\KaguraDB\config\kagura.env` |
| データディレクトリ | `%ProgramData%\KaguraDB\data` |
| ログディレクトリ | `%ProgramData%\KaguraDB\logs` |

`KAGURA_MASTER_KEY` はインストール時（初回のみ）にランダム生成され `kagura.env` に
書き込まれます。Linux 版と同様、再インストール時は既存の値を保持します。
