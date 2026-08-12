# 🗄️ KAGURA DB

> Rust で一から実装したオリジナルのデータベースエンジン。  
> 列指向ストレージ・ラベル検索・KDB暗号化バイナリ形式・SQL SELECT・REST API・Web管理UI を備えたフルスタックな DB システムです。  
> バージョンの情報については以下の補足を確認ください。  
> 補足  
> ・メジャーバージョンが異なると互換性は無くなります。  
> ・メジャーバージョンが一致し、マイナーバージョンだけが異なる場合問題なく移行ができ互換性を保ちます。

**バージョン: `1.4.1`**

| 区分 | 説明 |
|------|------|
| **A = 1** (メジャー) | KDB 暗号化バイナリ形式導入による旧 JSON 形式との破壊的変更 |
| **B = 4** (マイナー) | Web管理UI・DB Info API・SQL SELECT・テストデータ生成 の 4 機能追加 |
| **C = 1** (ビルド) | バージョン情報・UI表示の修正 |

---

## 📋 目次

- [概要](#概要)
- [アーキテクチャ](#アーキテクチャ)
- [機能一覧](#機能一覧)
- [プロジェクト構成](#プロジェクト構成)
- [クイックスタート](#クイックスタート)
- [KDB ストレージフォーマット](#kdb-ストレージフォーマット)
- [SQL 機能](#sql-機能)
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
| **SQL SELECT** | `FROM label.xxx` 構文による独自拡張 SQL クエリ |
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
    |   sql_handlers / app / models / ui                     |
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
    |              sql_engine  (SQL SELECT エンジン)           |
    |   ast / parser / executor                               |
    |   手書き再帰下降パーサー（外部ライブラリ不使用）         |
    +----------------------------------------------------------+
```

---

## 機能一覧

### db_engine（コアエンジン）
- ✅ CRUD（Insert / Get / Update / Delete）
- ✅ 列指向ストレージ（`ColumnStore`）
- ✅ ラベル付与・検索・削除（attach / detach）
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
- ✅ 15 本のエンドポイント（レコード CRUD・ラベル管理・DB 情報・SQL・Web UI）
- ✅ KDB モード（`.kdb`）と JSON モードの自動判別・後方互换
- ✅ 起動時自動ロード・書き込み時自動セーブ
- ✅ 環境変数による設定（`DB_FILE` / `DB_ADDR` / `KAGURA_MASTER_KEY`）
- ✅ ブラウザ Web 管理 UI（Records / DB Info / SQL Query の 3 ビュー）
- ✅ `GET /db/info` でバージョン・ストレージ状態・統計を取得
- ✅ `POST /sql` で SQL SELECT クエリを実行

### sql_engine（SQL SELECT エンジン）
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

### dynamic_label_management
- ✅ ラベルの AND / OR 検索
- ✅ ラベルのリネーム（DB 全体一括）
- ✅ ラベルのコピー・差分・グルーピング
- ✅ ラベル統計情報（件数降順）

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
├── db_engine/                          # コアエンジン (v1.4.1)
│   └── src/
│       ├── lib.rs                      # Database / Record / DataType / 永続化
│       ├── codec.rs                    # バイナリシリアライザ
│       ├── crypto.rs                   # XChaCha20-Poly1305 AEAD（純Rust）
│       ├── kdb_store.rs                # .kdb ファイル管理（WAL方式）
│       └── tests.rs                    # ユニットテスト（38件）
├── db_client/                          # REST API サーバー (v1.4.1)
│   ├── src/
│   │   ├── main.rs                     # エントリーポイント（KDB/JSON 自動判別）
│   │   ├── app.rs                      # Router 定義
│   │   ├── handlers.rs                 # レコード CRUD ハンドラー
│   │   ├── label_handlers.rs           # ラベル管理ハンドラー
│   │   ├── info_handlers.rs            # DB 情報 API ハンドラー
│   │   ├── sql_handlers.rs             # SQL 実行ハンドラー
│   │   ├── models.rs                   # JSON DTO
│   │   └── ui.html                     # 管理画面（HTML/CSS/JS）
│   └── tests/
│       ├── test_records.rs             # レコード API テスト（12件）
│       ├── test_labels.rs              # ラベル API テスト（12件）
│       └── test_persistence.rs         # 永続化テスト（4件）
├── sql_engine/                         # SQL SELECT エンジン (v1.4.1)
│   └── src/
│       ├── ast.rs / parser.rs / executor.rs
│       └── lib.rs                      # 公開 API（run_select）
├── dynamic_label_management/           # ラベル管理ライブラリ (v1.4.1)
│   └── src/lib.rs + tests.rs           # ユニットテスト（24件）
├── db_ffi/                             # C FFI バインディング (v1.4.1)
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

# レコード作成
curl -X POST http://localhost:3000/records \
  -H 'Content-Type: application/json' \
  -d '{"columns":{"name":{"type":"text","value":"田中"},"age":{"type":"integer","value":35}},"labels":["employee"]}'

# SQL で検索
curl -X POST http://localhost:3000/sql \
  -H 'Content-Type: application/json' \
  -d '{"query": "SELECT * FROM label.employee WHERE age > 30"}'

# Web UI
open http://localhost:3000/ui
```

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

`POST /sql` エンドポイントで SQL SELECT クエリを実行できます。

### 構文

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

### リクエスト例

```bash
curl -X POST http://localhost:3000/sql \
  -H 'Content-Type: application/json' \
  -d '{"query": "SELECT employee_name, age, salary FROM label.employee WHERE age > 30 ORDER BY salary DESC LIMIT 10"}'
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

## API リファレンス

### エンドポイント一覧

| メソッド | パス | 説明 |
|---------|------|------|
| `GET`    | `/ui` | Web 管理 UI |
| `GET`    | `/records` | 全レコード取得 |
| `POST`   | `/records` | レコード作成 |
| `GET`    | `/records/{id}` | ID で取得 |
| `PUT`    | `/records/{id}` | レコード更新 |
| `DELETE` | `/records/{id}` | レコード削除 |
| `GET`    | `/records/label/{label}` | ラベルで絞り込み取得 |
| `GET`    | `/records/{id}/labels` | レコードのラベル一覧 |
| `POST`   | `/records/{id}/labels` | ラベル追加 |
| `DELETE` | `/records/{id}/labels/{label}` | ラベル削除 |
| `GET`    | `/labels` | 全ラベル一覧+統計 |
| `POST`   | `/labels/search` | AND/OR ラベル検索 |
| `PUT`    | `/labels/rename` | ラベルリネーム |
| `GET`    | `/db/info` | DB バージョン・統計情報 |
| `POST`   | `/sql` | SQL SELECT 実行 |

### GET /db/info レスポンス例

```json
{
  "engine_name":    "KAGURA DB Engine",
  "app_version":    "1.4.1",
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
| **📋 Records** | レコードの一覧・検索・作成・編集・削除・ラベルフィルター・自動更新（10秒） |
| **ℹ️ DB Info** | バージョン・ストレージモード・暗号化状態・統計カード・カラム一覧 |
| **🔍 SQL Query** | SQL SELECT 実行・テーブル形式結果表示・ Ctrl+Enter 対応 |

---

## テストデータ生成

```bash
# 10,000件生成 + 性能テスト
python3 examples/python/generate_testdata.py --bench

# 性能テストのみ
python3 examples/python/generate_testdata.py --bench-only --repeat 5
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
| `db_engine` | 38 | CRUD・ラベル操作・KDB暗号化・バイナリコーデック |
| `dynamic_label_management` | 24 | AND/OR 検索・リネーム・差分 |
| `db_ffi` | 10 | FFI 関数・メモリ管理 |
| `db_client`（統合テスト） | 28 | HTTP API・ラベル操作・永続化 |
| `sql_engine` | 13 | パーサー（SELECT・WHERE・ORDER BY・LIKE） |
| **合計** | **113** | |

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
| **1.4.1** | DB Info UI のバージョン・ストレージ表示修正、全クレートバージョン統一 |
| **1.4.0** | KDB 暗号化バイナリ形式（XChaCha20-Poly1305）、WAL 高速 INSERT、JSON 後方互换 |
| **1.3.0** | テストデータ自動生成スクリプト（`generate_testdata.py`）、SQL 性能テスト |
| **1.2.0** | SQL SELECT エンジン（`sql_engine` クレート新設）、`POST /sql` エンドポイント |
| **1.1.0** | DB Info API（`GET /db/info`）、Web UI に DB Info ビュー・SQL Query ビュー追加 |
| **1.0.0** | Web 管理 UI（Records ビュー）、ラベル管理 API 拡充 |
| **0.1.0** | 初期実装（db_engine・db_client・dynamic_label_management・db_ffi） |

---

## ライセンス

MIT License
