"""
kdb_engine.py - Python から db_ffi.so を ctypes 経由で呼び出す .kdb セッションラッパー

暗号化 .kdb ストレージ（WAL 追記・SQL・NDJSON エクスポート）を Python から利用する。
平文 JSON・全書き直しの db_engine.DbEngine に対し、こちらは KAGURA DB を
唯一のデータストアにする本格利用向け。

使い方:
    from kdb_engine import KdbEngine

    db = KdbEngine.open("race.kdb")
    db.insert({"race_no": 1, "payout": 1200}, labels=["race", "y2025", "d20250906"])
    db.insert_many([
        {"columns": {"race_no": 2, "payout": 870}, "labels": ["race", "y2025"]},
        {"columns": {"race_no": 3, "payout": 1540}, "labels": ["race", "y2025"]},
    ])                                                   # -> 挿入件数
    db.sql("SELECT * FROM label.race WHERE race_date = '2025-09-06' ORDER BY race_no LIMIT 12")
    db.sql("DELETE FROM label.d20250906")                # 冪等な再取込
    db.get_by_label("y2024")                             # -> 型情報付き dict のリスト
    db.get_by_label_ndjson("y2024", "/tmp/y2024.ndjson") # -> 件数（polars.scan_ndjson で読む）
    db.count(); db.save(); db.close()

    # コンテキストマネージャ（close を保証）
    with KdbEngine.open("race.kdb") as db:
        ...

単一ライター制約:
    あるプロセスが KdbEngine.open 中は、同じ .kdb に対して別プロセス
    （db_client サーバーや別の KdbEngine）を起動しないこと。WAL の二重書き込みで
    ファイルが破損する。open は "<path>.lock" を排他作成してこれを補助する
    （別プロセスがロック中なら open は RuntimeError）。
"""
import ctypes
import json
import os
from pathlib import Path
from typing import Any, Optional


def _find_lib() -> str:
    if env := os.environ.get("DB_FFI_LIB"):
        return env
    here = Path(__file__).resolve().parent
    for p in [here / "../../target/release/libdb_ffi.so",
              here / "../../target/debug/libdb_ffi.so"]:
        if p.exists():
            return str(p.resolve())
    raise FileNotFoundError(
        "libdb_ffi.so not found. Run: cargo build --release -p db_ffi")


def _load_lib() -> ctypes.CDLL:
    lib = ctypes.CDLL(_find_lib())
    # --- 文字列解放（db_engine.py と共通の規約）---
    lib.db_string_free.restype = None
    lib.db_string_free.argtypes = [ctypes.c_void_p]
    # --- セッション管理 ---
    lib.kdb_open.restype = ctypes.c_void_p
    lib.kdb_open.argtypes = [ctypes.c_char_p]
    lib.kdb_close.restype = None
    lib.kdb_close.argtypes = [ctypes.c_void_p]
    lib.kdb_save.restype = ctypes.c_int32
    lib.kdb_save.argtypes = [ctypes.c_void_p]
    lib.kdb_count.restype = ctypes.c_uint64
    lib.kdb_count.argtypes = [ctypes.c_void_p]
    # --- 書き込み ---
    lib.kdb_insert.restype = ctypes.c_uint64
    lib.kdb_insert.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
    lib.kdb_insert_many.restype = ctypes.c_int64
    lib.kdb_insert_many.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
    # --- SQL ---
    lib.kdb_sql.restype = ctypes.c_void_p
    lib.kdb_sql.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
    # --- ラベル検索 ---
    lib.kdb_get_by_label.restype = ctypes.c_void_p
    lib.kdb_get_by_label.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
    lib.kdb_get_by_label_ndjson.restype = ctypes.c_int64
    lib.kdb_get_by_label_ndjson.argtypes = [
        ctypes.c_void_p, ctypes.c_char_p, ctypes.c_char_p]
    return lib


def _py_to_col(v: Any) -> dict:
    if isinstance(v, bool):
        return {"type": "boolean", "value": v}
    if isinstance(v, int):
        return {"type": "integer", "value": v}
    if isinstance(v, float):
        return {"type": "float", "value": v}
    if isinstance(v, str):
        return {"type": "text", "value": v}
    if v is None:
        return {"type": "null", "value": None}
    return {"type": "text", "value": str(v)}


def _col_to_py(col: dict) -> Any:
    return None if col.get("type") == "null" else col.get("value")


def _to_py(raw: dict) -> dict:
    return {
        "id": raw["id"],
        "columns": {k: _col_to_py(v) for k, v in raw.get("columns", {}).items()},
        "labels": raw.get("labels", []),
    }


class KdbError(RuntimeError):
    """kdb_sql 等が {"ok": false} を返したときに送出する"""


