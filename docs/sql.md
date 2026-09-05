# SQL 機能

`POST /sql` エンドポイントで SQL SELECT / INSERT / UPDATE / DELETE クエリを実行できます。
通常の SQL とは異なり、テーブル名の代わりに `label.xxx` でラベルを指定する独自拡張構文です。
また、`MANAGE_USERS` 権限を持つユーザー向けに `CREATE USER` / `DROP USER` / `ALTER USER` / `SHOW USERS` / `GRANT` / `REVOKE`（[ユーザー管理](#ユーザー管理-create-user--drop-user--alter-user--show-users--grant--revoke)）も実行できます。

## SELECT 構文

```sql
SELECT * FROM label.employee
SELECT * FROM label.*
SELECT * FROM label.employee AND label.manager
SELECT * FROM label.employee OR label.developer
SELECT * FROM label.employee WHERE name = '田中'
SELECT * FROM label.employee WHERE age > 30 AND city = 'Tokyo'
SELECT * FROM label.employee WHERE name LIKE '田%'
SELECT * FROM label.employee ORDER BY salary DESC LIMIT 10

-- ':' など識別子に使えない文字を含むラベルは文字列リテラル形式で指定する
SELECT * FROM 'label.country:Japan'
SELECT * FROM label.customer AND 'label.category:drink'

-- 'label.' を省略した糖衣構文（標準SQLのテーブル名感覚で書ける）
SELECT * FROM employee WHERE age > 30

-- カラムに別名を付ける（ORDER BY からも別名で参照できる）
SELECT employee_name, employee_age AS age FROM label.employee ORDER BY age DESC

-- IS NULL / IS NOT NULL
SELECT * FROM label.employee WHERE department IS NULL
SELECT * FROM label.employee WHERE department IS NOT NULL

-- IN / NOT IN
SELECT * FROM label.employee WHERE department IN ('sales', 'dev')
SELECT * FROM label.employee WHERE department NOT IN ('sales')

-- BETWEEN / NOT BETWEEN（低い方 <= カラム <= 高い方 と同義）
SELECT * FROM label.employee WHERE age BETWEEN 20 AND 30
SELECT * FROM label.employee WHERE age NOT BETWEEN 20 AND 30

-- LIMIT と組み合わせたページング、OFFSET 単独指定も可能
SELECT * FROM label.employee ORDER BY age LIMIT 10 OFFSET 20
SELECT * FROM label.employee OFFSET 5
```

補足:
- ラベル名は `label.employee` のような識別子形式（英数字・`_`・日本語などの文字のみ）に加えて、
  `'label.country:Japan'` のような `'label.<name>'` 文字列リテラル形式でも指定できる。`:` や空白など
  識別子として使えない文字を含むラベル（`country:Japan` のような namespace 付きタグ）はリテラル形式が必要。
  `'label.*'` は `label.*` と同じ意味になる。`FROM` の `AND`/`OR` では両形式を混在させられる。
  `UPDATE`（`UPDATE 'label.xxx' SET ...`・`UPDATE LABEL 'label.old' SET 'label.new'`）・
  `DELETE`（`DELETE FROM 'label.xxx'`・`DELETE LABEL FROM 'label.xxx'`）でも同様。
  `INSERT INTO ('label.xxx')` は元から対応済み（[INSERT 構文](#insert-構文)を参照）。
- `label.` は `SELECT`/`UPDATE`/`DELETE` のラベル指定では省略でき、`FROM employee` は `FROM label.employee`
  と同じ意味になる（`UPDATE employee SET ...`・`DELETE FROM employee` も同様）。`label.*`（全レコード対象）は
  この糖衣構文の対象外で、引き続き `label.*` と明示する必要がある。
- `SELECT col AS alias` でカラムに別名を付けられる。出力の列名が別名に変わり、`ORDER BY alias` のように
  別名でソート対象を指定することもできる（`SELECT *` には別名を付けられない）。
- `WHERE column IS NULL` / `IS NOT NULL` は、カラムが未設定または `NULL` かどうかを判定する（従来からある
  `column = NULL` / `column != NULL` の特殊扱いと同じ判定を、標準SQLに近い構文で書けるようにしたもの）。
- `WHERE column IN (v1, v2, ...)` / `NOT IN` は、既存の `=` 比較を複数値に対して繰り返すのと同じ判定になる
  （型の混在の扱いも `=` に準じる）。
- `WHERE column BETWEEN low AND high` / `NOT BETWEEN` は `column >= low AND column <= high` と同義
  （数値・文字列の比較規則は既存の `>=`/`<=` に準じる）。
- `LIMIT n OFFSET m` で取得開始位置をずらせる（ページング用）。`OFFSET` は `LIMIT` を省略して単独でも指定できる。
  `total_matched`（`WHERE` 後・`LIMIT`/`OFFSET` 前の一致件数）は `OFFSET`/`LIMIT` の影響を受けない。

## INSERT 構文

```sql
-- カラム順を明示（推奨）: ラベル指定の直後に (col1, col2, ...) を置く
-- （標準SQLの INSERT INTO table (col1, col2) VALUES (...) に準じた語順）
INSERT INTO (label.employee) (employee_name, employee_age, employee_department) VALUE ('田中', 24, 'developer')

-- 複数ラベルを同時に付与
INSERT INTO (label.employee, label.manager) (employee_name, employee_age, employee_department) VALUE ('田中', 24, 'developer')

-- カラム順省略時: DB内の既存カラムをソート順に対応付ける（1件も既存カラムが無い場合はエラー）
INSERT INTO (label.employee) VALUE ('田中', 24, 'developer')

-- ラベル名は 'label.xxx' の文字列リテラル形式でも指定可能
INSERT INTO ('label.employee') (employee_name, employee_age, employee_department) VALUE ('田中', 24, 'developer')
```

補足:
- `INTO` の後のラベル名は必須で、`label.*` や空文字列は指定できない（無ラベル・全ラベル一括付与は不可）。
- 指定したラベルが未作成の場合、INSERT 実行時に自動作成される。
- `id` は自動採番のみで、INSERT 文からは指定できない。
- 同一 INSERT 文内で同じラベルを複数回指定した場合（例: `INTO (label.a, label.a)`）はラベル名の重複としてエラーになる。

## UPDATE 構文

```sql
-- データ更新: 対象ラベルのレコードのうち WHERE に一致する行の指定カラムを更新
UPDATE label.employee SET employee_name='木村' WHERE employee_name='木邑'

-- SET句は複数カラムを同時指定可能
UPDATE label.employee SET employee_age=30, employee_department='sales' WHERE employee_name='田中'

-- WHERE句は省略可能（省略時は対象ラベル内の全レコードが更新される）
UPDATE label.employee SET status='active'

-- ラベル名そのもののリネーム（WHERE省略時は old_label が付いた全レコードが一括対象。データ・他のラベルは変更しない）
UPDATE LABEL label.employee SET label.staff

-- WHEREで対象を絞り込み、一致したレコードだけラベルを付け替える（一括変更を避けたい場合）
UPDATE LABEL label.employee SET label.sales_staff WHERE department='sales'
```

補足:
- データ更新（`UPDATE label.xxx SET ...`）は SET句で指定したカラムのみを書き換える。それ以外の既存カラム・ラベルは維持される。
- `UPDATE LABEL label.old SET label.new [WHERE ...]` は `old` ラベルが付いているレコードのうち WHERE に一致するものだけ `old` を外し `new` を付け直す（レコードのカラムデータには影響しない）。`WHERE` を省略すると `old` が付いた全レコードが一括で対象になる。`label.*` は旧名・新名のどちらにも指定できない。
- 一致した行が無い場合もエラーにはならず、更新件数 0 として成功を返す。
- `UPDATE LABEL` のリネーム先ラベル名（`new`）が既にDB内の他のレコードで使われている場合は、WHEREでの絞り込みに関わらずラベル名の重複としてエラーを返し、何も変更しない。

## DELETE 構文

```sql
-- データ削除: 対象ラベルのレコードのうち WHERE に一致する行をデータごと完全に削除
DELETE FROM label.employee WHERE employee_name = '田中'

-- WHERE句を省略すると、対象ラベルが付いた全レコードが一括削除される
DELETE FROM label.employee

-- label.* を指定すると、DB全体の全レコードが対象になる（WHEREと組み合わせ可能）
DELETE FROM label.*

-- ラベルのみ削除: レコード自体・カラムデータ・他のラベルは削除せず、指定ラベルだけを対象レコードから外す
DELETE LABEL FROM label.employee WHERE employee_department = 'sales'

-- WHERE省略時は、そのラベルが付いた全レコードから一括でラベルを外す
DELETE LABEL FROM label.employee
```

補足:
- `DELETE FROM label.xxx [WHERE ...]` は対象ラベルのレコードを `WHERE` に一致する分だけデータごと完全に削除する（`WHERE` 省略時は対象ラベル内の全レコードを一括削除）。
- `DELETE FROM label.*` は全ラベル横断で対象を絞り込む（`WHERE` 省略時は DB 内の全レコードを削除する）。
- `DELETE LABEL FROM label.xxx [WHERE ...]` はレコードを削除せず、指定ラベルのみを対象レコードから外す（データ・他のラベルは変更しない）。`label.*` は指定できない。
- いずれも一致した行が無い場合はエラーにはならず、削除件数 0 として成功を返す。
- レスポンスの削除件数は `deleted_count` に格納される（`DELETE LABEL` の場合はラベルを外したレコード数）。

## ユーザー管理 (CREATE USER / DROP USER / ALTER USER / SHOW USERS / GRANT / REVOKE)

`POST /sql` では一般ユーザーの発行・管理を行う文も実行できます（**`MANAGE_USERS` 権限が必要**。
管理者 `kagura` は常に保持。持たないユーザーが実行すると `403`）。
これらの文はレコードストアではなく認証状態を操作します。

```sql
CREATE USER 'test' IDENTIFIED BY 'aA951753'
CREATE USER IF NOT EXISTS 'test' IDENTIFIED BY 'aA951753'
DROP USER 'test'
DROP USER IF EXISTS 'test'
ALTER USER 'test' IDENTIFIED BY 'Xy837261'
SHOW USERS
GRANT SELECT, INSERT TO 'test'
GRANT ALL TO 'alice', 'bob'
GRANT SUPER TO 'ops'
REVOKE DELETE FROM 'test'
```

- 簡単なパスワード（`1234` `aaa` / 8文字未満 / 単一文字種 / 連番 / `password` 等）は「脆弱」と判定され、
  `CREATE USER` / `ALTER USER` はそのままでは実行されず、レスポンスに
  `{"ok": false, "needs_confirmation": true, "warning": "Warning: The password strength is too weak. Do you want to proceed?\n\nPlease enter 'yes' to proceed or 'no' to cancel."}` を返す。
  リクエストに `"confirm_weak_password": true` を付けて再送すると作成を続行、`false` で中止（エラー）。
- ユーザー名・パスワードはクォート（`'...'` / `` `...` `` / `"..."`）または素の語で指定。クォート内の `'` は `''` でエスケープ。
- `GRANT` / `REVOKE` の権限は `SELECT` / `INSERT` / `UPDATE` / `DELETE` / `MANAGE_USERS` と、
  まとめ指定用の `ALL`（データ操作4種）・`SUPER`（4種 + `MANAGE_USERS`）。一般ユーザーは
  自分の `privileges` の範囲でのみデータ操作でき、不足時は `403`。詳細は
  [authentication.md](authentication.md#権限privilegesと-grant--revoke) を参照。
- 詳細は [authentication.md](authentication.md) を参照。

## リクエスト例

```bash
curl -X POST http://localhost:3000/sql \
  -H 'Content-Type: application/json' \
  -d '{"query": "SELECT employee_name, age, salary FROM label.employee WHERE age > 30 ORDER BY salary DESC LIMIT 10"}'

curl -X POST http://localhost:3000/sql \
  -H 'Content-Type: application/json' \
  -d "{\"query\": \"INSERT INTO (label.employee) (employee_name, employee_age, employee_department) VALUE ('田中', 24, 'developer')\"}"

curl -X POST http://localhost:3000/sql \
  -H 'Content-Type: application/json' \
  -d "{\"query\": \"UPDATE label.employee SET employee_name='木村' WHERE employee_name='木邑'\"}"

curl -X POST http://localhost:3000/sql \
  -H 'Content-Type: application/json' \
  -d "{\"query\": \"DELETE FROM label.employee WHERE employee_name='田中'\"}"
```

> リクエストには `Authorization: Bearer <token>` ヘッダーが必要です（[authentication.md](authentication.md) を参照）。

## 10,000件での性能測定

| クエリ | 応答時間 |
|--------|-------|
| `SELECT * FROM label.* LIMIT 100` | **9.7 ms** |
| `WHERE 等値検索` | **9.7 ms** |
| `WHERE LIKE` | **10.6 ms** |
| `WHERE 複合 AND` | **12.7 ms** |
| `FROM label.A AND label.B` | **30.9 ms** |
| `ORDER BY LIMIT 50` | **33.9 ms** |
| `WHERE 範囲検索` | **88.7 ms** |
| `SELECT * FROM label.employee`（2000件） | **154.8 ms** |
| `FROM label.A OR label.B`（3200件） | **252.9 ms** |
| `SELECT * FROM label.*`（全 10,000件） | **750.4 ms** |
