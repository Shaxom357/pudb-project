# 機能一覧

各クレートが提供する機能の詳細です。全体像は [../README.md](../README.md) を参照してください。

## db_engine（コアエンジン）
- ✅ CRUD（Insert / Get / Update / Delete）
- ✅ 列指向ストレージ（`ColumnStore`）
- ✅ ラベル付与・検索・削除（attach / detach）。**同一レコードへのラベル重複付与は禁止**：既に付与済みのラベルを再度付与しようとした場合は変更せず `DuplicateLabel` エラーを返す（INSERT で1レコードに同じラベルを複数指定した場合も同様にエラー）
- ✅ ラベル仮想テーブル（`HashMap<String, Vec<usize>>`）による高速検索
- ✅ 5 種類のデータ型（Text / Integer / Float / Boolean / Null）
- ✅ **KDB バイナリ暗号化ストレージ**（`.kdb` 形式。詳細は [storage-format.md](storage-format.md) を参照）
  - XChaCha20-Poly1305 AEAD による暗号化（外部クレート不使用・純 Rust 実装）
  - WAL（Write-Ahead Log）追記方式で INSERT が O(1) の高速書き込み
  - ファイル固有 Salt による鍵派生（`HChaCha20`）
  - 改ざん検知（Poly1305 MAC）
- ✅ JSON 形式永続化（後方互换・アトミック書き込み）
- ✅ `KAGURA_MASTER_KEY` 環境変数によるカスタムマスターキー設定

## db_client（REST API サーバー）
- ✅ **認証機能**: 管理者ユーザー（既定 `kagura` / 初期パスワード `root`）による Bearer トークン認証。`/ui`（Web UI の殻）と `POST /auth/login` を除く全エンドポイントがログイン必須（詳細は [authentication.md](authentication.md) を参照）
- ✅ **一般ユーザー管理**: 管理者が SQL 文 `CREATE USER 'name' IDENTIFIED BY 'password'`（`DROP USER` / `ALTER USER` / `SHOW USERS` も対応）で一般ユーザーを発行。脆弱なパスワードは yes/no 確認を求める。Web UI の設定画面からも操作可能
- ✅ **権限管理 (GRANT / REVOKE)**: `GRANT <権限>[, ...] TO <ユーザー>[, ...]` / `REVOKE ... FROM ...` で一般ユーザーの `privileges` を付与・剥奪。権限は `SELECT` / `INSERT` / `UPDATE` / `DELETE` / `MANAGE_USERS` と、まとめ指定用の `ALL` / `SUPER`。一般ユーザーの `/sql`・`/records`・`/labels` 操作は付与された権限の範囲に制限され、不足時は `403`（管理者ロールは常に全権限）
- ✅ 21 本のエンドポイント（認証・レコード CRUD・ラベル管理・DB 情報・SQL・設定・ログ・Web UI。詳細は [api.md](api.md) を参照）
- ✅ KDB モード（`.kdb`）と JSON モードの自動判別・後方互换
- ✅ 起動時自動ロード・書き込み時自動セーブ
- ✅ 環境変数による設定（`DB_FILE` / `DB_ADDR` / `KAGURA_MASTER_KEY` / `KAGURA_AUTH_FILE`）
- ✅ ブラウザ Web 管理 UI（Records / DB Info / SQL Query / 設定 の 4 ビュー）
- ✅ `GET /db/info` でバージョン・ストレージ状態・統計を取得
- ✅ `POST /sql` で SQL SELECT / INSERT / UPDATE / DELETE クエリを実行
- ✅ `GET /settings` / `PUT /settings` で HTTPリクエスト（`/records` `/labels` 系 REST API）受付のオンオフを切替可能（**既定は無効**。データ操作は基本 SQL 経由とし、REST API は大量テストデータ投入など用途に応じて有効化する）
- ✅ Web UI の Records 一覧は `POST /sql`（`SELECT * FROM label.*`）経由で取得するため、REST API が無効でも常に閲覧可能
- ✅ **ログ出力保管機能**: 起動/停止時刻、停止理由（正常 / エラー / HW・ストレージ障害）、SQL・HTTPリクエストの成功/失敗、Webクライアントの応答時間、DB保存・レコード/ラベル操作などを JSON Lines 形式でファイルへ永続保存（詳細は [api.md](api.md#ログ機能) を参照）。`GET /logs` および Web UI の「ログ」ビューから閲覧可能

## sql_engine（SQL SELECT/INSERT/UPDATE/DELETE エンジン）
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

構文の詳細と例は [sql.md](sql.md) を参照してください。

## dynamic_label_management
- ✅ ラベルの AND / OR 検索
- ✅ ラベルのリネーム（DB 全体一括。対象レコードが元々リネーム先と同名のラベルも持っていた場合は重複付与エラーを避けるため付け直しをスキップ）
- ✅ ラベルのコピー・差分・グルーピング（コピー先が既に持つラベルは重複付与エラーを避けるためスキップし、新規コピー数のみを返す）
- ✅ ラベル統計情報（件数降順）
- ✅ 同一レコードへのラベル重複付与を禁止（`add_label` は既に付与済みのラベルに対してエラーを返す）

## db_ffi
- ✅ C ABI 互换の共有ライブラリ（`libdb_ffi.so`）
- ✅ Python `ctypes` ラッパークラス（`DbEngine`）
- ✅ JSON 文字列による全データ型対応

## kdb_cli（コマンドラインクライアント）
- ✅ KAGURA DB インストール後に使える専用コマンド `kdb`（詳細は [cli.md](cli.md) を参照）
- ✅ `kdb login -u <ユーザー名> -p <パスワード> -a <接続先>` によるログイン。`-p` 省略時は非表示入力で対話的にプロンプト
- ✅ `-u` / `-p` / `-a` は環境変数 `KDB_USER` / `KDB_PASSWORD` / `KDB_ADDR` でも指定可能
- ✅ 接続先はスキーム省略（`host:port`）・`http://`・`https://` のいずれの形式でも指定可能
- ✅ ログイン成功時にセッション（トークン・ユーザー名・接続先）を `~/.config/kdb/session.json`（パーミッション600）へ保存し、`kdb logout` / `kdb whoami` から利用
- ✅ `kdb logout` でサーバー側のトークン失効とローカルセッションの削除
- ✅ `kdb whoami` で現在のログインユーザー・接続先・セッションの有効性を確認
- ✅ `-k` / `--insecure` で自己署名証明書など TLS 証明書検証のスキップに対応
