# 認証機能

KAGURA DB は単一の管理者（マスター）ユーザーによる Bearer トークン認証を備えています。
`/ui`（Web UI のHTMLシェル。ログイン画面自体を表示するため）と `POST /auth/login` を除く
**全エンドポイントがログイン必須**です（`GET /db/info` `POST /sql` `GET`/`PUT /settings` `GET /logs`
`/records` `/labels` 系すべてが対象。既存の HTTPリクエスト受付オンオフ設定 `http_api_enabled` は
認証の後段でさらに `/records` `/labels` 系のみを絞り込む形で従来通り機能する）。

| 項目 | 内容 |
|------|------|
| 管理者ユーザー名 | `kagura`（固定・現バージョンでは複数ユーザー管理は非対応） |
| 初期パスワード | `root`（初回起動時に認証情報ファイルが無い場合のみ自動作成される。**運用開始前に必ず変更してください**） |
| 認証方式 | `POST /auth/login` でユーザー名/パスワードを検証し、成功したらセッショントークンを発行。以後のリクエストは `Authorization: Bearer <token>` ヘッダーを付与する |
| トークンの保存場所 | サーバーのメモリ上のみ（再起動すると全セッションが失効し再ログインが必要） |
| トークンの有効期限 | 発行から24時間 |
| パスワードのハッシュ化 | [argon2](https://docs.rs/argon2) クレートによる Argon2id |
| 認証情報の永続化先 | 環境変数 `KAGURA_AUTH_FILE`（既定: `auth.json`）に `{"username":..., "password_hash":...}` の形式で保存 |
| ログイン失敗時のロック | 同一ユーザー名で5回連続失敗すると15分間ロック（`423 Locked` を返す）。成功するとカウントはリセットされる |
| パスワード変更 | `PUT /auth/password`（要ログイン）。現在のパスワードの検証に成功すると新しいパスワードへ更新し、盗まれたトークン対策として既存の全セッション（実行中のリクエスト自身のトークンを含む）を失効させる。新しいパスワードは8文字以上が必要 |

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
パスワード変更も行える。

コマンドラインからログイン・セッション管理を行う場合は [cli.md](cli.md)（`kdb` コマンド）を参照してください。
