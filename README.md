# 🗄️ KAGURA DB

> Rust で一から実装したオリジナルのデータベースエンジン。  
> 列指向ストレージ・ラベル検索・KDB暗号化バイナリ形式・SQL SELECT/INSERT/UPDATE/DELETE・REST API・認証機能（複数ユーザー対応）・Web管理UI を備えたフルスタックな DB システムです。  
> バージョンの情報については以下の補足を確認ください。  
> 補足  
> ・メジャーバージョンが異なると互換性は無くなります。  
> ・メジャーバージョンが一致し、マイナーバージョンだけが異なる場合問題なく移行ができ互換性を保ちます。

**バージョン: `4.10.0`**

| 区分 | 説明 |
|------|------|
| **A = 4** (メジャー) | **【破壊的変更】** 一般ユーザーの `privileges` を実際に強制する。`/sql` の SELECT/INSERT/UPDATE/DELETE と `/records`・`/labels` 系 REST API は、ログイン中ユーザーが対応する権限（`SELECT`/`INSERT`/`UPDATE`/`DELETE`）を持たない場合 `403 Forbidden` を返す（管理者ロール `kagura` は常に全権限。既存の一般ユーザーは作成時の既定で4種すべてを保持するため挙動は変わらない）。権限を付与・剥奪する SQL 文 `GRANT <権限>[, ...] TO <ユーザー>[, ...]` / `REVOKE <権限>[, ...] FROM <ユーザー>[, ...]`（`MANAGE_USERS` 権限が必要。指定できる権限は `SELECT`/`INSERT`/`UPDATE`/`DELETE`/`MANAGE_USERS` と、まとめ指定用の `ALL`=データ操作4種・`SUPER`=4種+`MANAGE_USERS`）。3.0.0 で導入した「`/ui` と `POST /auth/login` を除く全エンドポイントがログイン必須」は継続。`.kdb` バイナリフォーマット・`auth.json` フォーマットは変更なし |
| **B = 10** (マイナー) | **【新機能】** バックアップ／復元機能を追加。`.kdb` 本体（暗号化されたまま）・サイドカーファイル（`*.indexes.json` / `*.schema.json` / `*.memory.json`）・`auth.json`（既定同梱・`WITHOUT AUTH` で除外）を、`manifest.json`（形式バージョン・KAGURA バージョン・作成日時・レコード件数・**マスターキー指紋**・各ファイルの SHA-256）付きの1つの `.kbak` アーカイブ（依存ライブラリなしの独自コンテナ・非圧縮）にまとめる。**別サーバーや新規／再インストール環境でも復旧できる**ことを主眼とする。SQL 文 `BACKUP TO '<保存先>' [WITH KEY] [WITHOUT AUTH]` / `RESTORE FROM '<.kbak>' [OLD KEY '<hex64>']`（`/sql`・Web UI の SQL Query ビュー）と、`kdb backup` / `kdb restore` / `kdb keyinfo` CLI（稼働中サーバー経由＋サーバー停止中の `--direct --env-file`）。**管理者ロール `kagura` のみ**・**トランザクション中は不可**（`409`）。マスターキー本体はアーカイブに含めない（`.kbak` だけでは復元できず、`WITH KEY` 同梱か `OLD KEY` 指定か、復元先で同じ `KAGURA_MASTER_KEY` を用意する必要がある）。復元先のマスターキーが作成時と異なる場合、旧キーで復号し復元先の現キーで**リキー（再暗号化）**してから配置する。`RESTORE` は現ファイルを `pre-restore-<timestamp>/` へ退避してから置き換え、適用後は「再起動が必要」＝稼働中プロセスは書き込み系リクエストを `409` で拒否し `auto_save` を抑止する（メモリ上の復元前データでディスクを上書きしないため。`SELECT` は可能）。**メジャーバージョン非互換の `.kbak` は復元を拒否**。サーバーローカル保存先はバックアップ成功のたびに**世代管理**（保持世代数の上限＝既定7・保持日数＝既定30、`<DB_FILE>.backup.json` に永続化。`0` で無制限）で剪定する。マスターキー表示エンドポイント `GET /settings/master-key`（管理者のみ・監査ログにキー値は残さない）と `kdb keyinfo [--reveal]` を追加。実装は `db_engine`（`sha256`・`backup` モジュール）＋ `db_client`（`backup`・`backup_settings`・`sql_handlers` のディスパッチ）＋ `kdb_cli`（`backup_cmd`）。ユニット・統合テストを新規24件追加。**この版の範囲**: Web 管理 UI のバックアップカードと REST エンドポイント（`POST /backup` 等）、`PUT /settings` からの世代設定変更は後続バージョンで追加予定。全クレートの `Cargo.toml` を `4.10.0` に同期 |
| **C = 0** (ビルド) | （マイナー更新に伴いリセット） |

