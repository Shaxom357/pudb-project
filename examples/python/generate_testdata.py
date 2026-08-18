#!/usr/bin/env python3
"""
KAGURA DB  10,000件テストデータ自動生成スクリプト
================================================
使い方:
  python3 generate_testdata.py                 # デフォルト(10,000件)
  python3 generate_testdata.py --count 5000    # 件数変更
  python3 generate_testdata.py --url http://localhost:3001
  python3 generate_testdata.py --workers 20   # 並列数変更
  python3 generate_testdata.py --bench-only   # 性能テストのみ
  python3 generate_testdata.py --bench        # 生成後に性能テストも実行

  # サーバーに接続せず、POST /records にそのまま投げられる形の
  # JSON配列ファイルを生成するだけ（インストール後の再投入などに利用）
  python3 generate_testdata.py --dump-json testdata_10000.json

  # 生成済みのJSONファイルを読み込んで投入（ランダム再生成せず、同じデータを再投入したい場合）
  python3 generate_testdata.py --load-json testdata_10000.json --url http://localhost:3000

依存: Python 3.6+ 標準ライブラリのみ（pip不要）
"""

import argparse
import json
import random
import sys
import time
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor, as_completed

# ===========================================================================
# ★ デフォルト設定（変更可能）
# ===========================================================================
DEFAULT_URL     = "http://localhost:3000"
DEFAULT_COUNT   = 10_000
DEFAULT_WORKERS = 16
PROGRESS_STEP   = 200
RANDOM_SEED     = 42

# ===========================================================================
# マスターデータ
# ===========================================================================
FIRST_NAMES = [
    "田中", "佐藤", "鈴木", "高橋", "伊藤", "山本", "中村", "小林", "加藤", "吉田",
    "山田", "佐々木", "松本", "井上", "木村", "林", "清水", "山崎", "池田", "渡辺",
    "Alice", "Bob", "Carol", "Dave", "Eve", "Frank", "Grace", "Heidi",
]
DEPARTMENTS = [
    "engineering", "sales", "hr", "finance", "marketing",
    "legal", "ops", "support", "design", "research",
]
CITIES_JP = ["Tokyo", "Osaka", "Nagoya", "Fukuoka", "Sapporo",
             "Yokohama", "Kyoto", "Kobe", "Hiroshima", "Sendai"]
CITIES_US = ["NewYork", "LosAngeles", "Chicago", "Houston", "Phoenix"]
CITIES    = CITIES_JP + CITIES_US
PRODUCTS  = [
    "Cola", "Coffee", "Tea", "Water", "Juice", "Beer", "Wine",
    "Laptop", "Mouse", "Keyboard", "Monitor", "Headphone",
    "Shirt", "Pants", "Shoes", "Bag", "Watch", "Book", "Pen",
]
CATEGORIES = {
    "Cola": "drink", "Coffee": "drink", "Tea": "drink",
    "Water": "drink", "Juice": "drink", "Beer": "drink", "Wine": "drink",
    "Laptop": "electronics", "Mouse": "electronics", "Keyboard": "electronics",
    "Monitor": "electronics", "Headphone": "electronics",
    "Shirt": "fashion", "Pants": "fashion", "Shoes": "fashion",
    "Bag": "fashion", "Watch": "fashion",
    "Book": "stationery", "Pen": "stationery",
}
STATUSES     = ["active", "inactive", "pending", "suspended"]
SENSOR_TYPES = ["temperature", "humidity", "pressure", "vibration", "co2"]
LOG_LEVELS   = ["INFO", "WARN", "ERROR", "DEBUG"]
REGIONS      = ["region-east", "region-west", "region-north", "region-south"]
ENVS         = ["env-prod", "env-staging", "env-dev"]

# ===========================================================================
# ヘルパー関数
# ===========================================================================

def txt(c):             return {"type": "text",    "value": random.choice(c)}
def i64(lo, hi):        return {"type": "integer", "value": random.randint(lo, hi)}
def f64(lo, hi, d=2):   return {"type": "float",   "value": round(random.uniform(lo, hi), d)}
def bln():              return {"type": "boolean", "value": random.random() < 0.5}
def null():             return {"type": "null",    "value": None}
def maybe_null(fn, p=0.05): return null() if random.random() < p else fn()


