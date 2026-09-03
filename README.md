# 🗄️ KAGURA DB

> Rust で一から実装したオリジナルのデータベースエンジン。  
> 列指向ストレージ・ラベル検索・KDB暗号化バイナリ形式・SQL SELECT/INSERT/UPDATE/DELETE・REST API・認証機能（複数ユーザー対応）・Web管理UI を備えたフルスタックな DB システムです。  
> バージョンの情報については以下の補足を確認ください。  
> 補足  
> ・メジャーバージョンが異なると互換性は無くなります。  
> ・メジャーバージョンが一致し、マイナーバージョンだけが異なる場合問題なく移行ができ互換性を保ちます。

**バージョン: `4.0.0`**

| 区分 | 説明 |
|------|------|
| **A = 4** (メジャー) | **【破壊的変更】** 一般ユーザーの `privileges` を実際に強制するようになった。`/sql` の SELECT/INSERT/UPDATE/DELETE と `/records`・`/labels` 系 REST API は、ログイン中ユーザーが対応する権限（`SELECT`/`INSERT`/`UPDATE`/`DELETE`）を持たない場合 `403 Forbidden` を返す（管理者ロール `kagura` は常に全権限。既存の一般ユーザーは作成時の既定で4種すべてを保持するため挙動は変わらない）。あわせて権限を付与・剥奪する SQL 文 `GRANT <権限>[, ...] TO <ユーザー>[, ...]` / `REVOKE <権限>[, ...] FROM <ユーザー>[, ...]` を追加（`MANAGE_USERS` 権限が必要。指定できる権限は `SELECT`/`INSERT`/`UPDATE`/`DELETE`/`MANAGE_USERS` と、まとめ指定用の `ALL`=データ操作4種・`SUPER`=4種+`MANAGE_USERS`）。3.0.0 で導入した「`/ui` と `POST /auth/login` を除く全エンドポイントがログイン必須」は継続。`.kdb` バイナリフォーマット・`auth.json` フォーマットは変更なし |
| **B = 0** (マイナー) | （今回のメジャー更新でリセット） |
| **C = 0** (ビルド) | （今回のメジャー更新でリセット） |

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
| [docs/installation.md](docs/installation.md) | 本番運用向けインストール（Linux systemd / Windows タスクスケジューラ） |
| [docs/development.md](docs/development.md) | テスト・テストデータ生成・Python バインディング |
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
├── db_ffi/                             # C FFI バインディング (v1.6.0)
│   └── src/lib.rs                      # FFI 関数（13本）＋テスト（10件）
├── kdb_cli/                             # コマンドラインクライアント (v3.3.0, バイナリ名: kdb)
│   └── src/
│       ├── main.rs                      # CLI定義（login/logout/whoami）とエントリーポイント
│       ├── client.rs                    # REST API クライアント（/auth/login /auth/logout /db/info）
│       └── session.rs                   # セッション永続化（~/.config/kdb/session.json）
├── packaging/                           # Linux/Windows パッケージング一式
└── examples/python/
    ├── db_engine.py                    # Python ctypes ラッパー
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