---

## 📚 ドキュメント

この README は全体像とクイックスタートに絞っています。各機能の詳細は `docs/` を参照してください。

| ドキュメント | 内容 |
|--------------|------|
| [docs/features.md](docs/features.md) | 機能一覧（クレートごとの詳細） |
| [docs/authentication.md](docs/authentication.md) | 認証機能（Bearer トークン・パスワード変更・ロックアウト） |
| [docs/cli.md](docs/cli.md) | `kdb` コマンドラインクライアント（login/logout/whoami・シェルプロンプト連携） |
| [docs/sql.md](docs/sql.md) | SQL 機能（SELECT / INSERT / UPDATE / DELETE 構文と例・性能測定） |
| [docs/api.md](docs/api.md) | API リファレンス・Web 管理UI・ログ機能 |
| [docs/storage-format.md](docs/storage-format.md) | KDB 暗号化バイナリ形式の仕様 |
| [docs/backup.md](docs/backup.md) | バックアップ／復元（`.kbak`・`BACKUP`/`RESTORE`・`kdb backup`・マスターキーの扱い・災害復旧手順） |
| [docs/installation.md](docs/installation.md) | 本番運用向けインストール（Linux systemd / Windows タスクスケジューラ） |
| [docs/development.md](docs/development.md) | テスト・テストデータ生成・Python バインディング |
| [docs/version-history.md](docs/version-history.md) | 各バージョンの変更履歴（changelog） |
| [packaging/README.md](packaging/README.md) | パッケージ成果物の生成方法（メンテナー向け） |

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
               | HTTP (POST /auth/login → Bearer トークンを取得して以後の全リクエストに付与)
    +----------+----------------------------------+----------+
    |              db_client  (axum REST API + Web UI)       |
    |   auth / auth_handlers (認証・セッション管理)          |
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

## 機能概要

各項目の詳細は [docs/features.md](docs/features.md) を参照してください。

- **db_engine（コアエンジン）** — CRUD・列指向ストレージ・ラベル仮想テーブル・5 種のデータ型・KDB 暗号化ストレージ（WAL 方式）・JSON 後方互換
- **db_client（REST API サーバー）** — Bearer トークン認証・21 本のエンドポイント・KDB/JSON 自動判別・Web 管理UI・ログ出力保管機能
- **sql_engine** — 独自拡張 SQL `SELECT/INSERT/UPDATE/DELETE ... label.xxx`・`WHERE`/`AND`/`OR`/`NOT`・`ORDER BY`・`LIMIT`・ラベル AND/OR 絞り込み
- **dynamic_label_management** — ラベルの AND/OR 検索・リネーム・コピー/差分/グルーピング・統計・重複付与の禁止
- **db_ffi** — C ABI 互換の共有ライブラリと Python `ctypes` ラッパー
- **kdb_cli** — 専用コマンド `kdb`（login/logout/whoami・シェルプロンプト連携）

---

## プロジェクト構成

```
pudb-project/
├── Cargo.toml
├── README.md
├── docs/                                # 詳細ドキュメント（上記「📚 ドキュメント」を参照）
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
│   │   ├── auth.rs                     # 認証状態（複数ユーザー・権限・パスワード強度・セッション・永続化/移行）
│   │   ├── auth_handlers.rs            # 認証 API ハンドラー（ログイン/ログアウト/パスワード変更）
│   │   ├── user_sql.rs                 # ユーザー管理 SQL（CREATE/DROP/ALTER USER・SHOW USERS）のパース＆実行
│   │   ├── handlers.rs                 # レコード CRUD ハンドラー
│   │   ├── label_handlers.rs           # ラベル管理ハンドラー
│   │   ├── info_handlers.rs            # DB 情報 API ハンドラー
│   │   ├── sql_handlers.rs             # SQL 実行ハンドラー（SELECT / INSERT / UPDATE / DELETE / ユーザー管理）
│   │   ├── settings_handlers.rs        # 設定 API ハンドラー（HTTPリクエスト受付オンオフ）
│   │   ├── models.rs                   # JSON DTO
│   │   └── ui.html                     # 管理画面（HTML/CSS/JS、ログイン画面・ユーザー管理を含む）
│   └── tests/
│       ├── test_auth.rs                # 認証 API テスト（8件）
│       ├── test_users.rs               # 一般ユーザー管理テスト（7件）
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
├── db_ffi/                             # C FFI バインディング
│   └── src/
│       ├── lib.rs                      # 平文 JSON 版 FFI 関数（db_new/insert/get/... 13本）
│       └── kdb_session.rs              # 暗号化 .kdb セッション FFI（kdb_open/insert/sql/... 9本）＋テスト（10件）
├── kdb_cli/                             # コマンドラインクライアント (v3.3.0, バイナリ名: kdb)
│   └── src/
│       ├── main.rs                      # CLI定義（login/logout/whoami）とエントリーポイント
│       ├── client.rs                    # REST API クライアント（/auth/login /auth/logout /db/info）
│       └── session.rs                   # セッション永続化（~/.config/kdb/session.json）
├── packaging/                           # Linux/Windows パッケージング一式
└── examples/python/
    ├── db_engine.py                    # Python ctypes ラッパー（平文 JSON 版 DbEngine）
    ├── kdb_engine.py                   # Python ctypes ラッパー（暗号化 .kdb 版 KdbEngine）
    ├── demo.py                         # デモスクリプト
    └── generate_testdata.py           # 10,000件テストデータ自動生成スクリプト
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

本番運用向けのインストール（systemd / タスクスケジューラ）は [docs/installation.md](docs/installation.md) を参照してください。

### 動作確認

```bash
# まずログインしてトークンを取得する（既定: username=kagura, password=root）
# /db/info /sql /records /labels /settings /logs はすべてログイン必須（詳細は docs/authentication.md を参照）
TOKEN=$(curl -s -X POST http://localhost:3000/auth/login \
  -H 'Content-Type: application/json' \
  -d '{"username": "kagura", "password": "root"}' | python3 -c 'import json,sys;print(json.load(sys.stdin)["token"])')

