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

## 一般ユーザー管理 (CREATE USER / DROP USER / ALTER USER / SHOW USERS)

一般ユーザーの発行・管理は **`POST /sql` 経由の SQL 文**で行います（`kagura` などの管理者権限が必要。
一般ユーザーが実行すると `403 Forbidden`）。これらの文はレコードストアではなく認証状態を操作します。

| 文 | 説明 |
|----|------|
| `CREATE USER 'name' IDENTIFIED BY 'password'` | 一般ユーザーを作成。`CREATE USER IF NOT EXISTS ...` で重複時にエラーにしない |
| `DROP USER 'name' [IF EXISTS]` | ユーザーを削除（`kagura` など管理者は削除不可）。そのユーザーのセッションも失効する |
| `ALTER USER 'name' IDENTIFIED BY 'newpassword'` | 管理者権限でパスワードをリセット。そのユーザーのセッションも失効する |
| `SHOW USERS` | ユーザー一覧（`username, role, privileges, created_at, disabled`）。パスワードハッシュは返さない |

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

### 権限（privileges）について

一般ユーザーには将来の `GRANT` / `REVOKE` を見据えた `privileges` リストを保持しています
（作成時の既定は `select, insert, update, delete`）。現バージョンで実際に参照するのは
`manage_users`（ユーザー管理系操作の可否）のみで、データ操作（`SELECT`/`INSERT`/`UPDATE`/`DELETE`）は
ログイン済みであれば従来どおり誰でも実行できます。
