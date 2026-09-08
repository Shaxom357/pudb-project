# 開発・テスト

- [テスト](#テスト)
- [テストデータ生成](#テストデータ生成)
- [Python バインディング](#python-バインディング)
- [Python から暗号化 .kdb を使う（KdbEngine）](#python-から暗号化-kdb-を使うkdbengine)

---

## テスト

```bash
cargo test --workspace
```

| クレート | テスト数 | 内容 |
|----------|----------|---------|
| `db_engine` | 43 | CRUD・ラベル操作（重複付与エラーを含む）・KDB暗号化・バイナリコーデック・マスターキーのユーティリティ／リキー（バックアップ基盤） |
| `dynamic_label_management` | 24 | AND/OR 検索・リネーム・差分・ラベル重複付与エラー |
| `db_ffi` | 20 | FFI 関数・メモリ管理・`.kdb` セッション（WAL 追記・`kdb_sql`・NDJSON エクスポート・advisory lock） |
| `db_client`（ユニットテスト） | 44 | ログ出力保管機能（JSON Lines 書き込み/読み出し・HW起因エラー判定・authカテゴリ記録）・認証（ログイン成功/失敗・トークン検証・ログアウト・ロックアウト・パスワード変更・複数ユーザー・権限付与/剥奪）・ユーザー管理SQL（CREATE/DROP/ALTER USER・SHOW USERS・GRANT/REVOKE のパースと実行） |
| `db_client`（統合テスト） | 77 | HTTP API・ラベル操作（重複付与エラーを含む）・永続化・SQL SELECT/INSERT/UPDATE/DELETE（ラベル重複エラー・UPDATE LABELのWHERE絞り込み・DELETE/DELETE LABELのWHERE絞り込みを含む）・設定（HTTPリクエスト受付オンオフ）・ログ（`GET /logs`）・認証（未ログイン時の401・ログイン/ログアウト・ロックアウト・パスワード変更後の再ログイン必須化・ログイン成功/失敗のログ記録）・一般ユーザー管理（CREATE/DROP/ALTER USER・SHOW USERS・GRANT/REVOKE と `/sql`・REST API の権限強制） |
| `sql_engine` | 60 | パーサー・実行エンジン（SELECT・INSERT・UPDATE・DELETE・WHERE・ORDER BY・LIKE・ラベル重複エラー・UPDATE LABEL/DELETE LABELのWHERE絞り込み） |
| **合計** | **268** | |

---

## テストデータ生成

`generate_testdata.py` は実行時に自動で `POST /auth/login`（既定 `kagura` / `root`）してから投入する。
初期パスワードを変更済みの場合は `--username`/`--password` を指定する。

```bash
# 10,000件生成 + 性能テスト
python3 examples/python/generate_testdata.py --bench

# 性能テストのみ
python3 examples/python/generate_testdata.py --bench-only --repeat 5
```

### 事前生成済みJSONの投入（再インストール後のデータ再投入など）

`examples/testdata/kagura_testdata_10000.json` に、`POST /records` へそのまま投げられる
形式（`{"columns": {...}, "labels": [...]}` の配列）で10,000件のテストデータを同梱しています。
インストールし直した直後など、ランダム生成をやり直さずに同じデータを再投入したい場合に使えます。

```bash
# ログインしてトークンを取得し、HTTPリクエスト(REST API)を有効化してから投入（既定で無効のため）
TOKEN=$(curl -s -X POST http://localhost:3000/auth/login -H 'Content-Type: application/json' \
  -d '{"username":"kagura","password":"root"}' | python3 -c 'import json,sys;print(json.load(sys.stdin)["token"])')
curl -X PUT http://localhost:3000/settings -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' -d '{"http_api_enabled": true}'

# 同梱JSONを読み込んで投入（generate_testdata.py 自身がログインするため上記トークンは不要）
python3 examples/python/generate_testdata.py \
  --load-json examples/testdata/kagura_testdata_10000.json \
  --url http://localhost:3000

# 新しいJSONを作り直したい場合（サーバー接続不要）
python3 examples/python/generate_testdata.py --dump-json testdata_10000.json --count 10000
```

**依存**: Python 3.6+ 標準ライブラリのみ（pip 不要）

---

## Python バインディング

```bash
cargo build --release -p db_ffi
cd examples/python && python3 demo.py
```

```python
from db_engine import DbEngine
db = DbEngine()
id = db.insert({"name": "Alice", "age": 30}, labels=["dept:engineering"])
print(db.get(id))
db.save("/tmp/kagura.json")
```

`DbEngine` は平文 JSON・全書き直しのみ。暗号化 `.kdb`・WAL 追記・SQL を Python から使う場合は
次の `KdbEngine` を使う。

---

## Python から暗号化 .kdb を使う（KdbEngine）

`examples/python/kdb_engine.py` の `KdbEngine` は、暗号化 `.kdb` ストレージ（WAL 追記・SQL・
NDJSON エクスポート）を `ctypes` 経由で利用するラッパー。KAGURA DB を唯一のデータストアにして
Python の取込ワーカーや ML から読み書きする用途向け。

```bash
cargo build --release -p db_ffi
```

```python
from kdb_engine import KdbEngine

db = KdbEngine.open("race.kdb")                       # 無ければ新規作成

# 書き込み（いずれも WAL 追記 = O(1)）
db.insert({"race_no": 1, "payout": 1200}, labels=["race", "y2025", "d20250906"])
db.insert_many([                                       # 大量バックフィル用（-> 成功件数）
    {"columns": {"race_no": 2, "payout": 870},  "labels": ["race", "y2025"]},
    {"columns": {"race_no": 3, "payout": 1540}, "labels": ["race", "y2025"]},
])

# SQL（JOIN・GROUP BY 非対応。WHERE / ORDER BY / LIMIT / ラベル AND-OR は可）
rows = db.select("SELECT * FROM label.race WHERE race_no >= 2 ORDER BY race_no LIMIT 12")
db.sql("UPDATE label.race SET payout = 9999 WHERE race_no = 1")
db.sql("DELETE FROM label.d20250906")                 # その日の分を消して冪等に再取込

# ラベルスライスの取得
recs = db.get_by_label("y2024")                        # 型情報付き dict のリスト
n = db.get_by_label_ndjson("y2024", "/tmp/y2024.ndjson")   # -> 件数

db.count(); db.save(); db.close()

# with 文なら close を保証
with KdbEngine.open("race.kdb") as db:
    ...
```

### `get_by_label_ndjson` と polars

`ctypes` 越しに年 40 万件クラスの巨大 JSON 文字列を返すのは重いため、`get_by_label_ndjson` は
**`SELECT *` 相当のフラット NDJSON**（1 行 = `{"id": .., <各カラム>, "labels": [..]}`）を
ファイルに書き出す。Python 側は polars で遅延読み込みする。

```python
import polars as pl
lf = pl.scan_ndjson("/tmp/y2024.ndjson")
```

- 数値カラムに `null` が混じる列、またはスライス内で全て `null` の列は型推論が不安定になりうる。
  その場合は `pl.scan_ndjson(path, infer_schema_length=None)` か明示スキーマを渡す。
- `id` / `labels` は予約キー。同名のカラムを作っているとカラム側の値で上書きされる
  （`SELECT *` の表示列と同じ挙動）。厳密な型が必要なら `get_by_label()` か `sql()` を使う。

### 単一ライター制約（重要）

`.kdb` は WAL 追記方式のため、**同じファイルに同時に書き込めるプロセスは 1 つだけ**。
あるプロセスが `KdbEngine.open`（= `kdb_open`）でファイルを開いている間は、同じ `.kdb` に対して
`db_client` サーバーや別の `KdbEngine` を起動しないこと。WAL が二重に書かれてファイルが破損する。

これを補助するため `kdb_open` は `<path>.lock` を排他作成する advisory lock を張る。
別プロセスがロック中なら `KdbEngine.open` は `RuntimeError` を送出する。`close()`（`with` 文なら
ブロック終了時）でロックは解放される。プロセスが強制終了（SIGKILL 等）してロックファイルが
残った場合は、他に開いているプロセスが無いことを確認したうえで `<path>.lock` を手動削除する。

**依存**: `KdbEngine` 自体は Python 標準ライブラリのみ（`polars` は NDJSON を読む場合のみ）。
