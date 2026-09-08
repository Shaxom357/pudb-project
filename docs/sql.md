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

-- DISTINCT
SELECT DISTINCT department FROM label.employee

-- 集計関数（COUNT/SUM/AVG/MIN/MAX）と GROUP BY / HAVING
SELECT COUNT(*) FROM label.employee
SELECT department, COUNT(*) AS n, AVG(age) AS avg_age
  FROM label.employee
  GROUP BY department
  HAVING n >= 2
  ORDER BY avg_age DESC

-- 算術演算（+ - * /）。'-' を減算として使うときは前後に空白が必要（詰めて書くと負数リテラルになる）
SELECT odds * 100 AS pct FROM label.race
SELECT price - 10 AS discounted FROM label.product
SELECT (a + b) * 2 AS n FROM label.x
SELECT -age AS negated FROM label.employee

-- CASE 式（条件部は WHERE と同じ構文が使える）
SELECT name, CASE WHEN rank = 1 THEN 'win' WHEN rank <= 3 THEN 'place' ELSE 'lose' END AS result
  FROM label.race

-- COALESCE（先頭から順にNULLでない最初の値を返す）
SELECT COALESCE(nickname, name) AS display_name FROM label.employee

-- CAST（text / integer / float / boolean へ変換。変換できなければ NULL）
SELECT CAST(age AS text) AS age_text FROM label.employee

-- スカラ関数: LENGTH / LOWER / UPPER / SUBSTR(str, start[, length]) / ROUND(num[, digits]) / ABS
SELECT LENGTH(name) AS len, UPPER(name) AS name_upper, ROUND(odds, 1) AS odds_r FROM label.race
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
  `total_matched`（`WHERE` 後・`GROUP BY`/`LIMIT`/`OFFSET` 前の一致件数）は `OFFSET`/`LIMIT`/`DISTINCT` の
  影響を受けない。
- `SELECT DISTINCT col1, col2, ...` で、選択したカラムの値の組が重複する行をまとめる。重複判定は
  `LIMIT`/`OFFSET` を適用する前の出力行に対して行う。
- 集計関数 `COUNT(*)` / `COUNT(col)` / `SUM(col)` / `AVG(col)` / `MIN(col)` / `MAX(col)` に対応。
  `COUNT`/`SUM`/`AVG`/`MIN`/`MAX` は予約語ではなく、直後に `(` が続くときだけ集計関数として扱われる
  （`SELECT count FROM ...` のように、同名の通常カラムとしても引き続き使える）。
  - `COUNT(*)` は行数、`COUNT(col)` は `col` が `NULL`・未設定でない行数を数える。
  - `SUM`/`MIN`/`MAX` は対象カラムの値が全て整数なら整数を、`Float` が1つでも混ざれば小数を返す。
    `AVG` は常に小数を返す。対象カラムに数値（整数・小数）が1件も無ければ `NULL` を返す
    （`COUNT` は対象0件でも `0` を返す）。文字列・真偽値の列を渡した場合、その行は集計対象から除外される。
  - `GROUP BY col1, col2, ...` を省略した場合、集計関数は WHERE 適用後の全レコードを1つのグループとして
    計算する（`WHERE` の一致件数が0件でも `COUNT(*)` は `0` の1行を返す）。
  - `GROUP BY` に含まれない素のカラムを同時に SELECT した場合、標準SQLのようにはエラーにせず、
    各グループの代表レコード（先頭の1件）の値をそのまま返す（緩めの仕様）。
  - `HAVING <条件>` で集計後のグループを絞り込める。`WHERE` と同じ構文が使えるが、参照できるのは
    `GROUP BY` のカラム名と、集計関数に付けた `AS` エイリアスのみ（`HAVING COUNT(*) > 5` のように
    集計関数の呼び出し自体を直接書くことはできないため、必ずエイリアスを付けて名前で参照する）。
  - `ORDER BY` から集計関数の結果を参照する場合も同様に `AS` エイリアスが必要
    （`SELECT COUNT(*) AS n ... ORDER BY n` のように書く）。
  - `SELECT *` は `GROUP BY`/`HAVING` と組み合わせられない（集計対象・グループ代表値の区別がつかなくなるため）。
  - `SUM`/`AVG`/`MIN`/`MAX` に `*` は使えない（`COUNT(*)` のみ許可）。