# ===========================================================================
# シナリオ別レコード生成関数
# ===========================================================================

def make_employee(i):
    dept = random.choice(DEPARTMENTS)
    age  = random.randint(22, 65)
    lbls = ["employee", f"dept:{dept}", f"age_group:{(age//10)*10}s"]
    if age >= 40 and random.random() < 0.3: lbls.append("manager")
    if random.random() < 0.1: lbls.append("remote")
    return {"columns": {
        "employee_name":     {"type":"text","value": random.choice(FIRST_NAMES)},
        "age":               {"type":"integer","value": age},
        "department":        txt(DEPARTMENTS),
        "city":              txt(CITIES),
        "salary":            i64(300_000, 1_200_000),
        "is_active":         bln(),
        "years_at_company":  i64(0, 35),
        "performance_score": f64(1.0, 5.0),
        "email":             {"type":"text","value": f"user{i}@example.com"},
    }, "labels": lbls}

def make_product(i):
    name = random.choice(PRODUCTS)
    cat  = CATEGORIES.get(name, "other")
    lbls = ["product", f"category:{cat}"]
    if random.random() < 0.3: lbls.append("sale")
    if random.random() < 0.1: lbls.append("limited")
    return {"columns": {
        "product_name": {"type":"text","value": name},
        "price":        i64(100, 150_000),
        "stock":        i64(0, 5000),
        "category":     {"type":"text","value": cat},
        "rating":       f64(1.0, 5.0),
        "is_available": bln(),
        "weight_kg":    maybe_null(lambda: f64(0.1, 20.0)),
        "sku":          {"type":"text","value": f"SKU-{i:06d}"},
    }, "labels": lbls}

def make_order(i):
    status = random.choice(STATUSES)
    lbls   = ["order", f"status:{status}"]
    if random.random() < 0.5: lbls.append(random.choice(REGIONS))
    return {"columns": {
        "order_id":    {"type":"text","value": f"ORD-{i:07d}"},
        "customer_id": i64(1, 5000),
        "amount":      i64(500, 500_000),
        "status":      {"type":"text","value": status},
        "items_count": i64(1, 20),
        "is_paid":     bln(),
        "discount":    maybe_null(lambda: f64(0.0, 0.5), p=0.6),
        "year":        i64(2020, 2026),
        "month":       i64(1, 12),
    }, "labels": lbls}

def make_customer(i):
    city    = random.choice(CITIES)
    country = "Japan" if city in CITIES_JP else "USA"
    lbls    = ["customer", f"country:{country}"]
    if random.random() < 0.2: lbls.append("vip")
    if random.random() < 0.1: lbls.append("churned")
    return {"columns": {
        "customer_name":  txt(FIRST_NAMES),
        "email":          {"type":"text","value": f"cust{i}@mail.com"},
        "city":           {"type":"text","value": city},
        "country":        {"type":"text","value": country},
        "age":            maybe_null(lambda: i64(18, 90)),
        "total_purchase": i64(0, 5_000_000),
        "is_subscribed":  bln(),
        "loyalty_points": i64(0, 100_000),
    }, "labels": lbls}

def make_sensor(i):
    stype  = random.choice(SENSOR_TYPES)
    region = random.choice(REGIONS)
    lbls   = [f"sensor:{stype}", region, "iot"]
    if random.random() < 0.05: lbls.append("alert")
    vmap = {"temperature":f64(-10.0,45.0),"humidity":f64(10.0,99.9),
            "pressure":f64(900.0,1100.0),"vibration":f64(0.0,10.0),"co2":f64(300.0,5000.0)}
    umap = {"temperature":"C","humidity":"%","pressure":"hPa","vibration":"m/s2","co2":"ppm"}
    return {"columns": {
        "sensor_id":   {"type":"text","value": f"SEN-{i:05d}"},
        "sensor_type": {"type":"text","value": stype},
        "value":       vmap[stype],
        "unit":        {"type":"text","value": umap[stype]},
        "device_id":   i64(1, 500),
        "is_anomaly":  bln(),
        "battery_pct": maybe_null(lambda: i64(1, 100), p=0.1),
    }, "labels": lbls}

