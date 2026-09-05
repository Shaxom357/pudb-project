# 機能一覧

各クレートが提供する機能の詳細です。全体像は [../README.md](../README.md) を参照してください。

## db_engine（コアエンジン）
- ✅ CRUD（Insert / Get / Update / Delete）
- ✅ 列指向ストレージ（`ColumnStore` はラベル索引を担当。カラム値の実体は `records` が単一の保持元）
- ✅ **全データオンメモリ ON/OFF**: `Database::set_memory_policy` による切替。既定（`all_in_memory = true`）は全レコードをメモリ常駐（従来動作・オーバーヘッドなし）。`false` にすると `MemorySizeSpec`（絶対バイト数 or 搭載メモリに対する割合）の上限内にオンメモリ量を抑え、あふれた分は直近未使用の行から `.kdb` へ退避する（アクセス時に `KdbFile::read_record_at` で単体読み直し、再アクセスで立ち退き順が更新される＝実質LRU）。`.kdb` 形式は無変更。`memory_stats()` で常駐バイト数・行数を観測できる
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
- ✅ **二次インデックス**: `Database::create_index`/`drop_index` によるカラム単位の等値検索インデックス（`HashMap<値, Vec<行番号>>`。ラベルの検索索引と同じ発想）。`.kdb`/JSON 本体には実体を永続化せず、呼び出し側が定義を再投入して再構築する
- ✅ **任意スキーマ層**: `Database::define_column`/`enable_schema`/`disable_schema` によるラベル単位・カラム単位の任意スキーマ（型・`NOT NULL`・`DEFAULT`・`UNIQUE`）。既定でスキーマレスのまま、宣言＋`ENABLE SCHEMA` したラベルだけ `INSERT`/`UPDATE` 時に検証する。`UNIQUE` 指定時は二次インデックスを自動作成して重複チェックに利用。`validate_label` で既存データの整合性を事前確認可能。DB全体の一時停止スイッチ `schema_enforcement_enabled`（既定 `true`）も持つ

## sql_engine（簡易プランナ / EXPLAIN）
- ✅ `WHERE` 句全体がちょうど1つの等値比較で、対象カラムに二次インデックスがあれば自動的に使用（`AND`/`OR`/`NOT` を含む複合条件では現状未対応）
- ✅ `EXPLAIN <SELECT文>` でクエリを実行せず実行計画（インデックス使用の有無）だけを確認可能