- SELECT のカラムには算術演算・`CASE`式・`COALESCE`・`CAST`・スカラ関数を使った一般の式を書ける。
  式の結果は `AS` エイリアスがあればその名前、無ければ `expr1`/`expr2`... のような既定名で出力される
  （`ORDER BY` からもこの名前で参照できる）。
  - 算術演算子は `+` `-` `*` `/`（`*`/`/` が `+`/`-` より優先。`()` で優先順位を変更可能）。
    `-` を減算として使うときは前後どちらかに空白を入れる必要がある（`price -10` や `price-10` のように
    詰めて書くと、`-10` が負数リテラル1個として解釈されてしまいエラーになる）。`-age` のような単項マイナスも使える。
  - `/` は常に小数を返す（整数同士の除算でも切り捨てない）。ゼロ除算は `NULL` を返す。
  - どちらかのオペランドが数値でない場合（文字列・真偽値・`NULL`・未設定カラムなど）は `NULL` を返す。
  - `CASE WHEN <条件> THEN <式> [WHEN ... THEN ...]* [ELSE <式>] END`。条件部は `WHERE` 句と同じ構文
    （比較・`IS NULL`・`IN`・`BETWEEN`・`AND`/`OR`/`NOT`）が使える。`ELSE` 省略時、どの条件にも一致しなければ
    `NULL` を返す。
  - `COALESCE(expr1, expr2, ...)` は先頭から順に評価し、`NULL` でない最初の値を返す（全て `NULL` なら `NULL`）。
  - `CAST(expr AS <type>)` の `<type>` は `TEXT`（別名 `STRING`/`VARCHAR`）/`INTEGER`（別名 `INT`）/
    `FLOAT`（別名 `DOUBLE`/`REAL`）/`BOOLEAN`（別名 `BOOL`）。値が変換できない場合（`'abc'` を `INTEGER` へ
    等）は `NULL` を返す（エラーにはならない）。
  - スカラ関数: `LENGTH(x)`（文字数）・`LOWER(x)`・`UPPER(x)`・`SUBSTR(x, start[, length])`（`start` は1始まり。
    `SUBSTRING` も同義）・`ROUND(x[, digits])`（`digits` 省略時は0桁。常に小数を返す）・`ABS(x)`。
    `COUNT`/`SUM`/`AVG`/`MIN`/`MAX` と同様に予約語ではなく、直後に `(` が続くときだけ関数として扱われる。
  - **集計関数（`COUNT`/`SUM`/`AVG`/`MIN`/`MAX`）を式の内側に直接ネストすることはできない**
    （`ROUND(AVG(age), 0)` は不可）。代わりに集計関数へ `AS` でエイリアスを付け、別の式からそのエイリアスを
    カラム参照する（`AVG(age) AS avg_age, ROUND(avg_age, 0) AS avg_rounded` のように2項目に分ける）。
  - 先に指定した式のエイリアスを、後続の式からカラム参照として使うこともできる
    （`age * 2 AS doubled, doubled + 1 AS plus_one` のように左から順に評価される）。
