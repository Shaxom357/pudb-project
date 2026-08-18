# packaging/

KAGURA DB を Linux 上で一般的なパッケージと同様に導入・運用するための一式です。
利用者向けの手順はリポジトリルートの [README.md](../README.md) の「Linux へのインストール」を参照してください。
ここではメンテナー向けに、各成果物の生成方法をまとめます。

## 構成

```
packaging/
├── systemd/kagura-db.service   systemdユニット（3方式で共通）
├── env/kagura.env.example      環境設定ファイルの雛形
├── scripts/install.sh          ソースビルド + FHS配置 + systemd登録（推奨・動作確認済み）
├── scripts/uninstall.sh        installer.shの対になるアンインストーラ
├── debian/{postinst,prerm,postrm}   dpkgメンテナスクリプト（cargo-deb用）
└── rpm/{post_install,pre_uninstall,post_uninstall}.sh   rpmスクリプトレット（cargo-generate-rpm用）
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

## 共通仕様

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