## db_client（REST API サーバー）
- ✅ **認証機能**: 管理者ユーザー（既定 `kagura` / 初期パスワード `root`）による Bearer トークン認証。`/ui`（Web UI の殻）と `POST /auth/login` を除く全エンドポイントがログイン必須（詳細は [authentication.md](authentication.md) を参照）
- ✅ **一般ユーザー管理**: 管理者が SQL 文 `CREATE USER 'name' IDENTIFIED BY 'password'`（`DROP USER` / `ALTER USER` / `SHOW USERS` も対応）で一般ユーザーを発行。脆弱なパスワードは yes/no 確認を求める。Web UI の設定画面からも操作可能
- ✅ **権限管理 (GRANT / REVOKE)**: `GRANT <権限>[, ...] TO <ユーザー>[, ...]` / `REVOKE ... FROM ...` で一般ユーザーの `privileges` を付与・剥奪。権限は `SELECT` / `INSERT` / `UPDATE` / `DELETE` / `MANAGE_USERS` と、まとめ指定用の `ALL` / `SUPER`。一般ユーザーの `/sql`・`/records`・`/labels` 操作は付与された権限の範囲に制限され、不足時は `403`（管理者ロールは常に全権限）
- ✅ **二次インデックス管理 (CREATE INDEX / DROP INDEX / SHOW INDEXES)**: `MANAGE_USERS` 権限を持つユーザーが SQL 文でカラム単位の等値検索インデックスを作成・削除。定義はサイドカーファイル `<DB_FILE>.indexes.json` へ保存し起動時に再構築（`.kdb`/JSON 本体のフォーマットは不変）
- ✅ **任意スキーマ層管理 (ALTER LABEL ... DEFINE COLUMN / ENABLE|DISABLE SCHEMA / DESCRIBE / SHOW SCHEMAS / VALIDATE LABEL)**: `MANAGE_USERS` 権限を持つユーザーが SQL 文でラベルのカラムに型・`NOT NULL`・`DEFAULT`・`UNIQUE` を宣言・削除し、強制の有効/無効を切替。定義しただけでは強制されず `ENABLE SCHEMA` して初めて検証される（既定で無効）。定義はサイドカーファイル `<DB_FILE>.schema.json` へ保存し起動時に再構築。Web UI（設定画面）からも操作可能
- ✅ 21 本のエンドポイント（認証・レコード CRUD・ラベル管理・DB 情報・SQL・設定・ログ・Web UI。詳細は [api.md](api.md) を参照）
- ✅ KDB モード（`.kdb`）と JSON モードの自動判別・後方互换
- ✅ 起動時自動ロード・書き込み時自動セーブ
- ✅ 環境変数による設定（`DB_FILE` / `DB_ADDR` / `KAGURA_MASTER_KEY` / `KAGURA_AUTH_FILE`）
- ✅ ブラウザ Web 管理 UI（Records / DB Info / SQL Query / 設定 の 4 ビュー。設定ビューに「スキーマ管理」「スキーマ強制（全体設定）」セクションを追加）
- ✅ `GET /db/info` でバージョン・ストレージ状態・統計を取得
- ✅ `POST /sql` で SQL SELECT / INSERT / UPDATE / DELETE / EXPLAIN クエリ、および二次インデックス・スキーマ管理文を実行
- ✅ `GET /settings` / `PUT /settings` で HTTPリクエスト（`/records` `/labels` 系 REST API）受付のオンオフ、およびスキーマ強制の全体スイッチ `schema_enforcement_enabled`（**既定は有効**。`false` でラベルごとの `ENABLE SCHEMA` 状態に関わらず一時停止。サーバー再起動のたびに既定へ戻る）を切替可能。`http_api_enabled` の既定は無効（データ操作は基本 SQL 経由とし、REST API は大量テストデータ投入など用途に応じて有効化する）。いずれも指定したフィールドだけを更新し、省略したフィールドは現在値を維持
- ✅ **トランザクション**: `/sql` の `BEGIN`（`BEGIN TRANSACTION` / `START TRANSACTION`）・`COMMIT`・`ROLLBACK`。複数の `INSERT`/`UPDATE`/`DELETE` を1単位にまとめて全確定/全取消。同時に開けるのは1つ（`BEGIN` したログインユーザーが所有者。他ユーザーの書き込み・別の `BEGIN`・トランザクション中の DDL は `409`）。トランザクション中の `SELECT` は未コミット値が見える。`BEGIN`〜`COMMIT` 間は `.kdb`/JSON を書き換えず `COMMIT` 時に一括永続化、`ROLLBACK` はディスクから読み直す。無操作 300 秒で自動 `ROLLBACK`。容量制限モードでもトランザクション中は追い出しを停止
- ✅ **全データオンメモリ設定**: `GET /settings` は現在の設定（`all_in_memory` / `memory_limit` / 解決後の上限バイト数 / 現在の常駐バイト数・行数）を返し、`PUT /settings` の `all_in_memory`（bool）・`memory_limit`（`{"kind":"bytes"|"percent","value":n}`）・`memory_limit_spec`（`"500MB"` / `"2GB"` / `"20%"` の文字列）で変更できる。変更は**管理者ロール `kagura` のみ**（他ユーザーは `403`）、かつ **`.kdb` モードのみ**（JSON モードは `400`）。設定はサイドカーファイル `<DB_FILE>.memory.json` に保存し再起動をまたいで保持する。`kdb in-memory-mode enable|disable` / `kdb in-memory-size <値>` からも変更可能。Web UI の設定ビューにトグルと上限入力を追加
- ✅ Web UI の Records 一覧は `POST /sql`（`SELECT * FROM label.*`）経由で取得するため、REST API が無効でも常に閲覧可能
- ✅ **ログ出力保管機能**: 起動/停止時刻、停止理由（正常 / エラー / HW・ストレージ障害）、SQL・HTTPリクエストの成功/失敗、Webクライアントの応答時間、DB保存・レコード/ラベル操作などを JSON Lines 形式でファイルへ永続保存（詳細は [api.md](api.md#ログ機能) を参照）。`GET /logs` および Web UI の「ログ」ビューから閲覧可能

## sql_engine（SQL SELECT/INSERT/UPDATE/DELETE エンジン）
- ✅ 独自拡張 SQL `SELECT ... FROM label.xxx` 構文
- ✅ `WHERE` 句（`=` `!=` `<` `<=` `>` `>=` `LIKE`）
- ✅ `WHERE column IS NULL` / `IS NOT NULL`
- ✅ `WHERE column IN (v1, v2, ...)` / `NOT IN`
- ✅ `WHERE column BETWEEN low AND high` / `NOT BETWEEN`
- ✅ `AND` / `OR` / `NOT` / 括弧による複合条件
- ✅ `FROM label.A AND label.B`（ラベル AND 絞り込み）
- ✅ `FROM label.A OR label.B`（ラベル OR 結合）
- ✅ `FROM label.*`（全レコード対象）
- ✅ `label.` を省略した糖衣構文（`FROM employee` = `FROM label.employee`。`UPDATE`/`DELETE`の対象ラベル指定でも同様。`label.*`は対象外）
- ✅ `ORDER BY col [ASC|DESC]`（複数カラム対応。`SELECT ... AS alias` で付けた別名でも指定可能）
- ✅ `LIMIT n` / `LIMIT n OFFSET m` / `OFFSET m` 単独指定（ページング）
- ✅ カラム指定 `SELECT col1, col2 FROM ...`、`AS` によるカラムの別名指定
- ✅ `SELECT DISTINCT col1, col2, ...`（出力行の重複排除）
- ✅ 集計関数 `COUNT(*)` / `COUNT(col)` / `SUM(col)` / `AVG(col)` / `MIN(col)` / `MAX(col)`
- ✅ `GROUP BY col1, col2, ...`（省略時は集計関数を全件で1グループとして計算）・`HAVING <条件>`
- ✅ 算術演算式 `+` `-` `*` `/`（優先順位・括弧対応、単項マイナス対応）
- ✅ `CASE WHEN <条件> THEN <式> ... [ELSE <式>] END`（条件部は `WHERE` と同じ構文）
- ✅ `COALESCE(expr, ...)`・`CAST(expr AS type)`（`TEXT`/`INTEGER`/`FLOAT`/`BOOLEAN`）
- ✅ スカラ関数 `LENGTH` / `LOWER` / `UPPER` / `SUBSTR` / `ROUND` / `ABS`
- ✅ 大文字小文字無視・セミコロン対応
- ✅ `SELECT *` は `id` 列を先頭、`labels` 列（カンマ区切り）を末尾に付与して返す（`GROUP BY`/`HAVING` とは組み合わせ不可）
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
- ✅ ラベル名の文字列リテラル形式 `'label.xxx'`（`'label.*'` を含む）を `SELECT`/`UPDATE`/`DELETE` のラベル指定全箇所（`FROM`・`UPDATE LABEL`・`DELETE LABEL`）で使用可能。識別子形式（英数字・`_`のみ）では書けない `:` 等を含むラベル名（`country:Japan` のような namespace 付きタグ）を扱える。従来 `INSERT` のみで対応していた形式を拡張したもの

構文の詳細と例は [sql.md](sql.md) を参照してください。

## dynamic_label_management
- ✅ ラベルの AND / OR 検索
- ✅ ラベルのリネーム（DB 全体一括。対象レコードが元々リネーム先と同名のラベルも持っていた場合は重複付与エラーを避けるため付け直しをスキップ）
- ✅ ラベルのコピー・差分・グルーピング（コピー先が既に持つラベルは重複付与エラーを避けるためスキップし、新規コピー数のみを返す）
- ✅ ラベル統計情報（件数降順）
- ✅ 同一レコードへのラベル重複付与を禁止（`add_label` は既に付与済みのラベルに対してエラーを返す）

## db_ffi
- ✅ C ABI 互换の共有ライブラリ（`libdb_ffi.so`）
- ✅ Python `ctypes` ラッパークラス（平文 JSON 版 `DbEngine` / 暗号化 `.kdb` 版 `KdbEngine`）
- ✅ JSON 文字列による全データ型対応
- ✅ 暗号化 `.kdb` セッション（`KdbSession` 不透明ポインタ）: `kdb_open`/`kdb_close`/`kdb_save`（チェックポイント）/`kdb_count`
- ✅ WAL 追記 INSERT（`kdb_insert` = O(1)）／一括バックフィル（`kdb_insert_many`、JSON 配列を 1 件ずつ追記し成功件数を返す）
- ✅ `kdb_sql`: SELECT / INSERT / UPDATE / DELETE / EXPLAIN を文字列で実行し JSON で返す。`run_select`/`run_explain`（sql_engine）をそのまま呼ぶため、WHERE（IS NULL/IN/BETWEEN 等含む）・ORDER BY・LIMIT/OFFSET・ラベル AND-OR・集計関数・GROUP BY/HAVING・DISTINCT・算術演算/CASE/COALESCE/CAST/スカラ関数・二次インデックスを使った実行計画（EXPLAIN）まで REST API（`/sql`）と同じ SQL 構文が使える（JOIN のみ非対応）。UPDATE/DELETE は自動チェックポイント。**`CREATE`/`DROP INDEX` 自体はこの FFI からはまだ作成できない**（現時点では REST API 経由のみ。インデックス定義はサイドカーファイル `<DB_FILE>.indexes.json` に保存されるが `kdb_open` はこれを読まないため、db_client 側で作成したインデックスは `KdbEngine` セッションでは有効にならない。単一ライター制約により両者が同時に同じ `.kdb` を開くことは無いが、順番に使う場合でも今のところ引き継がれない）
- ✅ `kdb_get_by_label`（型情報付き）／`kdb_get_by_label_ndjson`（`SELECT *` 相当のフラット NDJSON をファイルへ書き出し。`polars.scan_ndjson` 向け）
- ✅ 単一ライター制約を補助する advisory lock ファイル（`<path>.lock` の排他作成）

## kdb_cli（コマンドラインクライアント）
- ✅ KAGURA DB インストール後に使える専用コマンド `kdb`（詳細は [cli.md](cli.md) を参照）
- ✅ `kdb login -u <ユーザー名> -p <パスワード> -a <接続先>` によるログイン。`-p` 省略時は非表示入力で対話的にプロンプト
- ✅ `-u` / `-p` / `-a` は環境変数 `KDB_USER` / `KDB_PASSWORD` / `KDB_ADDR` でも指定可能
- ✅ 接続先はスキーム省略（`host:port`）・`http://`・`https://` のいずれの形式でも指定可能
- ✅ ログイン成功時にセッション（トークン・ユーザー名・接続先）を `~/.config/kdb/session.json`（パーミッション600）へ保存し、`kdb logout` / `kdb whoami` から利用
- ✅ `kdb logout` でサーバー側のトークン失効とローカルセッションの削除
- ✅ `kdb whoami` で現在のログインユーザー・接続先・セッションの有効性を確認
- ✅ `-k` / `--insecure` で自己署名証明書など TLS 証明書検証のスキップに対応
