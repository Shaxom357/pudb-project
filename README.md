# 🗄️ KAGURA DB

> Rust で一から実装したオリジナルのデータベースエンジン。  
> 列指向ストレージ・ラベル検索・KDB暗号化バイナリ形式・SQL SELECT・REST API・Web管理UI を備えたフルスタックな DB システムです。

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
