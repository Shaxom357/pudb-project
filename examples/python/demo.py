#!/usr/bin/env python3
"""
demo.py - DbEngine Python ラッパーのデモスクリプト

実行方法:
    cd examples/python
    python3 demo.py
"""
import sys, os, json, tempfile
sys.path.insert(0, os.path.dirname(__file__))
from db_engine import DbEngine

def section(title):
    print(f"\n{'='*50}")
    print(f"  {title}")
    print('='*50)

# ---------------------------------------------------------------------------
# 1. レコードの作成
# ---------------------------------------------------------------------------
section("1. レコードの挿入")
db = DbEngine()

id1 = db.insert(
    {"name": "Alice", "age": 30, "score": 95.5, "active": True},
    labels=["dept:engineering", "role:backend", "env:prod"]
)
id2 = db.insert(
    {"name": "Bob", "age": 25, "score": 88.0},
    labels=["dept:engineering", "role:frontend", "env:prod"]
)
id3 = db.insert(
    {"name": "Carol", "age": 35},
    labels=["dept:sales", "env:staging"]
)

print(f"挿入完了: id1={id1}, id2={id2}, id3={id3}")
print(f"レコード数: {db.count()}")

# ---------------------------------------------------------------------------
# 2. ID で取得
# ---------------------------------------------------------------------------
section("2. ID でレコードを取得")
record = db.get(id1)
print(f"get({id1}):")
print(f"  name   = {record['columns']['name']}")
print(f"  age    = {record['columns']['age']}")
print(f"  score  = {record['columns']['score']}")
print(f"  labels = {record['labels']}")

not_found = db.get(9999)
print(f"get(9999) = {not_found}")  # None

# ---------------------------------------------------------------------------
# 3. ラベルで検索
# ---------------------------------------------------------------------------
section("3. ラベルで検索")
results = db.get_by_label("dept:engineering")
print(f"dept:engineering の件数: {len(results)}")
for r in results:
    print(f"  id={r['id']} name={r['columns']['name']}")

# ---------------------------------------------------------------------------
# 4. レコードの更新
# ---------------------------------------------------------------------------
section("4. レコードの更新")
ok = db.update(id1,
    {"name": "Alice (updated)", "score": 99.9},
    labels=["dept:engineering", "role:tech-lead", "env:prod"]
)
print(f"update({id1}): {ok}")
updated = db.get(id1)
print(f"  name  = {updated['columns']['name']}")
print(f"  score = {updated['columns']['score']}")
print(f"  labels= {updated['labels']}")

# ---------------------------------------------------------------------------
# 5. レコードの削除
# ---------------------------------------------------------------------------
section("5. レコードの削除")
ok = db.delete(id3)
print(f"delete({id3}): {ok}")
print(f"削除後のレコード数: {db.count()}")
print(f"get({id3}) = {db.get(id3)}")  # None

# ---------------------------------------------------------------------------
# 6. 全レコード一覧
# ---------------------------------------------------------------------------
section("6. 全レコード一覧")
for r in db.list_all():
    print(f"  id={r['id']} name={r['columns'].get('name')} labels={r['labels']}")

# ---------------------------------------------------------------------------
# 7. ラベル一覧
# ---------------------------------------------------------------------------
section("7. ラベル一覧")
print(db.list_labels())

# ---------------------------------------------------------------------------
# 8. 永続化（保存 → 再ロード）
# ---------------------------------------------------------------------------
section("8. 永続化テスト（save → load）")
with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as f:
    tmp_path = f.name

db.save(tmp_path)
print(f"保存先: {tmp_path}")

# ファイルの中身を確認
with open(tmp_path) as f:
    saved = json.load(f)
print(f"保存されたレコード数: {len(saved['records'])}")

# 再ロード
db2 = DbEngine.load(tmp_path)
print(f"再ロード後のレコード数: {db2.count()}")
r = db2.get(id1)
print(f"  id={r['id']} name={r['columns']['name']} labels={r['labels']}")

# ラベル検索もインデックスが復元されている
restored = db2.get_by_label("dept:engineering")
print(f"  dept:engineering (再ロード後): {len(restored)} 件")

os.unlink(tmp_path)
print("\n✅ 全デモ完了!")