class KdbEngine:
    """
    暗号化 .kdb セッション（不透明ポインタのラッパー）。

    - insert / insert_many : WAL 追記による O(1) 書き込み
    - sql                  : SELECT / INSERT / UPDATE / DELETE（JOIN・GROUP BY 非対応）
    - get_by_label         : 型情報付きの取得
    - get_by_label_ndjson  : 巨大スライスをフラット NDJSON ファイルへ（polars 直読み）
    - save                 : チェックポイント（WAL 全書き直し）
    """

    def __init__(self, ptr: int, lib: ctypes.CDLL, path: str):
        self._lib = lib
        self._ptr = ptr
        self._path = path

    # -- 生成 ------------------------------------------------------------
    @classmethod
    def open(cls, path: str) -> "KdbEngine":
        """.kdb を開く（無ければ新規作成）。別プロセスがロック中なら RuntimeError。"""
        lib = _load_lib()
        ptr = lib.kdb_open(str(path).encode())
        if not ptr:
            raise RuntimeError(
                f"kdb_open('{path}') failed "
                "（別プロセスが同じ .kdb を開いている / パスが不正 / I/O エラー）")
        return cls(ptr, lib, str(path))

    # -- コンテキストマネージャ ----------------------------------------
    def __enter__(self) -> "KdbEngine":
        return self

    def __exit__(self, *_exc) -> None:
        self.close()

    def __del__(self):
        # close 忘れの保険。明示的な close() を推奨。
        if getattr(self, "_ptr", None):
            try:
                self._lib.kdb_close(self._ptr)
            finally:
                self._ptr = None

    def __repr__(self) -> str:
        return f"KdbEngine(path={self._path!r}, count={self.count()})"

    # -- 内部ヘルパー --------------------------------------------------
    def _read_free(self, ptr: Optional[int]) -> Optional[str]:
        if not ptr:
            return None
        s = ctypes.string_at(ptr).decode()
        self._lib.db_string_free(ptr)
        return s

    def _check_open(self) -> None:
        if not self._ptr:
            raise RuntimeError("KdbEngine is closed")

    # -- 書き込み ----------------------------------------------------
    def insert(self, columns: dict, labels: Optional[list] = None) -> int:
        """レコードを 1 件挿入（WAL 追記）。戻り値: 発行されたレコードID。"""
        self._check_open()
        payload = {
            "id": 0,
            "columns": {k: _py_to_col(v) for k, v in columns.items()},
            "labels": labels or [],
        }
        rid = self._lib.kdb_insert(self._ptr, json.dumps(payload).encode())
        if rid == 0:
            raise KdbError("kdb_insert() failed（不正な値 or 重複ラベル）")
        return int(rid)

    def insert_many(self, records: list) -> int:
        """
        複数レコードを一括挿入（バックフィル用。FFI 呼び出しを 1 回に集約）。
        records: [{"columns": {...}, "labels": [...]}, ...]
                 （"columns" は Python の生の値。内部で型タグへ変換する）
        戻り値: 挿入に成功した件数。個々の失敗はスキップされる。
        """
        self._check_open()
        payload = [
            {
                "id": 0,
                "columns": {k: _py_to_col(v) for k, v in r.get("columns", {}).items()},
                "labels": r.get("labels", []),
            }
            for r in records
        ]
        n = self._lib.kdb_insert_many(self._ptr, json.dumps(payload).encode())
        if n < 0:
            raise KdbError("kdb_insert_many() failed（配列としてパースできない）")
        return int(n)

    # -- SQL -------------------------------------------------------
    def sql(self, query: str) -> dict:
        """
        SELECT / INSERT / UPDATE / DELETE を実行する。
        戻り値（成功時）:
            SELECT : {"ok": True, "columns": [...], "rows": [[...]],
                      "total_matched": N, "returned": M, "records": [dict, ...]}
            INSERT : {"ok": True, "inserted_id": N, "columns": [...], "labels": [...]}
            UPDATE : {"ok": True, "updated_count": N}
            DELETE : {"ok": True, "deleted_count": N}
        失敗時は KdbError を送出。
        """
        self._check_open()
        raw = self._read_free(self._lib.kdb_sql(self._ptr, query.encode()))
        res = json.loads(raw) if raw else {"ok": False, "error": "no response"}
        if not res.get("ok"):
            raise KdbError(res.get("error") or "kdb_sql() failed")
        # SELECT の結果は columns/rows に加え、行 dict のリストも添える
        if "columns" in res and "rows" in res:
            cols = res["columns"]
            res["records"] = [dict(zip(cols, row)) for row in res["rows"]]
        return res

    def select(self, query: str) -> list:
        """SELECT 専用のショートカット。行を dict のリストで返す。"""
        return self.sql(query)["records"]

    # -- ラベル検索 -----------------------------------------------
    def get_by_label(self, label: str) -> list:
        """
        ラベルでレコードを取得する（型情報を保持）。
        戻り値: [{"id": .., "columns": {..}, "labels": [..]}, ...]
        """
        self._check_open()
        raw = self._read_free(
            self._lib.kdb_get_by_label(self._ptr, label.encode()))
        return [_to_py(r) for r in json.loads(raw)] if raw else []

    def get_by_label_ndjson(self, label: str, out_path: str) -> int:
        """
        ラベルのレコードをフラット NDJSON でファイルに書き出す。
        1 行 = {"id": .., <各カラム>, "labels": [..]}（SELECT * 相当）。
        巨大スライス（年 40 万件クラス）向け。Python 側は
            import polars as pl
            lf = pl.scan_ndjson(out_path)          # 遅延読み込み
        で読む。数値カラムに NULL が混じる場合は
            pl.scan_ndjson(out_path, infer_schema_length=None)
        あるいは明示スキーマを渡す。
        戻り値: 書き出した件数。ファイル I/O 失敗時は -1。
        """
        self._check_open()
        n = self._lib.kdb_get_by_label_ndjson(
            self._ptr, label.encode(), str(out_path).encode())
        if n < 0:
            raise KdbError(f"kdb_get_by_label_ndjson() failed: {out_path}")
        return int(n)

    # -- セッション ---------------------------------------------------
    def count(self) -> int:
        """有効レコード数"""
        self._check_open()
        return int(self._lib.kdb_count(self._ptr))

    def save(self) -> None:
        """チェックポイント（WAL 全書き直し）。UPDATE/DELETE 後や終了前に呼ぶ。"""
        self._check_open()
        if not self._lib.kdb_save(self._ptr):
            raise KdbError("kdb_save() failed")

    def close(self) -> None:
        """セッションを閉じ、ロックファイルを解放する。二重呼び出しは無害。"""
        if getattr(self, "_ptr", None):
            self._lib.kdb_close(self._ptr)
            self._ptr = None