# バージョン・状態確認
curl http://localhost:3000/db/info -H "Authorization: Bearer $TOKEN"

# SQL でレコード作成（既定の状態でそのまま使える）
curl -X POST http://localhost:3000/sql \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d "{\"query\": \"INSERT INTO (label.employee) (name, age) VALUE ('田中', 35)\"}"

# SQL で検索
curl -X POST http://localhost:3000/sql \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"query": "SELECT * FROM label.employee WHERE age > 30"}'

# HTTPリクエスト(REST API)は既定で無効。大量テストデータ投入などで使う場合は先に有効化する
curl -X PUT http://localhost:3000/settings \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' -d '{"http_api_enabled": true}'

curl -X POST http://localhost:3000/records \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"columns":{"name":{"type":"text","value":"田中"},"age":{"type":"integer","value":35}},"labels":["employee"]}'

# 至急、初期パスワードを変更する（成功すると上で取得した $TOKEN を含む全セッションが失効するので、以後は新パスワードで再ログインする）
curl -X PUT http://localhost:3000/auth/password \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"current_password": "root", "new_password": "your-new-strong-password"}'

# Web UI（未ログインの場合はログイン画面が表示される。Records一覧はSQL経由のため、HTTPリクエストが無効でも閲覧可能）
open http://localhost:3000/ui
```

SQL 構文の詳細は [docs/sql.md](docs/sql.md)、全エンドポイントは [docs/api.md](docs/api.md) を参照してください。

---

## 技術スタック

| カテゴリ | 技術 |
|----------|---------|
| 言語 | Rust 2024 edition |
| HTTP フレームワーク | [axum](https://github.com/tokio-rs/axum) 0.8 |
| 非同期ランタイム | [tokio](https://tokio.rs/) 1.x |
| シリアライゼーション | [serde](https://serde.rs/) + serde_json |
| 暗号化（KDBストレージ） | XChaCha20-Poly1305 AEAD（純 Rust 手実装・外部クレートなし） |
| 認証（パスワードハッシュ） | [argon2](https://docs.rs/argon2) クレート（Argon2id） |
| SQL パーサー | 手書き再帰下降パーサー（外部ライブラリなし） |
| FFI | Rust `extern "C"` + Python `ctypes` |
| フロントエンド | バニラ HTML / CSS / JavaScript（外部依存なし） |
| テスト（HTTP） | [reqwest](https://github.com/seanmonstar/reqwest) 0.12 |
| テストデータ生成 | Python 3.6+ 標準ライブラリのみ |
| ログ | JSON Lines 形式でファイル保存（[chrono](https://github.com/chronotope/chrono) でタイムスタンプ生成） |

テスト（`cargo test --workspace`・合計 289 件）の内訳は [docs/development.md](docs/development.md) を参照してください。

---

## バージョン履歴

バージョン表記は **A.B.C**（A=メジャー: 破壊的変更・旧バージョンとの非互換、B=マイナー: 破壊的変更を伴わない新機能追加、C=ビルド: 規模を問わないその他の変更・修正）。最新バージョンの区分の内訳はこの README 上部の表を参照してください。

各バージョンの詳細な変更履歴は **[docs/version-history.md](docs/version-history.md)** にまとめています。

---

## ライセンス

商用・非商用問わずソースコードを使ってのサードパーティソフトの公開は禁止  
KAGURA DBを利用したアプリケーション等の公開は可能
