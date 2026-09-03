# 開発・テスト

- [テスト](#テスト)
- [テストデータ生成](#テストデータ生成)
- [Python バインディング](#python-バインディング)

---

## テスト

```bash
cargo test --workspace
```

| クレート | テスト数 | 内容 |
|----------|----------|---------|
| `db_engine` | 39 | CRUD・ラベル操作（重複付与エラーを含む）・KDB暗号化・バイナリコーデック |
| `dynamic_label_management` | 24 | AND/OR 検索・リネーム・差分・ラベル重複付与エラー |
| `db_ffi` | 10 | FFI 関数・メモリ管理 |
| `db_client`（ユニットテスト） | 44 | ログ出力保管機能（JSON Lines 書き込み/読み出し・HW起因エラー判定・authカテゴリ記録）・認証（ログイン成功/失敗・トークン検証・ログアウト・ロックアウト・パスワード変更・複数ユーザー・権限付与/剥奪）・ユーザー管理SQL（CREATE/DROP/ALTER USER・SHOW USERS・GRANT/REVOKE のパースと実行） |
| `db_client`（統合テスト） | 77 | HTTP API・ラベル操作（重複付与エラーを含む）・永続化・SQL SELECT/INSERT/UPDATE/DELETE（ラベル重複エラー・UPDATE LABELのWHERE絞り込み・DELETE/DELETE LABELのWHERE絞り込みを含む）・設定（HTTPリクエスト受付オンオフ）・ログ（`GET /logs`）・認証（未ログイン時の401・ログイン/ログアウト・ロックアウト・パスワード変更後の再ログイン必須化・ログイン成功/失敗のログ記録）・一般ユーザー管理（CREATE/DROP/ALTER USER・SHOW USERS・GRANT/REVOKE と `/sql`・REST API の権限強制） |
| `sql_engine` | 60 | パーサー・実行エンジン（SELECT・INSERT・UPDATE・DELETE・WHERE・ORDER BY・LIKE・ラベル重複エラー・UPDATE LABEL/DELETE LABELのWHERE絞り込み） |
| **合計** | **254** | |

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
