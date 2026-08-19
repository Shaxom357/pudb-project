# 🗄️ KAGURA DB

> Rust で一から実装したオリジナルのデータベースエンジン。  
> 列指向ストレージ・ラベル検索・KDB暗号化バイナリ形式・SQL SELECT/INSERT/UPDATE/DELETE・REST API・Web管理UI を備えたフルスタックな DB システムです。  
> バージョンの情報については以下の補足を確認ください。  
> 補足  
> ・メジャーバージョンが異なると互換性は無くなります。  
> ・メジャーバージョンが一致し、マイナーバージョンだけが異なる場合問題なく移行ができ互換性を保ちます。

**バージョン: `2.5.1`**

| 区分 | 説明 |
|------|------|
| **A = 2** (メジャー) | ラベルの重複付与を禁止する破壊的変更。既存レコードへの同一ラベル再付与は従来「冪等（サイレント成功）」だったが、変更せずエラーを返す仕様に変更（`POST /records/{id}/labels` は既存クライアントの再送処理などに影響しうる） |
| **B = 5** (マイナー) | Windows へのインストール機能を新規追加。PowerShellスクリプト（`install.ps1`/`uninstall.ps1`）と、`.exe`インストーラー+自動生成アンインストーラーを作る Inno Setup スクリプト（`installer.iss`）を同梱（1機能追加） |
| **C = 1** (ビルド) | Web UI の Records 画面で、数万〜10万件規模のデータを開くと「Connection failed」と誤表示されクラッシュしたように見える不具合を修正 |

---

## 📋 目次