def make_log(i):
    level = random.choice(LOG_LEVELS)
    env   = random.choice(ENVS)
    lbls  = [f"log:{level.lower()}", env, "system_log"]
    if level == "ERROR": lbls.append("alert")
    return {"columns": {
        "log_id":      {"type":"text","value": f"LOG-{i:08d}"},
        "level":       {"type":"text","value": level},
        "service":     txt(["api","db","auth","worker","scheduler"]),
        "response_ms": i64(1, 10_000),
        "status_code": txt(["200","201","400","401","403","404","500","503"]),
        "is_error":    {"type":"boolean","value": level == "ERROR"},
        "retry_count": maybe_null(lambda: i64(0, 5), p=0.7),
    }, "labels": lbls}

def make_article(i):
    lbls = ["article", random.choice(["tech","business","science","sports","culture"])]
    if random.random() < 0.2: lbls.append("featured")
    if random.random() < 0.3: lbls.append(random.choice(ENVS))
    return {"columns": {
        "title":        {"type":"text","value": f"Article Title {i}"},
        "author":       txt(FIRST_NAMES),
        "views":        i64(0, 1_000_000),
        "likes":        i64(0, 50_000),
        "word_count":   i64(300, 10_000),
        "is_published": bln(),
        "score":        f64(0.0, 10.0),
    }, "labels": lbls}

def make_task(i):
    status = random.choice(["todo","in_progress","done","cancelled"])
    lbls   = [f"task:{status}", random.choice(DEPARTMENTS), "project"]
    if random.random() < 0.2: lbls.append("high_priority")
    return {"columns": {
        "task_name":    {"type":"text","value": f"Task-{i:05d}"},
        "assignee":     txt(FIRST_NAMES),
        "priority":     i64(1, 5),
        "status":       {"type":"text","value": status},
        "estimated_h":  f64(0.5, 80.0),
        "is_blocked":   bln(),
        "sprint":       i64(1, 50),
        "story_points": maybe_null(lambda: i64(1, 13), p=0.2),
    }, "labels": lbls}

def make_transaction(i):
    status = random.choice(["completed","pending","failed","refunded"])
    lbls   = ["transaction", f"tx:{status}"]
    if random.random() < 0.3: lbls.append(random.choice(REGIONS))
    return {"columns": {
        "tx_id":      {"type":"text","value": f"TX-{i:09d}"},
        "amount":     i64(100, 1_000_000),
        "fee":        f64(0.0, 500.0),
        "status":     {"type":"text","value": status},
        "currency":   txt(["JPY","USD","EUR"]),
        "is_fraud":   {"type":"boolean","value": random.random() < 0.01},
        "account_id": i64(1000, 9999),
    }, "labels": lbls}

def make_device(i):
    lbls = ["device", random.choice(["mobile","desktop","tablet"]), random.choice(ENVS)]
    if random.random() < 0.15: lbls.append("offline")
    return {"columns": {
        "device_id":     {"type":"text","value": f"DEV-{i:06d}"},
        "os":            txt(["iOS","Android","Windows","macOS","Linux"]),
        "version":       {"type":"text","value": f"{random.randint(1,15)}.{random.randint(0,9)}.{random.randint(0,9)}"},
        "is_active":     bln(),
        "storage_gb":    i64(16, 2048),
        "battery_pct":   maybe_null(lambda: i64(1, 100), p=0.2),
        "last_seen_ago": i64(0, 86_400),
    }, "labels": lbls}

# ===========================================================================
# シナリオのウエイト配分
# ===========================================================================

SCENARIOS = [
    (make_employee,    0.20),  # 2000件
    (make_product,     0.15),  # 1500件
    (make_order,       0.15),  # 1500件
    (make_customer,    0.12),  # 1200件
    (make_sensor,      0.12),  # 1200件
    (make_log,         0.10),  # 1000件
    (make_article,     0.07),  #  700件
    (make_task,        0.05),  #  500件
    (make_transaction, 0.02),  #  200件
    (make_device,      0.02),  #  200件
]