テスト（`cargo test --workspace`・合計 254 件）の内訳は [docs/development.md](docs/development.md) を参照してください。

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
| **4.0.0** | **【破壊的変更】** 一般ユーザーの `privileges` を実際に強制するようになり、あわせて権限付与・剥奪の SQL 文 `GRANT` / `REVOKE` を新規追加。`GRANT <権限>[, ...] TO <ユーザー>[, ...]` / `REVOKE <権限>[, ...] FROM <ユーザー>[, ...]`（`MANAGE_USERS` 権限が必要。指定できる権限は `SELECT`/`INSERT`/`UPDATE`/`DELETE`/`MANAGE_USERS` と、まとめ指定用の `ALL`=データ操作4種・`SUPER`=4種+`MANAGE_USERS`。複数権限・複数ユーザーをカンマ区切りで同時指定可。`ALL PRIVILEGES` 表記も許容）。`SUPER` / `ALL` は付与時に個別権限へ展開して `auth.json` へ永続化する。これらの文は他のユーザー管理文と同じく `db_client/src/user_sql.rs` の専用パーサーで処理（トークナイザに `,` を追加）。強制（enforce）は3か所: (1) `db_client/src/sql_handlers.rs` の `execute_sql` で SELECT→`SELECT` / INSERT→`INSERT` / UPDATE→`UPDATE` / DELETE→`DELETE` を要求、(2) `db_client/src/app.rs` に新ミドルウェア `require_data_privilege` を追加し `/records`・`/labels` 系 REST API をメソッド（GET/`POST /labels/search`→`SELECT`、POST→`INSERT`、PUT→`UPDATE`、DELETE→`DELETE`）で判定、(3) ユーザー管理文は従来どおり `MANAGE_USERS`。いずれも不足時は `403 Forbidden`（必要な権限名をメッセージに含む）。管理者ロール `kagura` は常に全権限。既存の一般ユーザーは作成時の既定で `SELECT`/`INSERT`/`UPDATE`/`DELETE` の4種を保持するため実挙動は変わらない。Web UI（設定→ユーザー管理）のユーザー一覧に権限チェックボックスを追加し、切り替えで `GRANT` / `REVOKE` を実行できるようにした。`.kdb` バイナリフォーマット・`auth.json` フォーマット・`kdb` CLI の挙動には影響しない。全クレートの `Cargo.toml` バージョンを `4.0.0` に同期 |
| **3.4.0** | 一般ユーザー管理機能を新規追加。管理者ユーザー `kagura` のみが `POST /sql` 経由で `CREATE USER 'name' IDENTIFIED BY 'password'` を実行して一般ユーザーを発行できる（`DROP USER 'name' [IF EXISTS]` / `ALTER USER 'name' IDENTIFIED BY 'pw'` / `SHOW USERS` にも対応）。これらの文はレコードストアではなく認証状態を操作するため `sql_engine` には手を入れず、`db_client/src/user_sql.rs`（新規）の専用パーサーで処理し `db_client/src/sql_handlers.rs` の `execute_sql` 冒頭でディスパッチする。簡単なパスワード（`1234` `aaa` / 8文字未満 / 単一文字種 / 連番 / よくある語）の場合は要件どおり `Warning: The password strength is too weak. Do you want to proceed?\n\nPlease enter 'yes' to proceed or 'no' to cancel.` を返す。`/sql` は `{"ok":false,"needs_confirmation":true,"warning":...}` を返し、クライアントは `confirm_weak_password: true` を付けて再送すると作成を続行、`false` で中止（エラー）。Web UI（`/ui` 設定画面）に「ユーザー管理」セクションを追加し、作成フォーム・`SHOW USERS` 一覧・削除ボタンを提供、脆弱パスワード時は `prompt` で yes/no を確認する。認証情報ファイル `auth.json`（`KAGURA_AUTH_FILE`）は単一ユーザー形式から複数ユーザー対応形式 `{"schema_version":2,"users":[{username,password_hash,role,privileges,created_at,disabled}]}` へ拡張。旧形式のファイルは起動時に自動で新形式へ移行する（ユーザー操作不要）。一般ユーザーには将来の GRANT/REVOKE を見据えた `privileges`（既定 `select,insert,update,delete`）を保持し、現バージョンで enforce するのは `manage_users`（ユーザー管理操作の可否）のみ。`PUT /auth/password` はトークンからログイン中ユーザーを解決して本人のパスワードを変更するよう変更。`.kdb` バイナリフォーマット（`VERSION`）は不変で、既存の REST API・Web UI・`kagura`/`root` ログイン・`kdb` CLI の挙動には影響しない。全クレートの `Cargo.toml` バージョンを `3.4.0` に同期 |
| **3.3.1** | 肥大化した `README.md`（約990行）を整理。認証・`kdb` CLI・インストール（Linux/Windows）・SQL 機能・KDB ストレージフォーマット・API リファレンス/Web UI/ログ機能・テスト/テストデータ生成/Python バインディングの各詳細セクションを `docs/` 配下の個別ファイル（`features.md` `authentication.md` `cli.md` `sql.md` `api.md` `storage-format.md` `installation.md` `development.md`）へ分割し、README 本体は概要・アーキテクチャ・機能概要・プロジェクト構成・クイックスタート・技術スタック・各ドキュメントへの入口・バージョン履歴に再編した。ドキュメントのみの変更でありコードの挙動・互換性には一切影響しない |
| **3.3.0** | `kdb shell-init bash`/`zsh`（シェル連携スクリプト出力）と `kdb prompt`（ログイン状態タグ出力。`shell-init`が内部的に利用）を新規追加。ログインしたことがOSのシェルプロンプト上からは分からず、`kdb login` を打ったあとも `[ec2-user@ip-10-0-0-5 scripts]$` のまま変化しないのが分かりにくいという指摘を受けて対応。`kdb login` 自体は子プロセスのため親シェルの `PS1`/`PROMPT` を直接書き換えることはできないので、`kube-ps1` 等と同様に「シェル設定ファイルに1行追加 → プロンプト描画のたびに `$(__kdb_ps1)` が `kdb prompt` を呼び出し、ローカルのセッションファイル（`~/.config/kdb/session.json`）の有無に応じてタグの表示/非表示を切り替える」方式を採用。ホスト名・カレントディレクトリ等の既存プロンプト情報は保持したまま、先頭に `(kdb:ユーザー名@接続先) ` タグを付加する。サーバーへの通信は行わずローカルのセッションファイルの有無のみで判定するため、トークンの期限切れ・サーバー側失効は反映されない（正確な有効性確認は `kdb whoami` を使う） |
| **3.2.1** | `kdb` バイナリのパッケージング不備を修正。`packaging/scripts/install.sh` を `cargo build --release -p db_client -p kdb_cli` でビルドし `/usr/bin/kdb` にも配置するよう変更（`uninstall.sh` も対応して削除）。`db_client/Cargo.toml` の `[package.metadata.deb]` / `[package.metadata.generate-rpm]` の `assets` に `target/release/kdb` → `/usr/bin/kdb` を追加し、`.deb`/`.rpm` パッケージにも同梱されるようにした。3.2.0時点ではworkspaceにクレートを追加しただけでいずれのインストール手順にも組み込んでおらず、`cargo build -p kdb_cli` を手動実行しない限り `kdb` コマンドが `PATH` 上に存在せず `command not found` になっていたため |
| **3.2.0** | KAGURA DB インストール後に使える専用コマンドラインクライアント `kdb`（新規クレート `kdb_cli`、バイナリ名 `kdb`。workspace members に追加）を新規追加。`kdb login -u <ユーザー名> -p <パスワード> -a <接続先>` で `POST /auth/login` を叩き、成功したらセッション（トークン・ユーザー名・接続先）を `~/.config/kdb/session.json`（パーミッション600）へ保存する。`-u` / `-p` / `-a` は `KDB_USER` / `KDB_PASSWORD` / `KDB_ADDR` 環境変数でも指定可能。`-p` を省略した場合は `rpassword` クレートによる非表示入力（シェル履歴・`ps`への平文パスワード漏えい回避）で対話的にプロンプトする一般的なDBクライアント（mysql/psql等）の慣習に合わせた。`-a` はスキーム省略時（`host:port`）に `http://` を自動補完し、`https://` も指定可能（`-k`/`--insecure` で自己署名証明書向けにTLS証明書検証をスキップ可能）。`kdb logout`（`POST /auth/logout` でサーバー側トークンを失効させローカルセッションを削除）、`kdb whoami`（保存済みセッションのユーザー名・接続先表示と `GET /db/info` によるトークン有効性確認）を追加。既存の REST API・Web UI・認証の挙動には影響しない |
| **3.1.0** | ログ出力保管機能に認証専用の `category: "auth"` を追加。従来ログイン成功/失敗・ログアウト・パスワード変更は `category: "db"` のログに `auth: ...` という接頭辞付きメッセージとして混在させて記録していたが、他のサブシステム（`sql` `http` `hardware`）と同様に独立したカテゴリへ分離し、`GET /logs` の絞り込みや Web UI ログビューのカテゴリセレクトから `auth` を選んで認証イベントだけを追えるようにした。`Logger` に `auth_info` / `auth_warn` を追加し、`auth_handlers.rs` のログイン・ログアウト・パスワード変更ハンドラーをこれらへ切り替え |
| **3.0.0** | **【破壊的変更】** 認証機能を新規追加。管理者（マスター）ユーザー `kagura`（初期パスワード `root`）による Bearer トークン認証を実装し、`/ui`（Web UI のHTMLシェル）と `POST /auth/login` を除く全エンドポイント（`/db/info` `/sql` `/settings` `/logs` `/records` `/labels` 系すべて）が既定でログイン必須になった。認証情報（ユーザー名・Argon2idハッシュ化されたパスワード）は環境変数 `KAGURA_AUTH_FILE`（既定 `auth.json`）に永続化し、ファイルが存在しない初回起動時のみ初期管理者を自動作成する。追加した認証 API は `POST /auth/login`（ログイン・トークン発行）、`POST /auth/logout`（トークン失効）、`PUT /auth/password`（パスワード変更。成功すると全セッションを失効させ再ログインを必須化）の3本。セッショントークンはメモリ上でのみ管理し有効期限は24時間、同一ユーザー名で5回連続ログイン失敗すると15分間ロックする（`423 Locked`）。Web UI にログイン画面・ログアウトボタン・パスワード変更フォームを追加。影響範囲: (1) 認証を追加する前提で書かれていない既存クライアント（curlスクリプト・`examples/python/generate_testdata.py` 等）は事前ログインが必要になる（`generate_testdata.py` は自動でログインするよう追随済み）。(2) 全クレートの `Cargo.toml` バージョンをこのREADMEと同期させた |
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