- [概要](#概要)
- [アーキテクチャ](#アーキテクチャ)
- [機能一覧](#機能一覧)
- [プロジェクト構成](#プロジェクト構成)
- [クイックスタート](#クイックスタート)
- [Linux へのインストール（systemd連携）](#linux-へのインストールsystemd連携)
- [Windows へのインストール](#windows-へのインストール)
- [KDB ストレージフォーマット](#kdb-ストレージフォーマット)
- [SQL 機能](#sql-機能)
- [ログ機能](#ログ機能)
- [各クレートの詳細](#各クレートの詳細)
- [API リファレンス](#api-リファレンス)
- [Web 管理UI](#web-管理ui)
- [テストデータ生成](#テストデータ生成)
- [Python バインディング](#python-バインディング)
- [テスト](#テスト)
- [技術スタック](#技術スタック)
- [バージョン履歴](#バージョン履歴)

---

## 概要

KAGURA DB は Rust で一から実装したデータベースエンジンです。  
リレーショナル DB とは異なる **列指向ストレージ** と **ラベル（タグ）ベースの検索** を特徴とします。

| 特徴 | 説明 |
|------|------|
| **列指向ストレージ** | カラムごとに `Vec<Option<DataType>>` で保持し、カラム追加が柔軟 |
| **ラベル仮想テーブル** | `HashMap<label, Vec<row_index>>` による O(1) のラベルインデックス |
| **スキーマレス** | レコードごとに任意のカラムを持てる |
| **KDB 暗号化ストレージ** | XChaCha20-Poly1305 AEAD による暗号化バイナリ形式（`.kdb`） |
| **WAL 高速挿入** | Write-Ahead Log 追記方式で INSERT が O(1)（ファイル全体の再書き込みなし） |
| **SQL SELECT/INSERT/UPDATE/DELETE** | `FROM label.xxx` / `INTO (label.xxx)` / `UPDATE label.xxx SET ...` / `DELETE FROM label.xxx` 構文による独自拡張 SQL クエリ |
| **多言語対応** | REST API と C FFI（Python 向け）の 2 種インターフェース |

---

## アーキテクチャ

```
ブラウザ (Web UI)       Python         curl / HTTP クライアント
      |                   |                      |
      +--------+----------+                      |
               | HTTP                            | HTTP
    +----------+----------------------------------+----------+
    |              db_client  (axum REST API + Web UI)       |
    |   handlers / label_handlers / info_handlers            |
    |   sql_handlers / settings_handlers / app / models / ui |
    +----------------------------+----------------------------+
                                 | Rust クレート依存
    +----------------------------+----------------------------+
    |         dynamic_label_management                        |
    |   LabelManager: AND/OR検索・リネーム・統計             |
    +----------------------------+----------------------------+
                                 |
    +----------------------------+----------------------------+
    |              db_engine  (コアエンジン)                  |
    |   Database: CRUD / ColumnStore / label_index           |
    |   codec:      バイナリシリアライズ                      |
    |   crypto:     XChaCha20-Poly1305 AEAD (純Rust実装)    |
    |   kdb_store:  .kdb ファイル管理 (WAL方式)              |
    +----------------------------+----------------------------+
                                 | C FFI (.so)
    +----------------------------+----------------------------+
    |       db_ffi + examples/python                          |
    +----------------------------------------------------------+

    +----------------------------------------------------------+
    |       sql_engine  (SQL SELECT/INSERT/UPDATE/DELETE       |
    |                     エンジン)                            |
    |   ast / parser / executor                               |
    |   手書き再帰下降パーサー（外部ライブラリ不使用）         |
    +----------------------------------------------------------+
```

---

## 機能一覧

### db_engine（コアエンジン）
- ✅ CRUD（Insert / Get / Update / Delete）
- ✅ 列指向ストレージ（`ColumnStore`）
- ✅ ラベル付与・検索・削除（attach / detach）。**同一レコードへのラベル重複付与は禁止**：既に付与済みのラベルを再度付与しようとした場合は変更せず `DuplicateLabel` エラーを返す（INSERT で1レコードに同じラベルを複数指定した場合も同様にエラー）
- ✅ ラベル仮想テーブル（`HashMap<String, Vec<usize>>`）による高速検索
- ✅ 5 種類のデータ型（Text / Integer / Float / Boolean / Null）
- ✅ **KDB バイナリ暗号化ストレージ**（`.kdb` 形式）
  - XChaCha20-Poly1305 AEAD による暗号化（外部クレート不使用・純 Rust 実装）
  - WAL（Write-Ahead Log）追記方式で INSERT が O(1) の高速書き込み
  - ファイル固有 Salt による鍵派生（`HChaCha20`）
  - 改ざん検知（Poly1305 MAC）
- ✅ JSON 形式永続化（後方互换・アトミック書き込み）
- ✅ `KAGURA_MASTER_KEY` 環境変数によるカスタムマスターキー設定

### db_client（REST API サーバー）
- ✅ 18 本のエンドポイント（レコード CRUD・ラベル管理・DB 情報・SQL・設定・ログ・Web UI）
- ✅ KDB モード（`.kdb`）と JSON モードの自動判別・後方互换
- ✅ 起動時自動ロード・書き込み時自動セーブ
- ✅ 環境変数による設定（`DB_FILE` / `DB_ADDR` / `KAGURA_MASTER_KEY`）
- ✅ ブラウザ Web 管理 UI（Records / DB Info / SQL Query / 設定 の 4 ビュー）
- ✅ `GET /db/info` でバージョン・ストレージ状態・統計を取得
- ✅ `POST /sql` で SQL SELECT / INSERT / UPDATE / DELETE クエリを実行
- ✅ `GET /settings` / `PUT /settings` で HTTPリクエスト（`/records` `/labels` 系 REST API）受付のオンオフを切替可能（**既定は無効**。データ操作は基本 SQL 経由とし、REST API は大量テストデータ投入など用途に応じて有効化する）
- ✅ Web UI の Records 一覧は `POST /sql`（`SELECT * FROM label.*`）経由で取得するため、REST API が無効でも常に閲覧可能
- ✅ **ログ出力保管機能**: 起動/停止時刻、停止理由（正常 / エラー / HW・ストレージ障害）、SQL・HTTPリクエストの成功/失敗、Webクライアントの応答時間、DB保存・レコード/ラベル操作などを JSON Lines 形式でファイルへ永続保存（詳細は [ログ機能](#ログ機能) を参照）。`GET /logs` および Web UI の「ログ」ビューから閲覧可能

### sql_engine（SQL SELECT/INSERT/UPDATE/DELETE エンジン）
- ✅ 独自拡張 SQL `SELECT ... FROM label.xxx` 構文
- ✅ `WHERE` 句（`=` `!=` `<` `<=` `>` `>=` `LIKE`）
- ✅ `AND` / `OR` / `NOT` / 括弧による複合条件
- ✅ `FROM label.A AND label.B`（ラベル AND 絞り込み）
- ✅ `FROM label.A OR label.B`（ラベル OR 結合）
- ✅ `FROM label.*`（全レコード対象）
- ✅ `ORDER BY col [ASC|DESC]`（複数カラム対応）
- ✅ `LIMIT n`
- ✅ カラム指定 `SELECT col1, col2 FROM ...`
- ✅ 大文字小文字無視・セミコロン対応
- ✅ `SELECT *` は `id` 列を先頭、`labels` 列（カンマ区切り）を末尾に付与して返す
- ✅ 独自拡張 SQL `INSERT INTO (label.xxx) VALUE (...)` 構文
- ✅ `INSERT INTO (label.a, label.b) VALUE (...)`（複数ラベルへの同時付与）
- ✅ `INTO (label.xxx) (col1, col2, ...)` によるカラム順の明示指定（省略時は既存カラムのソート順に対応）
- ✅ 存在しないラベルは INSERT 時に自動作成
- ✅ 独自拡張 SQL `UPDATE label.xxx SET col1=val1, ... WHERE ...` 構文（対象ラベルのレコードのデータ更新）
- ✅ SET句は複数カラムを同時指定可能。WHERE句は省略可（省略時は対象ラベル内の全レコードを更新）。SET対象外のカラム・ラベルは変更されない
- ✅ `UPDATE LABEL label.old SET label.new`（ラベル名そのもののリネーム、対象レコードのデータは変更しない）。**リネーム先のラベル名が既にDB内に存在する場合はラベル名の重複となるため、何も変更せずエラーを返す**（自分自身への同名リネームも同様にエラー）
- ✅ `UPDATE LABEL label.old SET label.new WHERE ...`（`WHERE`句で対象を絞り込み。省略時は`old`が付いた全レコードが一括で対象になる）
- ✅ 独自拡張 SQL `DELETE FROM label.xxx WHERE ...` 構文（対象ラベルのレコードをデータごと完全に削除）。`WHERE`句は省略可（省略時は対象ラベル内の全レコードを一括削除）
- ✅ `DELETE FROM label.*`（`WHERE`と組み合わせ可能。省略時はDB全体の全レコードを削除）
- ✅ `DELETE LABEL FROM label.xxx WHERE ...`（レコードそのものは削除せず、指定ラベルのみを対象レコードから外す。`WHERE`省略時は`xxx`が付いた全レコードが一括で対象になる）

### dynamic_label_management
- ✅ ラベルの AND / OR 検索
- ✅ ラベルのリネーム（DB 全体一括。対象レコードが元々リネーム先と同名のラベルも持っていた場合は重複付与エラーを避けるため付け直しをスキップ）
- ✅ ラベルのコピー・差分・グルーピング（コピー先が既に持つラベルは重複付与エラーを避けるためスキップし、新規コピー数のみを返す）
- ✅ ラベル統計情報（件数降順）
- ✅ 同一レコードへのラベル重複付与を禁止（`add_label` は既に付与済みのラベルに対してエラーを返す）

### db_ffi
- ✅ C ABI 互换の共有ライブラリ（`libdb_ffi.so`）
- ✅ Python `ctypes` ラッパークラス（`DbEngine`）
- ✅ JSON 文字列による全データ型対応

---

## プロジェクト構成

```
pudb-project/
├── Cargo.toml
├── README.md
├── db_engine/                          # コアエンジン (v1.6.0)
│   └── src/
│       ├── lib.rs                      # Database / Record / DataType / 永続化
│       ├── codec.rs                    # バイナリシリアライザ
│       ├── crypto.rs                   # XChaCha20-Poly1305 AEAD（純Rust）
│       ├── kdb_store.rs                # .kdb ファイル管理（WAL方式）
│       └── tests.rs                    # ユニットテスト（38件）
├── db_client/                          # REST API サーバー (v1.6.0)
│   ├── src/
│   │   ├── main.rs                     # エントリーポイント（KDB/JSON 自動判別）
│   │   ├── app.rs                      # Router 定義
│   │   ├── handlers.rs                 # レコード CRUD ハンドラー
│   │   ├── label_handlers.rs           # ラベル管理ハンドラー
│   │   ├── info_handlers.rs            # DB 情報 API ハンドラー
│   │   ├── sql_handlers.rs             # SQL 実行ハンドラー（SELECT / INSERT / UPDATE / DELETE）
│   │   ├── settings_handlers.rs        # 設定 API ハンドラー（HTTPリクエスト受付オンオフ）
│   │   ├── models.rs                   # JSON DTO
│   │   └── ui.html                     # 管理画面（HTML/CSS/JS）
│   └── tests/
│       ├── test_records.rs             # レコード API テスト（12件）
│       ├── test_labels.rs              # ラベル API テスト（12件）
│       ├── test_persistence.rs         # 永続化テスト（4件）
│       ├── test_sql.rs                 # SQL API テスト（23件）
│       └── test_settings.rs            # 設定 API テスト（4件）
├── sql_engine/                         # SQL SELECT/INSERT/UPDATE/DELETE エンジン (v1.6.0)
│   └── src/
│       ├── ast.rs / parser.rs / executor.rs
│       └── lib.rs                      # 公開 API（run_select・run_insert・run_update・run_delete）
├── dynamic_label_management/           # ラベル管理ライブラリ (v1.6.0)
│   └── src/lib.rs + tests.rs           # ユニットテスト（24件）
├── db_ffi/                             # C FFI バインディング (v1.6.0)
│   └── src/lib.rs                      # FFI 関数（13本）＋テスト（10件）
└── examples/python/
    ├── db_engine.py                    # Python ctypes ラッパー
    ├── demo.py                         # デモスクリプト
    └── generate_testdata.py            # 10,000件テストデータ自動生成スクリプト
```

---

## クイックスタート

### 必要環境

- Rust 1.80 以上（edition 2024）
- cargo

### ビルド & 起動

```bash
# KDB モード（推奨・デフォルト）
cargo run -p db_client
# → http://localhost:3000 で起動
# → db_data.kdb に暗号化して保存

# ポート・ファイルパスを指定
DB_ADDR=0.0.0.0:8080 DB_FILE=mydata.kdb cargo run -p db_client

# JSON モード（後方互换・暗号化なし）
DB_FILE=mydata.json cargo run -p db_client

# リリースビルド
cargo build --release -p db_client && ./target/release/db_client
```

### 動作確認

```bash
# バージョン・状態確認
curl http://localhost:3000/db/info

# SQL でレコード作成（既定の状態でそのまま使える）
curl -X POST http://localhost:3000/sql \
  -H 'Content-Type: application/json' \
  -d "{\"query\": \"INSERT INTO (label.employee) (name, age) VALUE ('田中', 35)\"}"

# SQL で検索
curl -X POST http://localhost:3000/sql \
  -H 'Content-Type: application/json' \
  -d '{"query": "SELECT * FROM label.employee WHERE age > 30"}'

# HTTPリクエスト(REST API)は既定で無効。大量テストデータ投入などで使う場合は先に有効化する
curl -X PUT http://localhost:3000/settings \
  -H 'Content-Type: application/json' -d '{"http_api_enabled": true}'

curl -X POST http://localhost:3000/records \
  -H 'Content-Type: application/json' \
  -d '{"columns":{"name":{"type":"text","value":"田中"},"age":{"type":"integer","value":35}},"labels":["employee"]}'

# Web UI（Records一覧はSQL経由のため、HTTPリクエストが無効でも閲覧可能）
open http://localhost:3000/ui
```

---

## Linux へのインストール（systemd連携）

一般的な Linux パッケージ（nginx・PostgreSQL 等）と同様に、専用の非rootシステムユーザーで
動作する systemd サービスとして導入できます。`systemctl start/stop/restart/enable` による
プロセス管理、`journalctl` によるログ確認、FHS準拠のディレクトリ配置に対応しています。

導入方法は3種類用意しています（詳細は [packaging/README.md](packaging/README.md)）。

| 方法 | コマンド | 備考 |
|------|---------|------|
| インストールスクリプト（推奨） | `sudo ./packaging/scripts/install.sh` | 追加ツール不要。ソースから即ビルド＆導入 |
| .deb パッケージ | `cargo deb -p db_client --no-build` → `dpkg -i` | Debian/Ubuntu系。要 `cargo-deb` |
| .rpm パッケージ | `cargo generate-rpm -p db_client` → `rpm -i` | RHEL/Fedora/Amazon Linux系。要 `cargo-generate-rpm` |

### 配置内容

| 項目 | パス |
|------|------|
| バイナリ | `/usr/bin/kagura-db` |
| systemdユニット | `kagura-db.service` |
| 環境設定ファイル | `/etc/kagura-db/kagura.env`（`DB_ADDR` / `DB_FILE` / `KAGURA_MASTER_KEY` / `KAGURA_LOG_FILE`） |
| データディレクトリ | `/var/lib/kagura-db` |
| ログディレクトリ | `/var/log/kagura-db` |
| 実行ユーザー | `kagura`（システムアカウント・ログインシェルなし） |

`KAGURA_MASTER_KEY`（KDB暗号化マスターキー）はインストール時に自動でランダム生成され、
`/etc/kagura-db/kagura.env` に `root:kagura` 640 権限で保存されます（既存ファイルがある場合は上書きしません）。

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

導入方法は2種類用意しています（詳細は [packaging/README.md](packaging/README.md)）。

| 方法 | コマンド | 備考 |
|------|---------|------|
| インストールスクリプト（推奨） | `powershell -ExecutionPolicy Bypass -File .\packaging\windows\install.ps1` | 追加ツール不要。ソースから即ビルド＆導入。管理者権限のPowerShellで実行 |
| `.exe` インストーラー | `iscc packaging\windows\installer.iss` → 生成された `KaguraDB-Setup-*.exe` を実行 | GUIウィザード形式。要 [Inno Setup](https://jrsoftware.org/isinfo.php)。アンインストーラーは自動生成され「プログラムと機能」に登録される |

### 配置内容

| 項目 | パス |
|------|------|
| バイナリ | `%ProgramFiles%\KaguraDB\kagura-db.exe` |
| 常駐方式 | タスクスケジューラ タスク `KaguraDB`（PC起動時に自動実行、異常終了時は自動再起動） |
| 環境設定ファイル | `%ProgramData%\KaguraDB\config\kagura.env`（`DB_ADDR` / `DB_FILE` / `KAGURA_MASTER_KEY` / `KAGURA_LOG_FILE`） |
| データディレクトリ | `%ProgramData%\KaguraDB\data` |
| ログディレクトリ | `%ProgramData%\KaguraDB\logs` |

`KAGURA_MASTER_KEY`（KDB暗号化マスターキー）はインストール時に自動でランダム生成され、
`kagura.env` に保存されます（既存ファイルがある場合は上書きしません）。

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

---

## KDB ストレージフォーマット

`.kdb` 拡張子のファイルは KAGURA DB 独自の暗号化バイナリ形式です。

### ファイル構造

```
┌──────────────────────────────────────────────────┐
│  HEADER  (64 bytes, 平文)                         │
│  magic[4]="KGDB"  version[2]  flags[2]           │
│  salt[16]  next_id[8]  record_count[8]            │
│  wal_entries[8]  reserved[16]                     │
├──────────────────────────────────────────────────┤
│  WAL ENTRIES (暗号化・追記ログ)                   │
│  [entry_len: u32][nonce: 24B][ciphertext + MAC]   │
│  ↑ レコード1件ずつ繰り返し                        │
└──────────────────────────────────────────────────┘
```

### 暗号化仕様

| 項目 | 仕様 |
|------|------|
| 暗号アルゴリズム | XChaCha20-Poly1305 AEAD |
| 鍵長 | 256 bit（32 bytes） |
| Nonce | 192 bit（24 bytes）・毎回ランダム生成 |
| 認証タグ | Poly1305 MAC（16 bytes）・改ざん検知 |
| 鍵導出 | HChaCha20(master_key, file_salt) |
| Salt | ファイル作成時に `/dev/urandom` から生成（16 bytes） |
| 実装 | 外部クレート不使用・純 Rust 手実装 |

### INSERT 高速化（WAL 方式）

```
従来の JSON 方式: O(n) → 10,000件目: 約600秒超・平均 5 req/s
KDB WAL 方式: O(1) → 10,000件目でも ~1,200 req/s（10,000件を 8.1秒で完了）
```

### セキュリティ特性

- テキストエディタで開いても内容は読めない（バイナリ）
- `strings` コマンドで意味のある文字列が抽出されない
- ファイル先頭 4 bytes は `KGDB` マジックナンバーのみ平文
- Poly1305 MAC により 1 bit の改ざんも検知可能
- マスターキーを変えれば同一ファイルを別環境で開けない

---

## SQL 機能

`POST /sql` エンドポイントで SQL SELECT / INSERT / UPDATE / DELETE クエリを実行できます。
通常の SQL とは異なり、テーブル名の代わりに `label.xxx` でラベルを指定する独自拡張構文です。

### SELECT 構文

```sql
SELECT * FROM label.employee
SELECT * FROM label.*
SELECT * FROM label.employee AND label.manager
SELECT * FROM label.employee OR label.developer
SELECT * FROM label.employee WHERE name = '田中'
SELECT * FROM label.employee WHERE age > 30 AND city = 'Tokyo'
SELECT * FROM label.employee WHERE name LIKE '田%'
SELECT * FROM label.employee ORDER BY salary DESC LIMIT 10
```

### INSERT 構文

```sql
-- カラム順を明示（推奨）: ラベル指定の直後に (col1, col2, ...) を置く
-- （標準SQLの INSERT INTO table (col1, col2) VALUES (...) に準じた語順）
INSERT INTO (label.employee) (employee_name, employee_age, employee_department) VALUE ('田中', 24, 'developer')

-- 複数ラベルを同時に付与
INSERT INTO (label.employee, label.manager) (employee_name, employee_age, employee_department) VALUE ('田中', 24, 'developer')

-- カラム順省略時: DB内の既存カラムをソート順に対応付ける（1件も既存カラムが無い場合はエラー）
INSERT INTO (label.employee) VALUE ('田中', 24, 'developer')

-- ラベル名は 'label.xxx' の文字列リテラル形式でも指定可能
INSERT INTO ('label.employee') (employee_name, employee_age, employee_department) VALUE ('田中', 24, 'developer')
```

補足:
- `INTO` の後のラベル名は必須で、`label.*` や空文字列は指定できない（無ラベル・全ラベル一括付与は不可）。
- 指定したラベルが未作成の場合、INSERT 実行時に自動作成される。
- `id` は自動採番のみで、INSERT 文からは指定できない。
- 同一 INSERT 文内で同じラベルを複数回指定した場合（例: `INTO (label.a, label.a)`）はラベル名の重複としてエラーになる。

### UPDATE 構文

```sql
-- データ更新: 対象ラベルのレコードのうち WHERE に一致する行の指定カラムを更新
UPDATE label.employee SET employee_name='木村' WHERE employee_name='木邑'

-- SET句は複数カラムを同時指定可能
UPDATE label.employee SET employee_age=30, employee_department='sales' WHERE employee_name='田中'

-- WHERE句は省略可能（省略時は対象ラベル内の全レコードが更新される）
UPDATE label.employee SET status='active'

-- ラベル名そのもののリネーム（WHERE省略時は old_label が付いた全レコードが一括対象。データ・他のラベルは変更しない）
UPDATE LABEL label.employee SET label.staff

-- WHEREで対象を絞り込み、一致したレコードだけラベルを付け替える（一括変更を避けたい場合）
UPDATE LABEL label.employee SET label.sales_staff WHERE department='sales'
```

補足:
- データ更新（`UPDATE label.xxx SET ...`）は SET句で指定したカラムのみを書き換える。それ以外の既存カラム・ラベルは維持される。
- `UPDATE LABEL label.old SET label.new [WHERE ...]` は `old` ラベルが付いているレコードのうち WHERE に一致するものだけ `old` を外し `new` を付け直す（レコードのカラムデータには影響しない）。`WHERE` を省略すると `old` が付いた全レコードが一括で対象になる。`label.*` は旧名・新名のどちらにも指定できない。
- 一致した行が無い場合もエラーにはならず、更新件数 0 として成功を返す。
- `UPDATE LABEL` のリネーム先ラベル名（`new`）が既にDB内の他のレコードで使われている場合は、WHEREでの絞り込みに関わらずラベル名の重複としてエラーを返し、何も変更しない。

### DELETE 構文

```sql
-- データ削除: 対象ラベルのレコードのうち WHERE に一致する行をデータごと完全に削除
DELETE FROM label.employee WHERE employee_name = '田中'

-- WHERE句を省略すると、対象ラベルが付いた全レコードが一括削除される
DELETE FROM label.employee

-- label.* を指定すると、DB全体の全レコードが対象になる（WHEREと組み合わせ可能）
DELETE FROM label.*

-- ラベルのみ削除: レコード自体・カラムデータ・他のラベルは削除せず、指定ラベルだけを対象レコードから外す
DELETE LABEL FROM label.employee WHERE employee_department = 'sales'

-- WHERE省略時は、そのラベルが付いた全レコードから一括でラベルを外す
DELETE LABEL FROM label.employee
```

補足:
- `DELETE FROM label.xxx [WHERE ...]` は対象ラベルのレコードを `WHERE` に一致する分だけデータごと完全に削除する（`WHERE` 省略時は対象ラベル内の全レコードを一括削除）。
- `DELETE FROM label.*` は全ラベル横断で対象を絞り込む（`WHERE` 省略時は DB 内の全レコードを削除する）。
- `DELETE LABEL FROM label.xxx [WHERE ...]` はレコードを削除せず、指定ラベルのみを対象レコードから外す（データ・他のラベルは変更しない）。`label.*` は指定できない。
- いずれも一致した行が無い場合はエラーにはならず、削除件数 0 として成功を返す。
- レスポンスの削除件数は `deleted_count` に格納される（`DELETE LABEL` の場合はラベルを外したレコード数）。

### リクエスト例

```bash
curl -X POST http://localhost:3000/sql \
  -H 'Content-Type: application/json' \
  -d '{"query": "SELECT employee_name, age, salary FROM label.employee WHERE age > 30 ORDER BY salary DESC LIMIT 10"}'

curl -X POST http://localhost:3000/sql \
  -H 'Content-Type: application/json' \
  -d "{\"query\": \"INSERT INTO (label.employee) (employee_name, employee_age, employee_department) VALUE ('田中', 24, 'developer')\"}"

curl -X POST http://localhost:3000/sql \
  -H 'Content-Type: application/json' \
  -d "{\"query\": \"UPDATE label.employee SET employee_name='木村' WHERE employee_name='木邑'\"}"

curl -X POST http://localhost:3000/sql \
  -H 'Content-Type: application/json' \
  -d "{\"query\": \"DELETE FROM label.employee WHERE employee_name='田中'\"}"
```

### 10,000件での性能測定

| クエリ | 応答時間 |
|--------|-------|
| `SELECT * FROM label.* LIMIT 100` | **9.7 ms** |
| `WHERE 等値検索` | **9.7 ms** |
| `WHERE LIKE` | **10.6 ms** |
| `WHERE 複合 AND` | **12.7 ms** |
| `FROM label.A AND label.B` | **30.9 ms** |
| `ORDER BY LIMIT 50` | **33.9 ms** |
| `WHERE 範囲検索` | **88.7 ms** |
| `SELECT * FROM label.employee`（2000件） | **154.8 ms** |
| `FROM label.A OR label.B`（3200件） | **252.9 ms** |
| `SELECT * FROM label.*`（全 10,000件） | **750.4 ms** |

---

## ログ機能

KAGURA DB の稼働ログを JSON Lines（1行1JSONオブジェクト）形式でファイルへ永続保存する。
保存先は環境変数 `KAGURA_LOG_FILE`（既定: `logs/kagura.log`）で変更できる。

### 記録される内容

| 項目 | 説明 |
|------|------|
| 起動・停止時刻 | サーバー起動時／停止時に `category: "lifecycle"` で記録 |
| 停止理由 | `stop_reason` フィールドで `normal`（SIGINT/SIGTERM による正常終了）・`error`（設定不備やバインド失敗などアプリケーション上のエラー）・`hardware`（ディスクI/Oエラー等、HW/ストレージ起因と判定できる異常）を区別 |
| SQLクエリ実行 | `category: "sql"` で成功/失敗（`success`）・実行時間（`duration_ms`）・エラー内容を記録 |
| HTTPリクエスト | `category: "http"` で全リクエストのメソッド・パス・ステータスコード・応答時間（`duration_ms`）を記録 |
| DB操作 | `category: "db"` でレコード/ラベルの作成・更新・削除、KDB/JSON保存の成功・失敗を記録 |
| HW/ストレージ異常 | `category: "hardware"` でディスクI/Oエラー（`EIO` `ENOSPC` `EROFS` `ENODEV` 等）を検知した際に記録 |

各エントリは共通で `timestamp`（RFC3339）・`level`（`info`/`warn`/`error`）・`category`・`message` を持つ。

### ログの閲覧

```bash
# 直近200件（既定）を新しい順に取得
curl http://localhost:3000/logs

# 件数を指定（上限2000）
curl "http://localhost:3000/logs?lines=50"
```

Web UI の「📜 ログ」ビューからも、カテゴリで絞り込みながら閲覧できる。

### 停止理由の判定について

「HW問題による停止」は、OSが返す I/O エラーコード（`EIO`=入出力エラー、`ENOSPC`=ディスク容量不足、`EROFS`=読み取り専用ファイルシステム等）をもとに分類する。物理的な故障そのものを検知しているわけではなく、OSレベルで観測できるストレージ関連のエラーを手がかりにした分類である点に留意する。

---

## API リファレンス

### エンドポイント一覧

HTTPリクエスト欄が「要HTTP API」の行は、既定では無効な `/records` `/labels` 系 REST API に属し、
`PUT /settings` で `http_api_enabled: true` にするまで `403 Forbidden` を返す（詳細は [設定](#web-管理ui) を参照）。

| メソッド | パス | 説明 | 要HTTP API |
|---------|------|------|:---:|
| `GET`    | `/ui` | Web 管理 UI | - |
| `GET`    | `/records` | 全レコード取得 | ✅ |
| `POST`   | `/records` | レコード作成 | ✅ |
| `GET`    | `/records/{id}` | ID で取得 | ✅ |
| `PUT`    | `/records/{id}` | レコード更新 | ✅ |
| `DELETE` | `/records/{id}` | レコード削除 | ✅ |
| `GET`    | `/records/label/{label}` | ラベルで絞り込み取得 | ✅ |
| `GET`    | `/records/{id}/labels` | レコードのラベル一覧 | ✅ |
| `POST`   | `/records/{id}/labels` | ラベル追加 | ✅ |
| `DELETE` | `/records/{id}/labels/{label}` | ラベル削除 | ✅ |
| `GET`    | `/labels` | 全ラベル一覧+統計 | ✅ |
| `POST`   | `/labels/search` | AND/OR ラベル検索 | ✅ |
| `PUT`    | `/labels/rename` | ラベルリネーム | ✅ |
| `GET`    | `/db/info` | DB バージョン・統計情報 | - |
| `POST`   | `/sql` | SQL SELECT / INSERT / UPDATE / DELETE 実行 | - |
| `GET`    | `/settings` | 現在の設定取得 | - |
| `PUT`    | `/settings` | 設定更新（HTTPリクエスト受付オンオフ） | - |
| `GET`    | `/logs` | 保管されたログを新しい順に取得（`?lines=` で件数指定、既定200・上限2000） | - |

### GET /db/info レスポンス例

```json
{
  "engine_name":    "KAGURA DB Engine",
  "app_version":    "2.2.0",
  "storage_mode":   "kdb",
  "storage_format": "KDB Binary (WAL + XChaCha20-Poly1305 encrypted)",
  "encrypted":      true,
  "db_file_path":   "db_data.kdb",
  "record_count":   10000,
  "label_count":    87,
  "column_count":   68,
  "next_id":        10001
}
```

---

## Web 管理UI

`http://localhost:3000/ui` でブラウザから DB を操作できます。

| ビュー | 説明 |
|--------|------|
| **📋 Records** | レコード一覧（`POST /sql` の `SELECT * FROM label.*` 経由で取得。HTTPリクエストが無効でも閲覧可能）・検索・ラベルフィルター・自動更新（10秒）。作成・編集・削除は REST API（`/records`）を使うため、これらの操作には設定で HTTPリクエストを有効化する必要がある |
| **ℹ️ DB Info** | バージョン・ストレージモード・暗号化状態・統計カード・カラム一覧 |
| **🔍 SQL Query** | SQL SELECT / INSERT / UPDATE / DELETE 実行・テーブル形式結果表示・ Ctrl+Enter 対応 |
| **⚙️ 設定** | HTTPリクエスト（`/records` `/labels` 系 REST API）受付のオンオフを切替（既定は無効）。大量テストデータ投入など HTTP 経由の操作が必要なときに有効化する |
| **📜 ログ** | 保管されたログ（起動/停止・SQL・HTTP・DB操作・HW異常）をカテゴリで絞り込みながら一覧表示・自動更新（10秒） |

---

## テストデータ生成

```bash
# 10,000件生成 + 性能テスト
python3 examples/python/generate_testdata.py --bench

# 性能テストのみ
python3 examples/python/generate_testdata.py --bench-only --repeat 5
```

### 事前生成済みJSONの投入（再インストール後のデータ再投入など）

`examples/testdata/kagura_testdata_10000.json` に、`POST /records` へそのまま投げられる
形式（`{"columns": {...}, "labels": [...]}` の配列）で10,000件のテストデータを同梱しています。
インストールし直した直後など、ランダム生成をやり直さずに同じデータを再投入したい場合に使えます。

```bash
# HTTPリクエスト(REST API)を有効化してから投入（既定で無効のため）
curl -X PUT http://localhost:3000/settings -H 'Content-Type: application/json' -d '{"http_api_enabled": true}'

# 同梱JSONを読み込んで投入
python3 examples/python/generate_testdata.py \
  --load-json examples/testdata/kagura_testdata_10000.json \
  --url http://localhost:3000

# 新しいJSONを作り直したい場合（サーバー接続不要）
python3 examples/python/generate_testdata.py --dump-json testdata_10000.json --count 10000
```

**依存**: Python 3.6+ 標準ライブラリのみ（pip 不要）

---

## Python バインディング

```bash
cargo build --release -p db_ffi
cd examples/python && python3 demo.py
```

```python
from db_engine import DbEngine
db = DbEngine()
id = db.insert({"name": "Alice", "age": 30}, labels=["dept:engineering"])
print(db.get(id))
db.save("/tmp/kagura.json")
```

---

## テスト

```bash
cargo test --workspace
```

| クレート | テスト数 | 内容 |
|----------|----------|---------|
| `db_engine` | 39 | CRUD・ラベル操作（重複付与エラーを含む）・KDB暗号化・バイナリコーデック |
| `dynamic_label_management` | 24 | AND/OR 検索・リネーム・差分・ラベル重複付与エラー |
| `db_ffi` | 10 | FFI 関数・メモリ管理 |
| `db_client`（ユニットテスト） | 5 | ログ出力保管機能（JSON Lines 書き込み/読み出し・HW起因エラー判定） |
| `db_client`（統合テスト） | 59 | HTTP API・ラベル操作（重複付与エラーを含む）・永続化・SQL SELECT/INSERT/UPDATE/DELETE（ラベル重複エラー・UPDATE LABELのWHERE絞り込み・DELETE/DELETE LABELのWHERE絞り込みを含む）・設定（HTTPリクエスト受付オンオフ）・ログ（`GET /logs`） |
| `sql_engine` | 60 | パーサー・実行エンジン（SELECT・INSERT・UPDATE・DELETE・WHERE・ORDER BY・LIKE・ラベル重複エラー・UPDATE LABEL/DELETE LABELのWHERE絞り込み） |
| **合計** | **197** | |

---

## 技術スタック

| カテゴリ | 技術 |
|----------|---------|
| 言語 | Rust 2024 edition |
| HTTP フレームワーク | [axum](https://github.com/tokio-rs/axum) 0.8 |
| 非同期ランタイム | [tokio](https://tokio.rs/) 1.x |
| シリアライゼーション | [serde](https://serde.rs/) + serde_json |
| 暗号化 | XChaCha20-Poly1305 AEAD（純 Rust 手実装・外部クレートなし） |
| SQL パーサー | 手書き再帰下降パーサー（外部ライブラリなし） |
| FFI | Rust `extern "C"` + Python `ctypes` |
| フロントエンド | バニラ HTML / CSS / JavaScript（外部依存なし） |
| テスト（HTTP） | [reqwest](https://github.com/seanmonstar/reqwest) 0.12 |
| テストデータ生成 | Python 3.6+ 標準ライブラリのみ |
| ログ | JSON Lines 形式でファイル保存（[chrono](https://github.com/chronotope/chrono) でタイムスタンプ生成） |

---

## バージョン履歴

バージョン表記: **A.B.C**

| 区分 | ルール |
|------|-------|
| **A（メジャー）** | 破壊的変更・旧バージョンとの非互換が生じたとき +1 |
| **B（マイナー）** | 破壊的変更を伴わない新機能追加のとき +1 |
| **C（ビルド）** | 規模に関わらず何らかの変更・修正を加えたとき +1 |

| バージョン | 主な変更内容 |
|-----------|-------------|
| **2.5.1** | Web UI「Records」画面の不具合修正。`SELECT * FROM label.*` で取得した全レコードを1つのHTML文字列に連結してから描画していたため、数万〜10万件規模のデータでは連結後の文字列がJavaScriptの文字列長上限を超え `RangeError: Invalid string length` が発生していた。この例外が `loadAll()` の汎用catchに捕捉され、実際はサーバーへの通信自体は成功しているにもかかわらず「Connection failed」と誤表示される（サーバーダウンしたかのように見える）事象があったため、Records一覧の描画件数に上限（1,000件）を設け、超過時は超過件数と絞り込み方法を通知する行を表示するよう変更。あわせて `loadAll()` の catch 側も、原因を問わず固定文言を出すのをやめ、実際のエラーメッセージを表示するように修正。また、2.4.0以降 `db_client`／`db_engine`／`db_ffi`／`dynamic_label_management`／`sql_engine` の `Cargo.toml` の `version` が `2.4.0` のまま更新されておらず、DB Info画面が参照する `CARGO_PKG_VERSION` がREADME上のバージョン表記（2.5.0）と食い違っていた（再ビルドしてもDB Infoの表示が古いまま変わらない不具合）ため、全クレートのバージョンをREADMEと同期させた |
| **2.5.0** | Windows 向けインストール機能を新規追加。Windows のサービス制御マネージャーへの正規登録には実行ファイル側の対応が必要なため、代わりにタスクスケジューラ（PC起動時に自動実行、異常終了時は自動再起動）で常駐運用する方式を採用。導入方法として (1) 追加ツール不要の `packaging/windows/install.ps1`／`uninstall.ps1`（ソースビルド＋`%ProgramFiles%\KaguraDB`・`%ProgramData%\KaguraDB`への配置＋タスク登録）、(2) [Inno Setup](https://jrsoftware.org/isinfo.php) による `.exe` インストーラー（`packaging/windows/installer.iss`。アンインストーラーは自動生成され「プログラムと機能」に登録される）の2種を用意。`KAGURA_MASTER_KEY` はインストール時にランダム生成し `%ProgramData%\KaguraDB\config\kagura.env` へ保存する運用とし、Linux版と同様の挙動（既存ファイルは上書きしない）とした |
| **2.4.0** | `examples/python/generate_testdata.py` に `--dump-json PATH`（サーバー接続不要でPOST /records用のJSON配列をファイルへ書き出し）と `--load-json PATH`（ランダム生成せずファイルから読み込んで投入）を追加。インストールし直した後などにランダム再生成せず同じテストデータを再投入できるように、事前生成済みの10,000件JSON配列 `examples/testdata/kagura_testdata_10000.json` を同梱（各要素は `{"columns": {...}, "labels": [...]}` の形式で `POST /records` にそのまま投入可能） |
| **2.3.0** | Linux 向けインストール機能を新規追加。一般的な Linux パッケージ（nginx・PostgreSQL 等）と同様に、専用の非rootシステムユーザー（`kagura`）で動作する systemd サービスとして導入可能に。`systemctl start/stop/restart/enable` によるプロセス管理、`journalctl` によるログ確認、FHS準拠のディレクトリ配置（`/usr/bin`・`/etc/kagura-db`・`/var/lib/kagura-db`・`/var/log/kagura-db`）に対応。導入方法として (1) 追加ツール不要の `packaging/scripts/install.sh`／`uninstall.sh`、(2) `cargo-deb` による `.deb` パッケージ、(3) `cargo-generate-rpm` による `.rpm` パッケージの3種を用意。`KAGURA_MASTER_KEY`（KDB暗号化マスターキー）はインストール時にランダム生成し `/etc/kagura-db/kagura.env` に600番台権限で保存するようにし、既定の固定キーへのフォールバックに頼らない運用を可能にした |
| **2.2.0** | SQL `DELETE` 文を新規追加。`DELETE FROM label.xxx [WHERE ...]`（対象ラベルのレコードをデータごと完全に削除。`WHERE`省略時は対象ラベル内の全レコードを一括削除、`label.*`でDB全体を対象化可能）と、`DELETE LABEL FROM label.xxx [WHERE ...]`（レコード自体は削除せず指定ラベルのみを対象レコードから外す）の2構文に対応。`POST /sql` が DELETE 文を受け付けるようになり、レスポンスに削除件数 `deleted_count` を追加 |
| **2.1.0** | SQL `UPDATE LABEL label.old SET label.new` に `WHERE` 句対応を追加。従来は `old` ラベルが付いた全レコードが常に一括でリネームされていたが、`WHERE` で条件を指定すると一致したレコードだけを対象にでき、一括変更を避けられるようになった（`WHERE` 省略時は従来通り全レコードが対象） |
| **2.0.0** | **【破壊的変更】** ラベルの重複付与を禁止。従来 `db_engine::Database::attach_label`（および `dynamic_label_management::LabelManager::add_label`）は既に付与済みのラベルを再付与しても何もせず成功していた（冪等）が、変更せず `DuplicateLabel` エラーを返す仕様に変更。影響範囲: (1) `POST /records/{id}/labels` は重複時に `409 Conflict` を返すようになる（以前は `200 OK`）。(2) SQL `INSERT INTO (label.a, label.a) ...` のように同一文内で同じラベルを複数指定した場合はエラーになる。(3) SQL `UPDATE LABEL label.old SET label.new` はリネーム先 `new` が既にDB内の他レコードで使われている場合エラーとし、何も変更しない（自己リネームも同様）。副作用として `dynamic_label_management` の `copy_labels`／`rename_label` は、コピー先／対象レコードが既に同名ラベルを持つ場合は重複エラーを避けるためスキップするよう防御的に修正 |
| **1.8.0** | SQL `UPDATE` 機能追加。`UPDATE label.xxx SET col=val, ... WHERE ...` によるデータ更新（SET対象外のカラム・ラベルは維持）と、`UPDATE LABEL label.old SET label.new` によるラベル名リネーム（DB全体一括）の2構文に対応。`POST /sql` が UPDATE 文を受け付けるようになり、レスポンスに更新件数 `updated_count` を追加 |
| **1.7.1** | SQL `INSERT` のパフォーマンス改善。従来は書き込みのたびに全レコードを読み直して再暗号化・全件書き直す方式（コンパクション）だったため件数に比例して遅くなっていたが、REST `/records` と同じ WAL 追記方式（`insert_fast`）に統一し O(1) 化。検証では約32,000件時点で 189ms → 0.4ms（約470倍）高速化 |
| **1.7.0** | ログ出力保管機能を追加。起動/停止時刻・停止理由（正常/エラー/HW異常）・SQL/HTTPリクエストの成功失敗・Webクライアントの応答時間・DB操作を JSON Lines 形式でファイルへ永続保存。`GET /logs` エンドポイントと Web UI「ログ」ビューを追加 |
| **1.6.0** | Web UI に「設定」ビューを追加し `GET`/`PUT /settings` で HTTPリクエスト（`/records` `/labels` 系 REST API）受付のオンオフを切替可能に（**既定を無効化**）。Web UI の Records 一覧を REST から SQL（`SELECT * FROM label.*`）経由の取得に変更し、REST API 無効時も閲覧可能に。`SELECT *` の結果に `labels` 列（カンマ区切り）を追加 |
| **1.5.0** | SQL INSERT 機能追加（`INSERT INTO (label.xxx) (col, ...) VALUE (...)`、複数ラベル同時付与、ラベル自動作成）、`POST /sql` の INSERT 対応 |
| **1.4.1** | DB Info UI のバージョン・ストレージ表示修正、全クレートバージョン統一 |
| **1.4.0** | KDB 暗号化バイナリ形式（XChaCha20-Poly1305）、WAL 高速 INSERT、JSON 後方互换 |
| **1.3.0** | テストデータ自動生成スクリプト（`generate_testdata.py`）、SQL 性能テスト |
| **1.2.0** | SQL SELECT エンジン（`sql_engine` クレート新設）、`POST /sql` エンドポイント |
| **1.1.0** | DB Info API（`GET /db/info`）、Web UI に DB Info ビュー・SQL Query ビュー追加 |
| **1.0.0** | Web 管理 UI（Records ビュー）、ラベル管理 API 拡充 |
| **0.1.0** | 初期実装（db_engine・db_client・dynamic_label_management・db_ffi） |

---

## ライセンス

商用・非商用問わずソースコードを使ってのサードパーティソフトの公開は禁止  
KAGURA DBを利用したアプリケーション等の公開は可能
