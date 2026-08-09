# 🗄️ KAGURA DB

> Rust で実装したオリジナルのデータベースエンジン。
> 列指向ストレージ・ラベル検索・REST API・Web管理UI・Python FFI を備えたフルスタックなDBシステムです。

---

## 📋 目次

- [概要](#概要)
- [アーキテクチャ](#アーキテクチャ)
- [機能一覧](#機能一覧)
- [プロジェクト構成](#プロジェクト構成)
- [クイックスタート](#クイックスタート)
- [各クレートの詳細](#各クレートの詳細)
- [API リファレンス](#api-リファレンス)
- [Web 管理UI](#web-管理ui)
- [Python バインディング](#python-バインディング)
- [テスト](#テスト)
- [技術スタック](#技術スタック)

---

## 概要

KAGURA DB は Rust で一から実装したデータベースエンジンです。
リレーショナルDBとは異なる **列指向ストレージ** と **ラベル（タグ）ベースの検索** を特徴とします。

| 特徴 | 説明 |
|------|------|
| **列指向ストレージ** | カラムごとに `Vec<Option<DataType>>` で保持し、カラム追加が柔軟 |
| **ラベル検索** | `HashMap<label, Vec<row_index>>` によるO(1)のラベルインデックス |
| **スキーマレス** | レコードごとに任意のカラムを持てる |
| **JSON永続化** | アトミック書き込み（tmp→rename）でデータを保存 |
| **多言語対応** | REST API と C FFI（Python向け）の2種インターフェース |

---

## アーキテクチャ

```
ブラウザ (Web UI)   Python   curl / HTTP
      |              |           |
      +----+---------+           |
           | HTTP                | HTTP
    +------+---------------------+------+
    |   db_client (axum REST API + UI)  |
    |   dynamic_label_management        |
    +----------------+------------------+
                     | Rust クレート
    +----------------+------------------+
    |   db_engine - CRUD/ラベル/永続化  |
    +----------------+------------------+
                     | C FFI (.so)
    +----------------+------------------+
    |   db_ffi + examples/python        |
    +-----------------------------------+
```

---

## 機能一覧

### db_engine（コアエンジン）
- ✅ CRUD（Insert / Get / Update / Delete）
- ✅ 列指向ストレージ（`ColumnStore`）
- ✅ ラベル付与・検索・削除（attach / detach）
- ✅ カラム値による絞り込み検索
- ✅ JSON ファイルへの永続化（アトミック書き込み）
- ✅ 5種類のデータ型（Text / Integer / Float / Boolean / Null）

### db_client（REST API サーバー）
- ✅ 12本のエンドポイント（レコードCRUD + ラベル管理）
- ✅ 起動時の自動ロード・書き込み時の自動セーブ
- ✅ 環境変数による設定（`DB_FILE` / `DB_ADDR`）
- ✅ ブラウザ Web 管理 UI（`/ui`）

### dynamic_label_management
- ✅ ラベルの AND / OR 検索
- ✅ ラベルのリネーム（DB 全体一括）
- ✅ ラベルのコピー・差分・グルーピング
- ✅ ラベル統計情報（件数降順）

### db_ffi
- ✅ C ABI 互換の共有ライブラリ（`libdb_ffi.so`）
- ✅ Python `ctypes` ラッパークラス（`DbEngine`）
- ✅ JSON 文字列による全データ型対応

## プロジェクト構成

```
kagura-db/
├── Cargo.toml                     # ワークスペース定義
├── README.md
├── db_engine/                     # コアエンジン
│   └── src/
│       ├── lib.rs                 # Database / Record / DataType / 永続化
│       ├── main.rs                # デモバイナリ
│       └── tests.rs               # ユニットテスト（34件）
├── db_client/                     # REST API サーバー
│   ├── src/
│   │   ├── app.rs                 # Router 定義
│   │   ├── handlers.rs            # レコード操作ハンドラー
│   │   ├── label_handlers.rs      # ラベル管理ハンドラー
│   │   ├── models.rs              # JSON DTO
│   │   ├── ui.rs                  # Web UI ハンドラー
│   │   └── ui.html                # 管理画面 HTML/CSS/JS
│   └── tests/
│       ├── test_records.rs        # レコード API テスト（12件）
│       ├── test_labels.rs         # ラベル API テスト（12件）
│       └── test_persistence.rs    # 永続化テスト（4件）
├── dynamic_label_management/      # ラベル管理ライブラリ
│   └── src/
│       ├── lib.rs                 # LabelManager
│       ├── error.rs               # LabelError
│       └── tests.rs               # ユニットテスト（24件）
├── db_ffi/                        # C FFI バインディング
│   └── src/
│       └── lib.rs                 # FFI 関数（13本）+ テスト（10件）
└── examples/
    └── python/
        ├── db_engine.py           # Python ラッパークラス DbEngine
        └── demo.py                # デモスクリプト
```

---

## クイックスタート

### 前提条件

- Rust 1.80 以上（`rustup` でインストール推奨）
- Python 3.9 以上（Python バインディングを使う場合）

### インストール

```bash
git clone https://github.com/your-username/kagura-db.git
cd kagura-db
```

### REST API サーバーの起動

```bash
# デフォルト: http://0.0.0.0:3000、保存先: db_data.json
cargo run -p db_client

# 任意指定
DB_FILE=/var/data/kagura.json DB_ADDR=0.0.0.0:8080 cargo run -p db_client
```

### Web 管理 UI

```
http://localhost:3000/ui
```

### 全テスト

```bash
cargo test --workspace
```

---

## 各クレートの詳細

### db_engine

```rust
use db_engine::{Database, DataType, Record};

let mut db = Database::new();
let mut record = Record::new(0);
record.set("name", DataType::Text("Alice".to_string()));
record.set("age",  DataType::Integer(30));
record.add_label("dept:engineering");

let id = db.insert(record).unwrap();           // id=1
let r  = db.get(id).unwrap();
let rs = db.get_by_label("dept:engineering");

db.save("kagura.json").unwrap();
let db2 = Database::load("kagura.json").unwrap();
```

### db_client（エンドポイント一覧）

| メソッド | パス | 説明 |
|----------|------|------|
| `GET` | `/records` | 全レコード一覧 |
| `GET` | `/records/{id}` | ID で取得 |
| `GET` | `/records/label/{label}` | ラベルで検索 |
| `POST` | `/records` | レコード作成 |
| `PUT` | `/records/{id}` | レコード更新 |
| `DELETE` | `/records/{id}` | レコード削除 |
| `GET` | `/records/{id}/labels` | ラベル一覧 |
| `POST` | `/records/{id}/labels` | ラベル追加 |
| `DELETE` | `/records/{id}/labels/{label}` | ラベル削除 |
| `GET` | `/labels` | 全ラベル一覧 + 統計 |
| `POST` | `/labels/search` | AND/OR 検索 |
| `PUT` | `/labels/rename` | ラベルリネーム |
| `GET` | `/ui` | Web 管理 UI |

### dynamic_label_management

```rust
use dynamic_label_management::LabelManager;

let mut mgr = LabelManager::new();
let ids = mgr.find_by_labels_all(&["role:backend", "role:senior"]); // AND
let ids = mgr.find_by_labels_any(&["dept:engineering", "dept:sales"]); // OR
let n   = mgr.rename_label("env:prod", "environment:production").unwrap();
for stat in mgr.label_stats() {
    println!("{}: {} 件", stat.label, stat.record_count);
}
```

### db_ffi（FFI 関数一覧）

| 関数 | 説明 |
|------|------|
| `db_new()` | Database 生成 |
| `db_free(ptr)` | Database 解放 |
| `db_string_free(ptr)` | Rust 文字列を解放 |
| `db_insert(ptr, json)` | レコード挿入 → ID |
| `db_get(ptr, id)` | ID で取得 → JSON |
| `db_delete(ptr, id)` | 削除 → 1=成功 |
| `db_update(ptr, id, json)` | 更新 → 1=成功 |
| `db_get_by_label(ptr, label)` | ラベル検索 → JSON配列 |
| `db_list_all(ptr)` | 全レコード → JSON配列 |
| `db_count(ptr)` | レコード数 |
| `db_save(ptr, path)` | ファイル保存 |
| `db_load(path)` | ファイルからロード |
| `db_list_labels(ptr)` | 全ラベル一覧 → JSON配列 |

---

## API リファレンス

### カラム値の JSON 形式

```json
{
  "columns": {
    "name":   {"type": "text",    "value": "Alice"},
    "age":    {"type": "integer", "value": 30},
    "score":  {"type": "float",   "value": 95.5},
    "active": {"type": "boolean", "value": true},
    "memo":   {"type": "null",    "value": null}
  },
  "labels": ["dept:engineering", "env:prod"]
}
```

### ラベル名の制約

使用可能: **英数字 / ハイフン(`-`) / コロン(`:`) / アンダースコア(`_`) / スラッシュ(`/`)**

```
✅ dept:engineering  env:prod  role-backend  v1/stable
❌ "dept engineering"  （スペース不可）
```

### リクエスト例

```bash
# レコード作成
curl -X POST http://localhost:3000/records \
  -H 'Content-Type: application/json' \
  -d '{"columns":{"name":{"type":"text","value":"Alice"}},"labels":["dept:engineering"]}'

# AND 検索
curl -X POST http://localhost:3000/labels/search \
  -H 'Content-Type: application/json' \
  -d '{"labels":["dept:engineering","env:prod"],"mode":"and"}'

# ラベルリネーム
curl -X PUT http://localhost:3000/labels/rename \
  -H 'Content-Type: application/json' \
  -d '{"old_label":"env:prod","new_label":"environment:production"}'
```

---

## Web 管理UI

`http://localhost:3000/ui` でブラウザから DB を管理できます。

| 機能 | 説明 |
|------|------|
| レコード一覧 | 全レコードをテーブル形式で表示 |
| ラベルフィルター | 左サイドバーからラベルで絞り込み |
| テキスト検索 | カラム値・ラベルをリアルタイム検索 |
| レコード作成 | カラム・型・値・ラベルを指定して新規作成 |
| レコード編集 | 既存レコードの値・ラベルを変更 |
| レコード削除 | 確認ダイアログ付きで削除 |
| 自動更新 | 10秒ごとに自動リロード |

---

## Python バインディング

```bash
cargo build --release -p db_ffi
cd examples/python && python3 demo.py
```

```python
import sys
sys.path.insert(0, 'examples/python')
from db_engine import DbEngine

db = DbEngine()
id = db.insert({"name": "Alice", "age": 30}, labels=["dept:engineering"])
print(db.get(id))
print(db.get_by_label("dept:engineering"))
db.save("/tmp/kagura.json")
db2 = DbEngine.load("/tmp/kagura.json")
print(db2.list_labels())
```

---

## テスト

```bash
cargo test --workspace
```

| クレート | テスト数 | 内容 |
|----------|----------|------|
| `db_engine` | 34 | CRUD・ラベル操作・永続化 |
| `dynamic_label_management` | 24 | AND/OR検索・リネーム・差分 |
| `db_ffi` | 10 | FFI 関数・メモリ管理 |
| `db_client`（統合テスト） | 28 | HTTP API・永続化 |
| **合計** | **96** | |

---

## 技術スタック

| カテゴリ | 技術 |
|----------|------|
| 言語 | Rust 2024 edition |
| HTTP フレームワーク | [axum](https://github.com/tokio-rs/axum) 0.8 |
| 非同期ランタイム | [tokio](https://tokio.rs/) 1.x |
| シリアライゼーション | [serde](https://serde.rs/) + serde_json |
| FFI | Rust `extern "C"` + Python `ctypes` |
| フロントエンド | バニラ HTML / CSS / JavaScript（外部依存なし） |
| テスト（HTTP） | [reqwest](https://github.com/seanmonstar/reqwest) 0.12 |

---

## ライセンス

MIT License
