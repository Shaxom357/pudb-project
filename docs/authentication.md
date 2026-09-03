# 認証機能

KAGURA DB は Bearer トークン認証を備えています。マスター管理者ユーザー `kagura` に加えて、
管理者が SQL で発行する**一般ユーザー**をサポートします（[一般ユーザー管理](#一般ユーザー管理-create-user--drop-user--alter-user--show-users) 参照）。
`/ui`（Web UI のHTMLシェル。ログイン画面自体を表示するため）と `POST /auth/login` を除く
**全エンドポイントがログイン必須**です（`GET /db/info` `POST /sql` `GET`/`PUT /settings` `GET /logs`
`/records` `/labels` 系すべてが対象。既存の HTTPリクエスト受付オンオフ設定 `http_api_enabled` は
認証の後段でさらに `/records` `/labels` 系のみを絞り込む形で従来通り機能する）。

| 項目 | 内容 |
|------|------|
| 管理者ユーザー名 | `kagura`（`role: admin`。削除・降格不可。全操作が可能） |
| 初期パスワード | `root`（初回起動時に認証情報ファイルが無い場合のみ自動作成される。**運用開始前に必ず変更してください**） |
| 一般ユーザー | 管理者が `CREATE USER` で発行（`role: user`）。既定の権限は `select, insert, update, delete`（データ操作一式）。ユーザー管理系の操作（`CREATE`/`DROP`/`ALTER USER`・`SHOW USERS`）は不可 |
| 認証方式 | `POST /auth/login` でユーザー名/パスワードを検証し、成功したらセッショントークンを発行。以後のリクエストは `Authorization: Bearer <token>` ヘッダーを付与する |
| トークンの保存場所 | サーバーのメモリ上のみ（再起動すると全セッションが失効し再ログインが必要） |
| トークンの有効期限 | 発行から24時間 |
| パスワードのハッシュ化 | [argon2](https://docs.rs/argon2) クレートによる Argon2id |
| 認証情報の永続化先 | 環境変数 `KAGURA_AUTH_FILE`（既定: `auth.json`）に `{"schema_version": 2, "users": [{ "username", "password_hash", "role", "privileges", "created_at", "disabled" }]}` の形式で保存。旧形式 `{"username":..., "password_hash":...}` のファイルは起動時に自動で新形式へ移行する |
| ログイン失敗時のロック | 同一ユーザー名で5回連続失敗すると15分間ロック（`423 Locked` を返す）。成功するとカウントはリセットされる |
| パスワード変更 | `PUT /auth/password`（要ログイン）。トークンからログイン中ユーザーを解決し、現在のパスワードの検証に成功すると本人のパスワードを更新。盗まれたトークン対策として既存の全セッション（実行中のリクエスト自身のトークンを含む）を失効させる。新しいパスワードは8文字以上が必要 |

## 認証 API

```bash
# ログイン（初期状態）
curl -X POST http://localhost:3000/auth/login \
  -H 'Content-Type: application/json' \
  -d '{"username": "kagura", "password": "root"}'
# -> {"token": "...", "username": "kagura"}

# 以後のリクエストは Authorization ヘッダーにトークンを付与する
curl http://localhost:3000/db/info -H 'Authorization: Bearer <token>'

# パスワード変更（成功すると渡したトークンを含め全セッションが失効するため、以後は新パスワードで再ログインする）
curl -X PUT http://localhost:3000/auth/password \
  -H 'Authorization: Bearer <token>' -H 'Content-Type: application/json' \
  -d '{"current_password": "root", "new_password": "your-new-strong-password"}'

# ログアウト（渡したトークンのみ失効）
curl -X POST http://localhost:3000/auth/logout -H 'Authorization: Bearer <token>'
```

Web UI（`/ui`）はトークンを持たない状態でアクセスするとログイン画面を表示し、ログイン成功後は
トークンをブラウザの `localStorage` に保存して以後の全リクエストに自動付与する。設定ビューから
パスワード変更・一般ユーザーの作成/一覧/削除も行える（一般ユーザーでログインしている場合、ユーザー管理操作は `403` になる）。

コマンドラインからログイン・セッション管理を行う場合は [cli.md](cli.md)（`kdb` コマンド）を参照してください。

## 一般ユーザー管理 (CREATE USER / DROP USER / ALTER USER / SHOW USERS / GRANT / REVOKE)

一般ユーザーの発行・管理は **`POST /sql` 経由の SQL 文**で行います（`MANAGE_USERS` 権限が必要。
`kagura` などの管理者は常に保持。持たないユーザーが実行すると `403 Forbidden`）。
これらの文はレコードストアではなく認証状態を操作します。

| 文 | 説明 |
|----|------|
| `CREATE USER 'name' IDENTIFIED BY 'password'` | 一般ユーザーを作成。`CREATE USER IF NOT EXISTS ...` で重複時にエラーにしない |
| `DROP USER 'name' [IF EXISTS]` | ユーザーを削除（`kagura` など管理者は削除不可）。そのユーザーのセッションも失効する |
| `ALTER USER 'name' IDENTIFIED BY 'newpassword'` | 管理者権限でパスワードをリセット。そのユーザーのセッションも失効する |
| `SHOW USERS` | ユーザー一覧（`username, role, privileges, created_at, disabled`）。パスワードハッシュは返さない |
| `GRANT <権限>[, ...] TO <ユーザー>[, ...]` | 権限を付与する |
| `REVOKE <権限>[, ...] FROM <ユーザー>[, ...]` | 権限を剥奪する |

ユーザー名はクォート（`'...'` / `` `...` `` / `"..."`）または素の語で指定できます（空白不可・1〜64文字）。
パスワードにクォートを含める場合は `''` のように2つ重ねてエスケープします。

### 脆弱なパスワードの確認フロー

`1234` `aaa`、8文字未満、単一文字種のみ、連番、`password` などのよくある語は「脆弱」と判定され、
`CREATE USER` / `ALTER USER` 実行時に確認を求めます。

```bash
# 1回目: 確認なしで脆弱パスワードを指定 -> 実行されず確認要求が返る
curl -X POST http://localhost:3000/sql \
  -H 'Authorization: Bearer <token>' -H 'Content-Type: application/json' \
  -d '{"query": "CREATE USER '\''test'\'' IDENTIFIED BY '\''1234'\''"}'
# -> {"ok": false, "needs_confirmation": true,
#     "warning": "Warning: The password strength is too weak. Do you want to proceed?\n\nPlease enter '\''yes'\'' to proceed or '\''no'\'' to cancel."}

# 2回目: confirm_weak_password で回答する（true=続行 / false=中止）
curl -X POST http://localhost:3000/sql \
  -H 'Authorization: Bearer <token>' -H 'Content-Type: application/json' \
  -d '{"query": "CREATE USER '\''test'\'' IDENTIFIED BY '\''1234'\''", "confirm_weak_password": true}'
# -> {"ok": true, ...}
```

Web UI では同じ判定時に確認ダイアログ（`yes` と入力）を表示し、`yes` なら `confirm_weak_password: true` で再送します。

### 例

```sql
CREATE USER 'test' IDENTIFIED BY 'aA951753';   -- 強いパスワードはそのまま作成される
SHOW USERS;
ALTER USER 'test' IDENTIFIED BY 'Xy837261';
DROP USER IF EXISTS 'test';
```

### 権限（privileges）と GRANT / REVOKE

一般ユーザーは `privileges` リストの範囲でのみ操作できます（作成時の既定は
`select, insert, update, delete`）。管理者ロール（`kagura`）は常に全権限を持ちます。

| 権限キーワード | 許可される操作 | 許可されない操作 |
|----------------|----------------|------------------|
| `SELECT` | データの表示・検索（`SELECT` / `GET /records` / `GET /labels` / `POST /labels/search`） | ユーザー一覧の表示（`SHOW USERS`） |
| `INSERT` | データ・ラベルの追加（`INSERT` / `POST /records` / `POST /records/{id}/labels`） | ユーザーの作成 |
| `UPDATE` | データ・ラベルの更新（`UPDATE` / `PUT /records/{id}` / `PUT /labels/rename`） | ユーザー名の変更・パスワード更新 |
| `DELETE` | データ・ラベルの削除（`DELETE` / `DELETE /records/{id}` / `DELETE /records/{id}/labels/{label}`） | ユーザーの削除 |
| `MANAGE_USERS` | ユーザー管理系 SQL（`CREATE`/`DROP`/`ALTER USER`・`SHOW USERS`・`GRANT`/`REVOKE`） | — |
| `SUPER` | `SELECT`+`INSERT`+`UPDATE`+`DELETE`+`MANAGE_USERS` をまとめて付与 | — |
| `ALL` | `SELECT`+`INSERT`+`UPDATE`+`DELETE` をまとめて付与（ユーザー操作は含まない） | — |

権限が不足している操作は `403 Forbidden` を返します（`/sql` はレスポンス `error`、
REST API は `{"error": "..."}` に必要な権限名を含めて返します）。

```sql
GRANT SELECT, INSERT TO 'alice';          -- 複数権限をまとめて付与
GRANT ALL TO 'alice', 'bob';              -- 複数ユーザーへ同時付与
GRANT SUPER TO 'ops';                     -- データ操作 + ユーザー管理
REVOKE DELETE FROM 'alice';               -- 剥奪
REVOKE ALL PRIVILEGES FROM 'bob';         -- `ALL PRIVILEGES` とも書ける
```

- 権限キーワードは大文字小文字を区別しません。ユーザー名はクォート（`'...'`）または素の語で指定します。
- 対象は既存の一般ユーザーのみ。存在しないユーザーは `404`、管理者ユーザーを対象にすると `403`。
- `SUPER` / `ALL` は付与時に個別権限へ展開して保存されます（`SHOW USERS` には展開後の一覧が出ます）。
- 変更は認証情報ファイル（`KAGURA_AUTH_FILE`）へ即時永続化されます。
- Web UI（設定 → ユーザー管理）のユーザー一覧では、権限チェックボックスの切り替えで `GRANT` / `REVOKE` を実行できます。