- `EXPLAIN <SELECT文>` で、クエリを実際には実行せず実行計画（二次インデックスを使ったかどうか）だけを
  確認できる（詳細は[二次インデックス](#二次インデックス-create-index--drop-index--show-indexes--explain)を参照）。

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

## 二次インデックス (CREATE INDEX / DROP INDEX / SHOW INDEXES / EXPLAIN)

`POST /sql` では、あるカラムの等値検索（`WHERE column = value`）を高速化する二次インデックスの
作成・削除もできます（**`MANAGE_USERS` 権限が必要**。管理者 `kagura` は常に保持。
持たないユーザーが実行すると `403`。`CREATE USER` 等と同様、レコードストアの構造を操作する
管理操作として扱う）。

```sql
CREATE INDEX ON label.race (venue)
CREATE INDEX IF NOT EXISTS ON label.race (venue)
DROP INDEX ON label.race (venue)
DROP INDEX IF EXISTS ON label.race (venue)
SHOW INDEXES

EXPLAIN SELECT * FROM label.race WHERE venue = 'edogawa'
```

補足:
- インデックスは**カラム単位**で作成される（`label.race` の部分は他の SQL 構文と書き味を揃えるための
  表記で、`label.` は省略もできる。`CREATE INDEX ON venue (venue)` のようにラベル名は実際には使わず、
  対象は**ラベルをまたいで「そのカラム名を持つ全レコード」**になる。KAGURA DB がスキーマレスであることに
  合わせた設計）。
- 対応するのは**等値検索のみ**（`WHERE column = value`）。`<`/`>`/`BETWEEN` のような範囲検索や
  `LIKE`・`IS NULL` は現時点では高速化されない。
- `WHERE` 句**全体がちょうど1つの等値比較**のときだけインデックスが使われる。
  `WHERE venue = 'edogawa' AND date = '2026-01-01'` のように `AND`/`OR`/`NOT` を含む複合条件では
  使われず、通常の全件走査になる（今後の拡張余地）。
- インデックスの実体（値→行番号の対応表）は `.kdb`/JSON 本体には保存されない。定義（インデックスを
  張ったカラム名）だけをサイドカーファイル `<DB_FILE>.indexes.json` に保存し、サーバー起動時に
  そのファイルを読んで既存レコードから中身を再構築する（ラベルの検索索引と同じ方式）。
- `EXPLAIN <SELECT文>` は実際にはクエリを実行せず、`"index scan: <column> (equality) — N candidate row(s) ..."`
  または `"full scan — N row(s) scanned ..."` の1行を返す。権限は元の `SELECT` と同じ（`SELECT`）。

## 任意スキーマ層 (ALTER LABEL ... DEFINE COLUMN / ENABLE|DISABLE SCHEMA / DESCRIBE / SHOW SCHEMAS / VALIDATE LABEL)

`POST /sql` では、ラベルのカラムに型・`NOT NULL`・`DEFAULT`・`UNIQUE` を宣言し、`INSERT`/`UPDATE`
時に検証させることもできます（**`MANAGE_USERS` 権限が必要**。`CREATE USER`/`CREATE INDEX` 等と
同様、レコードストアの構造を操作する管理操作として扱う）。KAGURA DB は既定でスキーマレスのままで、
このスキーマは宣言したラベル・カラムにだけオプトインで効きます。

```sql
-- 1. スキーマを定義する（この時点ではまだ INSERT/UPDATE に影響しない）
ALTER LABEL race DEFINE COLUMN date TYPE text NOT NULL
ALTER LABEL race DEFINE COLUMN odds TYPE float DEFAULT 0.0
ALTER LABEL racer DEFINE COLUMN racer_id TYPE integer NOT NULL UNIQUE

-- 2. 定義を確認する
DESCRIBE label.race
SHOW SCHEMAS

-- 3. 既存データが定義に沿っているか、有効化する前に確認する（任意）
VALIDATE LABEL race

-- 4. 有効化する。ここで初めて race ラベルの INSERT/UPDATE が検証され始める
ALTER LABEL race ENABLE SCHEMA

-- 一時的に無効化することもできる（定義自体は消えない）
ALTER LABEL race DISABLE SCHEMA

-- 不要になった列定義を削除する（データそのものは削除しない）
ALTER LABEL race DROP COLUMN date
```

補足:
- **`DEFINE COLUMN` しただけでは強制されない**。`ENABLE SCHEMA` して初めて、そのラベルの
  `INSERT`/`UPDATE` が検証され始める。「定義してすぐ既存データが壊れる」事故を避けるため、
  定義と有効化を意図的に分けている。`DISABLE SCHEMA` すれば定義を残したまま検証だけ止められる。
  新しく `DEFINE COLUMN` したラベルは既定で無効（`DISABLE`）の状態から始まる。
- `TYPE` は `TEXT`（別名 `STRING`/`VARCHAR`）/`INTEGER`（別名 `INT`）/`FLOAT`（別名 `DOUBLE`/`REAL`）/
  `BOOLEAN`（別名 `BOOL`）。`Integer` の値は `Float` 宣言にも適合する（数値の自動昇格）。
- `NOT NULL` はそのカラムが未設定・`NULL` の `INSERT`/`UPDATE` を拒否する。`DEFAULT` が指定されていれば
  未設定時は自動的に補完されるため拒否されない。
- `UNIQUE` は同じラベルを持つレコード同士でのみ重複を禁止する（ラベルが違えば同じ値を持てる）。
  `UNIQUE` を指定すると、まだ二次インデックスが無ければ自動的に作成される（`CREATE INDEX` と同じ仕組みを
  再利用して重複チェックを高速に行うため）。
- `VALIDATE LABEL <label>` は、`ENABLE SCHEMA` する前に「今のデータが定義に沿っているか」を確認する
  診断コマンドで、何も変更しない（`ENABLE SCHEMA` 自体は既存データを自動チェックしないため、
  事前にこれで確認しておくことを推奨する）。違反が無ければ空の結果を返す。
- `DESCRIBE label.<label>` は定義済みのカラム一覧（型・`NOT NULL`・`DEFAULT`・`UNIQUE`・有効/無効）を、
  `SHOW SCHEMAS` はスキーマが定義されているラベルの一覧（有効/無効・カラム数）を返す。
  スキーマが未定義のラベルへの `DESCRIBE` は `404` を返す。
- スキーマ定義の実体は `.kdb`/JSON 本体には保存されない。定義だけをサイドカーファイル
  `<DB_FILE>.schema.json` に保存し、サーバー起動時にそのファイルを読んで再構築する
  （二次インデックスと同じ方式）。
- `PUT /settings` の `schema_enforcement_enabled`（既定 `true`）で、DB全体のスキーマ強制を
  一時停止できる。`false` にすると、個々のラベルの `ENABLE SCHEMA` 状態に関わらず全ての
  スキーマ検証が止まる（大量バックフィル時などに、ラベルごとの設定を変えずに一時退避する用途）。
  この設定はサーバー再起動のたびに `true` へ戻る（永続化しない）。
- Web UI（`/ui` 設定画面）の「スキーマ管理」セクションからも、カラムの定義・有効化/無効化・
  詳細表示・検証ができる。「スキーマ強制（全体設定）」セクションに `schema_enforcement_enabled` の
  トグルもある。

## トランザクション (BEGIN / COMMIT / ROLLBACK)

複数の `INSERT` / `UPDATE` / `DELETE` を1つの単位としてまとめ、「全部確定」または「全部取り消し」できる。

```sql
BEGIN;                 -- BEGIN TRANSACTION / START TRANSACTION も可
UPDATE label.inventory SET stock = stock - 1 WHERE id = 42;
INSERT INTO (label.orders) (item_id, qty) VALUE (42, 1);
COMMIT;                -- ここで初めて .kdb / JSON に永続化される
-- 途中でやめる場合は ROLLBACK（BEGIN 直後の状態に戻る）
```

- **同時に開けるトランザクションは1つだけ。** `BEGIN` を実行したログインユーザーが所有者になり、
  トランザクションが開いている間、**他ユーザーの `INSERT`/`UPDATE`/`DELETE` と別の `BEGIN` は
  `409 Conflict`** を返す。所有者以外の `COMMIT`/`ROLLBACK` も `409`。
- トランザクション中の `SELECT` は、所有者・他ユーザーを問わず **未コミットの変更が見える**
  （read-your-writes。分離レベルは低い＝ read uncommitted 相当）。
- **トランザクション中は DDL（`CREATE USER` などのユーザー管理・`CREATE INDEX` などのインデックス
  管理・`ALTER LABEL` などのスキーマ管理）を実行できない**（`409`）。先に `COMMIT`/`ROLLBACK` する。
- `BEGIN`〜`COMMIT` の間は `.kdb`/JSON を一切書き換えない。`COMMIT` 時に一度だけまとめて永続化する。
  `ROLLBACK` はディスクから読み直すだけ（＝ BEGIN 前の状態に戻る）。
- **無操作タイムアウト**: 最後の文から 300 秒操作が無いと、次の書き込み系リクエスト（他ユーザー含む）を
  受けた時点で自動的に `ROLLBACK` される（ハングしたクライアントがサーバー全体の書き込みを
  止め続けるのを防ぐため）。サーバー停止時も未永続化のため実質ロールバックされる。
- 「全データオンメモリ」の容量制限モードでも、トランザクション中はメモリからの追い出しを止める
  （まだ永続化されていない行が失われないようにするため）。`COMMIT`/`ROLLBACK` 後に上限まで削り直す。
- **v1 の制限**: `/sql`（＝Web UI の SQL Query ビュー）からのみ利用可能。`kdb` CLI・`db_ffi`
  （Python バインディング）のトランザクション対応は未実装。`COMMIT` 時の `.kdb` 書き込みは
  既存の `UPDATE`/`DELETE` と同じ全書き直し（compact）方式で、途中クラッシュに対する原子性は
  既存の書き込みと同レベル。

## バックアップ／復元 (BACKUP TO / RESTORE FROM)

`.kdb` 本体・サイドカーファイル・`auth.json` を1つの `.kbak` アーカイブにまとめる／戻す。
**管理者ロール `kagura` のみ**、**トランザクション中は `409`**。

```sql
BACKUP TO '/var/lib/kagura-db/backups/'              -- ディレクトリなら自動命名
BACKUP TO '/tmp/snapshot.kbak' WITH KEY              -- マスターキーを同梱（取り扱い注意）
BACKUP TO '/backups/' WITHOUT AUTH                   -- auth.json を含めない

RESTORE FROM '/backups/kagura-backup-20260908-120000.kbak'
RESTORE FROM '/backups/....kbak' OLD KEY '<hex64>'   -- 作成時のマスターキー（WITH KEY なら不要）
```

- `RESTORE` は現ファイルを `pre-restore-<timestamp>/` へ退避してから置き換える。
  **反映にはサーバーの再起動が必要**で、適用後は書き込み系リクエストが `409`（`SELECT` は可能）。
- 復元先のマスターキーが作成時と異なる場合、旧キーで復号し復元先のキーで再暗号化（リキー）する。
- メジャーバージョンが異なる `.kbak` は復元を拒否する。
- 詳細・災害復旧の手順は [backup.md](backup.md) を参照。

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
