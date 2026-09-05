# API リファレンス / Web 管理UI / ログ機能

- [API リファレンス](#api-リファレンス)
- [Web 管理UI](#web-管理ui)
- [ログ機能](#ログ機能)

---

## API リファレンス

### エンドポイント一覧

「要ログイン」列が ✅ の行は `Authorization: Bearer <token>` が無い/無効だと `401 Unauthorized` を返す
（トークンは `POST /auth/login` で取得。詳細は [authentication.md](authentication.md) を参照）。
「要HTTP API」列が ✅ の行は、既定では無効な `/records` `/labels` 系 REST API に属し、
`PUT /settings` で `http_api_enabled: true` にするまで `403 Forbidden` を返す（詳細は [Web 管理UI](#web-管理ui) の設定を参照）。
ログイン必須とHTTP API有効化は独立したチェックで、両方が ✅ の行は両方を満たす必要がある。

さらに `/records` `/labels` 系と `/sql` のデータ操作は、ログイン中ユーザーの `privileges` を検査する。
参照系（`GET`・`POST /labels/search`・`SELECT`）は `SELECT`、追加系（`POST`・`INSERT`）は `INSERT`、
更新系（`PUT`・`UPDATE`）は `UPDATE`、削除系（`DELETE`）は `DELETE` を要求し、不足時は `403 Forbidden`
（必要な権限名をメッセージに含む）。管理者ロール（`kagura`）は常に全権限。付与・剥奪は
`GRANT` / `REVOKE`（[authentication.md](authentication.md#権限privilegesと-grant--revoke)）で行う。

| メソッド | パス | 説明 | 要ログイン | 要HTTP API |
|---------|------|------|:---:|:---:|
| `GET`    | `/ui` | Web 管理 UI（未ログイン時はログイン画面を表示） | - | - |
| `POST`   | `/auth/login` | ログイン（ユーザー名/パスワードを検証しトークンを発行） | - | - |
| `POST`   | `/auth/logout` | ログアウト（渡したトークンを失効） | ✅ | - |
| `PUT`    | `/auth/password` | パスワード変更（成功すると全セッションが失効） | ✅ | - |
| `GET`    | `/records` | 全レコード取得 | ✅ | ✅ |
| `POST`   | `/records` | レコード作成 | ✅ | ✅ |
| `GET`    | `/records/{id}` | ID で取得 | ✅ | ✅ |
| `PUT`    | `/records/{id}` | レコード更新 | ✅ | ✅ |
| `DELETE` | `/records/{id}` | レコード削除 | ✅ | ✅ |
| `GET`    | `/records/label/{label}` | ラベルで絞り込み取得 | ✅ | ✅ |
| `GET`    | `/records/{id}/labels` | レコードのラベル一覧 | ✅ | ✅ |
| `POST`   | `/records/{id}/labels` | ラベル追加 | ✅ | ✅ |
| `DELETE` | `/records/{id}/labels/{label}` | ラベル削除 | ✅ | ✅ |
| `GET`    | `/labels` | 全ラベル一覧+統計 | ✅ | ✅ |
| `POST`   | `/labels/search` | AND/OR ラベル検索 | ✅ | ✅ |
| `PUT`    | `/labels/rename` | ラベルリネーム | ✅ | ✅ |
| `GET`    | `/db/info` | DB バージョン・統計情報 | ✅ | - |
| `POST`   | `/sql` | SQL SELECT / INSERT / UPDATE / DELETE / EXPLAIN 実行（一般ユーザーは付与された権限の範囲のみ・不足時 `403`）、`MANAGE_USERS` 権限向けユーザー管理（`CREATE`/`DROP`/`ALTER USER`・`SHOW USERS`・`GRANT`/`REVOKE`。詳細は [authentication.md](authentication.md)）、同じく `MANAGE_USERS` 権限向け二次インデックス管理（`CREATE`/`DROP INDEX`・`SHOW INDEXES`）、および任意スキーマ層管理（`ALTER LABEL ... DEFINE COLUMN`/`DROP COLUMN`/`ENABLE`・`DISABLE SCHEMA`・`DESCRIBE`・`SHOW SCHEMAS`・`VALIDATE LABEL`。詳細は [sql.md](sql.md#二次インデックス-create-index--drop-index--show-indexes--explain) と [sql.md](sql.md#任意スキーマ層-alter-label--define-column--enabledisable-schema--describe--show-schemas--validate-label)） | ✅ | - |
| `GET`    | `/settings` | 現在の設定取得（HTTPリクエスト受付、スキーマ強制スイッチ、全データオンメモリ設定＝`all_in_memory`・`memory_limit`・解決後の上限バイト数・現在の常駐バイト数/行数） | ✅ | - |
| `PUT`    | `/settings` | 設定更新（`http_api_enabled`、`schema_enforcement_enabled`、および全データオンメモリの `all_in_memory`（bool）・`memory_limit`（`{"kind":"bytes"\|"percent","value":n}`）・`memory_limit_spec`（`"500MB"`/`"2GB"`/`"20%"`）。指定したフィールドだけを更新）。全データオンメモリ設定の変更は**管理者ロール `kagura` のみ**（他は `403`）かつ **`.kdb` モードのみ**（JSON は `400`）で、`<DB_FILE>.memory.json` へ永続化する | ✅ | - |
| `GET`    | `/logs` | 保管されたログを新しい順に取得（`?lines=` で件数指定、既定200・上限2000） | ✅ | - |

### GET /db/info レスポンス例

```json
{
  "engine_name":    "KAGURA DB Engine",
  "app_version":    "3.1.0",
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

`http://localhost:3000/ui` でブラウザから DB を操作できます。未ログインの場合はまずログイン画面が
表示されます（既定 `kagura` / `root`）。ログインに成功するとトークンをブラウザの `localStorage` に
保存し、以後の全リクエストへ自動的に付与します。ヘッダー右上の「Logout」でいつでもログアウトできます。

| ビュー | 説明 |
|--------|------|
| **📋 Records** | レコード一覧（`POST /sql` の `SELECT * FROM label.*` 経由で取得。HTTPリクエストが無効でも閲覧可能）・検索・ラベルフィルター・自動更新（10秒）。作成・編集・削除は REST API（`/records`）を使うため、これらの操作には設定で HTTPリクエストを有効化する必要がある |
| **ℹ️ DB Info** | バージョン・ストレージモード・暗号化状態・統計カード・カラム一覧 |
| **🔍 SQL Query** | SQL SELECT / INSERT / UPDATE / DELETE 実行・テーブル形式結果表示・ Ctrl+Enter 対応。脆弱パスワードの `CREATE USER` 実行時は確認プロンプトを表示 |
| **⚙️ 設定** | HTTPリクエスト（`/records` `/labels` 系 REST API）受付のオンオフを切替（既定は無効）。管理者パスワードの変更フォーム、一般ユーザーの作成・一覧・削除（管理者のみ）、スキーマ管理・スキーマ強制スイッチ、および「全データオンメモリ」設定（ON/OFF トグルと容量上限の入力。管理者のみ・`.kdb` モードのみ）もここにある |
| **📜 ログ** | 保管されたログ（起動/停止・SQL・HTTP・DB操作・認証・HW異常）をカテゴリで絞り込みながら一覧表示・自動更新（10秒） |

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
| 認証 | `category: "auth"` でログインの成功/失敗（`success`）・ログアウト・パスワード変更・アカウントロックを記録（失敗時はユーザー名と理由をメッセージに含む） |
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