def build_record_list(total):
    records = []
    for fn, weight in SCENARIOS:
        count = round(total * weight)
        for j in range(count):
            records.append(fn(len(records) + 1))
    while len(records) < total:
        fn, _ = random.choice(SCENARIOS)
        records.append(fn(len(records) + 1))
    records = records[:total]
    random.shuffle(records)
    return records

# ===========================================================================
# HTTP ユーティリティ
# ===========================================================================

def post_record(url, payload, timeout=30):
    data = json.dumps(payload).encode("utf-8")
    req  = urllib.request.Request(
        f"{url}/records", data=data,
        headers={"Content-Type": "application/json"}, method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return (resp.status in (200, 201), f"HTTP {resp.status}")
    except urllib.error.HTTPError as e:
        return (False, f"HTTP {e.code}: {e.read().decode()[:100]}")
    except Exception as e:
        return (False, str(e)[:100])

def get_json(url, path):
    try:
        with urllib.request.urlopen(f"{url}{path}", timeout=30) as resp:
            return json.loads(resp.read().decode())
    except Exception:
        return None

def post_sql(url, query):
    data = json.dumps({"query": query}).encode("utf-8")
    req  = urllib.request.Request(
        f"{url}/sql", data=data,
        headers={"Content-Type": "application/json"}, method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=60) as resp:
            return json.loads(resp.read().decode())
    except Exception as e:
        return {"ok": False, "error": str(e)}

# ===========================================================================
# 進捗表示クラス
# ===========================================================================

class Progress:
    def __init__(self, total):
        self.total    = total
        self.done     = 0
        self.errors   = 0
        self.start_ts = time.perf_counter()

    def update(self, success):
        if success: self.done   += 1
        else:       self.errors += 1

    def print(self, force=False):
        completed = self.done + self.errors
        if not force and completed % PROGRESS_STEP != 0:
            return
        elapsed = time.perf_counter() - self.start_ts
        pct     = completed / self.total * 100
        rps     = completed / elapsed if elapsed > 0 else 0
        eta     = (self.total - completed) / rps if rps > 0 else float("inf")
        bar_w   = 30
        filled  = int(bar_w * completed / self.total)
        bar     = "#" * filled + "." * (bar_w - filled)
        eta_str = f"{eta:.0f}s" if eta < 3600 else "N/A"
        sys.stdout.write(
            f"\r  [{bar}] {pct:5.1f}%  "
            f"{completed:>6}/{self.total}  "
            f"{rps:5.1f} req/s  ETA:{eta_str}  "
            f"err:{self.errors}"
        )
        sys.stdout.flush()


# ===========================================================================
# メイン生成処理
# ===========================================================================

def _fmt_bytes(b):
    if b is None: return "N/A"
    if b < 1024:      return f"{b} B"
    if b < 1024**2:   return f"{b/1024:.1f} KB"
    if b < 1024**3:   return f"{b/1024**2:.2f} MB"
    return f"{b/1024**3:.2f} GB"

def load_records_from_file(path):
    """--dump-json で保存したJSON配列（POST /records にそのまま投げられる形）を読み込む"""
    with open(path, "r", encoding="utf-8") as f:
        records = json.load(f)
    if not isinstance(records, list):
        print(f"  X {path} はJSON配列ではありません。")
        sys.exit(1)
    return records


def dump_records_to_file(records, path):
    """POST /records にそのまま投げられる形のJSON配列としてファイルへ保存する"""
    with open(path, "w", encoding="utf-8") as f:
        json.dump(records, f, ensure_ascii=False, indent=2)


def generate(url, total, workers, records=None):
    print(f"\n{chr(61)*60}")
    print(f"  KAGURA DB テストデータ生成")
    print(f"{chr(61)*60}")
    print(f"  対象URL   : {url}")
    print(f"  生成件数  : {total:,} 件")
    print(f"  並列数    : {workers} threads")
    print(f"{chr(61)*60}\n")

    print("  [1/3] サーバー接続確認...", end=" ", flush=True)
    info = get_json(url, "/db/info")
    if info is None:
        print(f"\n  X 接続失敗: {url} に到達できません。サーバー起動を確認してください。")
        sys.exit(1)
    existing = info.get("record_count", 0)
    print(f"OK  (既存レコード: {existing:,} 件)")

    if records is None:
        print(f"\n  [2/3] レコードデータ生成中...", end=" ", flush=True)
        t0 = time.perf_counter()
        records = build_record_list(total)
        print(f"完了 ({time.perf_counter() - t0:.2f}s)")
    else:
        print(f"\n  [2/3] ファイルから読み込んだ {len(records):,} 件を使用します")
        total = len(records)

    label_count = {}
    for r in records:
        for lbl in r.get("labels", []):
            label_count[lbl] = label_count.get(lbl, 0) + 1
    top = sorted(label_count.items(), key=lambda x: -x[1])[:15]
    print("  ラベル内訳(上伕15):")
    for lbl, cnt in top:
        bar = "|" * int(cnt / total * 50)
        print(f"    {lbl:<30} {cnt:>5}件  {bar}")

    print(f"\n  [3/3] データ投入開始 ({workers} threads)...")
    prog = Progress(total)
    t_start = time.perf_counter()

    with ThreadPoolExecutor(max_workers=workers) as ex:
        futures = {ex.submit(post_record, url, rec): rec for rec in records}
        for fut in as_completed(futures):
            ok, _ = fut.result()
            prog.update(ok)
            prog.print()

    elapsed = time.perf_counter() - t_start
    prog.print(force=True)
    print()

    print(f"\n{chr(61)*60}")
    print(f"  投入完了サマリー")
    print(f"{chr(61)*60}")
    print(f"  成功  : {prog.done:>7,} 件")
    print(f"  失敗  : {prog.errors:>7,} 件")
    print(f"  所要時間: {elapsed:.1f} 秒")
    if elapsed > 0:
        print(f"  スループット: {prog.done / elapsed:.1f} req/s")

    info2 = get_json(url, "/db/info")
    if info2:
        print(f"\n  DB状態:")
        print(f"    総レコード数  : {info2.get('record_count','?'):>8,} 件")
        print(f"    ラベル種類数  : {info2.get('label_count','?'):>8,} 種")
        print(f"    カラム種類数  : {info2.get('column_count','?'):>8,} 種")
        print(f"    ファイルサイズ: {_fmt_bytes(info2.get('db_file_size_bytes'))}")

    return {"success": prog.done, "error": prog.errors, "elapsed": elapsed}


# ===========================================================================
# SQL 性能テスト
# ===========================================================================

BENCH_QUERIES = [
    ("SELECT * FROM label.* (LIMIT 100)",
     "SELECT * FROM label.* LIMIT 100"),
    ("SELECT * FROM label.employee",
     "SELECT * FROM label.employee"),
    ("employee AND manager",
     "SELECT * FROM label.employee AND label.manager"),
    ("employee OR customer",
     "SELECT * FROM label.employee OR label.customer"),
    ("WHERE 等値 (employee_name = '田中')",
     "SELECT * FROM label.employee WHERE employee_name = '田中'"),
    ("WHERE 範囲 (age > 40)",
     "SELECT * FROM label.employee WHERE age > 40"),
    ("WHERE 複合 AND",
     "SELECT * FROM label.employee WHERE age > 30 AND city = 'Tokyo'"),
    ("WHERE LIKE '田%'",
     "SELECT * FROM label.employee WHERE employee_name LIKE '田%'"),
    ("ORDER BY salary DESC LIMIT 50",
     "SELECT * FROM label.employee ORDER BY salary DESC LIMIT 50"),
    ("product AND sale",
     "SELECT * FROM label.product AND label.sale"),
    ("sensor alert",
     "SELECT * FROM label.alert"),
    ("SELECT カラム指定 + ORDER BY",
     "SELECT employee_name, age, salary FROM label.employee WHERE age > 25 ORDER BY salary DESC LIMIT 20"),
    ("SELECT * FROM label.* (全件)",
     "SELECT * FROM label.*"),
]

def run_benchmark(url, repeat=3):
    print(f"\n{chr(61)*60}")
    print(f"  SQL 性能テスト")
    print(f"{chr(61)*60}")
    print(f"  各クエリを {repeat} 回実行して平均/最小/最大を計測")
    print()

    results = []
    for label, query in BENCH_QUERIES:
        times, rows, ok = [], None, True
        for _ in range(repeat):
            t0   = time.perf_counter()
            resp = post_sql(url, query)
            ms   = (time.perf_counter() - t0) * 1000
            if resp and resp.get("ok"):
                times.append(ms)
                rows = resp.get("total_matched", resp.get("returned", "?"))
            else:
                ok = False; break

        if ok and times:
            avg, mn, mx = sum(times)/len(times), min(times), max(times)
            results.append((label, avg, mn, mx, rows))
            status = "[OK ]" if avg < 100 else ("[MID]" if avg < 500 else "[SLW]")
            print(f"  {status} {label}")
            print(f"       avg:{avg:7.1f}ms  min:{mn:7.1f}ms  max:{mx:7.1f}ms  rows:{rows}")
        else:
            err = resp.get("error","unknown") if resp else "no response"
            print(f"  [ERR] {label}")
            print(f"       ERROR: {err[:80]}")
        print()

    if results:
        print(f"{chr(61)*60}")
        print(f"  サマリー (avg基準 昇順)")
        print(f"{chr(61)*60}")
        for lbl, avg, mn, mx, rows in sorted(results, key=lambda x: x[1]):
            bar = "|" * min(int(avg / 20), 40)
            print(f"  {avg:7.1f}ms  {bar:<40}  {lbl[:40]}")
        fastest = min(results, key=lambda x: x[1])
        slowest = max(results, key=lambda x: x[1])
        print(f"\n  *** 最速: {fastest[0]} ({fastest[1]:.1f}ms)")
        print(f"  *** 最遅: {slowest[0]} ({slowest[1]:.1f}ms)")


# ===========================================================================
# エントリーポイント
# ===========================================================================

def parse_args():
    p = argparse.ArgumentParser(
        description="KAGURA DB テストデータ自動生成スクリプト",
    )
    p.add_argument("--url",       default=DEFAULT_URL)
    p.add_argument("--count",     type=int, default=DEFAULT_COUNT)
    p.add_argument("--workers",   type=int, default=DEFAULT_WORKERS)
    p.add_argument("--bench-only",action="store_true")
    p.add_argument("--bench",     action="store_true")
    p.add_argument("--repeat",    type=int, default=3)
    p.add_argument("--seed",      type=int, default=RANDOM_SEED)
    p.add_argument("--dump-json", metavar="PATH",
                    help="サーバーに接続せず、POST /records用のJSON配列をPATHへ書き出して終了")
    p.add_argument("--load-json", metavar="PATH",
                    help="ランダム生成せず、PATHのJSON配列を読み込んで投入する")
    return p.parse_args()


def main():
    args = parse_args()
    random.seed(args.seed)

    if args.dump_json:
        print(f"  {args.count:,} 件のテストデータを生成中...", end=" ", flush=True)
        records = build_record_list(args.count)
        dump_records_to_file(records, args.dump_json)
        print("完了")
        print(f"  -> {args.dump_json} ({len(records):,} 件, POST /records にそのまま投入可能な配列)")
        print(f"  投入例: python3 {sys.argv[0]} --load-json {args.dump_json} --url {DEFAULT_URL}")
        return

    if args.bench_only:
        run_benchmark(args.url, repeat=args.repeat)
        return

    loaded = load_records_from_file(args.load_json) if args.load_json else None
    stats = generate(url=args.url, total=args.count, workers=args.workers, records=loaded)

    if args.bench:
        run_benchmark(args.url, repeat=args.repeat)
    elif stats["error"] == 0:
        print("\n  性能テストも実行しますか? [y/N]: ", end="", flush=True)
        try:
            if input().strip().lower() in ("y", "yes"):
                run_benchmark(args.url, repeat=args.repeat)
        except (EOFError, KeyboardInterrupt):
            pass

    print(f"\n  Done!\n")


if __name__ == "__main__":
    main()
