"""
db_engine.py - Python から db_ffi.so を ctypes 経由で呼び出すラッパー
使い方:
    from db_engine import DbEngine
    db = DbEngine()
    id = db.insert({"name": "Alice", "age": 30}, labels=["dept:eng"])
"""
import ctypes, json, os
from pathlib import Path
from typing import Any, Optional

def _find_lib():
    if env := os.environ.get("DB_FFI_LIB"):
        return env
    here = Path(__file__).resolve().parent
    for p in [here/"../../target/release/libdb_ffi.so", here/"../../target/debug/libdb_ffi.so"]:
        if p.exists(): return str(p.resolve())
    raise FileNotFoundError("libdb_ffi.so not found. Run: cargo build --release -p db_ffi")

def _load_lib():
    lib = ctypes.CDLL(_find_lib())
    lib.db_new.restype          = ctypes.c_void_p
    lib.db_new.argtypes         = []
    lib.db_free.restype         = None
    lib.db_free.argtypes        = [ctypes.c_void_p]
    lib.db_string_free.restype  = None
    lib.db_string_free.argtypes = [ctypes.c_void_p]
    lib.db_insert.restype       = ctypes.c_uint64
    lib.db_insert.argtypes      = [ctypes.c_void_p, ctypes.c_char_p]
    lib.db_get.restype          = ctypes.c_void_p
    lib.db_get.argtypes         = [ctypes.c_void_p, ctypes.c_uint64]
    lib.db_delete.restype       = ctypes.c_int32
    lib.db_delete.argtypes      = [ctypes.c_void_p, ctypes.c_uint64]
    lib.db_update.restype       = ctypes.c_int32
    lib.db_update.argtypes      = [ctypes.c_void_p, ctypes.c_uint64, ctypes.c_char_p]
    lib.db_get_by_label.restype  = ctypes.c_void_p
    lib.db_get_by_label.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
    lib.db_list_all.restype     = ctypes.c_void_p
    lib.db_list_all.argtypes    = [ctypes.c_void_p]
    lib.db_count.restype        = ctypes.c_uint64
    lib.db_count.argtypes       = [ctypes.c_void_p]
    lib.db_save.restype         = ctypes.c_int32
    lib.db_save.argtypes        = [ctypes.c_void_p, ctypes.c_char_p]
    lib.db_load.restype         = ctypes.c_void_p
    lib.db_load.argtypes        = [ctypes.c_char_p]
    lib.db_list_labels.restype  = ctypes.c_void_p
    lib.db_list_labels.argtypes = [ctypes.c_void_p]
    return lib

def _py_to_col(v):
    if isinstance(v, bool):  return {"type": "boolean", "value": v}
    if isinstance(v, int):   return {"type": "integer", "value": v}
    if isinstance(v, float): return {"type": "float",   "value": v}
    if isinstance(v, str):   return {"type": "text",    "value": v}
    if v is None:            return {"type": "null",    "value": None}
    return {"type": "text", "value": str(v)}

def _col_to_py(col):
    return None if col.get("type") == "null" else col.get("value")

def _to_py(raw):
    return {"id": raw["id"],
            "columns": {k: _col_to_py(v) for k, v in raw.get("columns",{}).items()},
            "labels": raw.get("labels", [])}

class DbEngine:
    """
    Python 向け DB エンジンラッパー。
    例:
        db = DbEngine()
        id = db.insert({"name": "Alice", "age": 30}, labels=["dept:eng"])
        db.get(id)       # -> {"id":1, "columns":{"name":"Alice","age":30}, "labels":[...]}
        db.save("/tmp/mydb.json")
        db2 = DbEngine.load("/tmp/mydb.json")
    """
    def __init__(self):
        self._lib = _load_lib()
        self._ptr = self._lib.db_new()
        if not self._ptr: raise RuntimeError("db_new() failed")

    def __del__(self):
        if getattr(self, "_ptr", None):
            self._lib.db_free(self._ptr); self._ptr = None

    def __repr__(self): return f"DbEngine(count={self.count()})"

    def _read_free(self, ptr):
        if not ptr: return None
        s = ctypes.string_at(ptr).decode()
        self._lib.db_string_free(ptr)
        return s

    def insert(self, columns: dict, labels: list = None) -> int:
        """レコードを挿入する。戻り値: 発行されたレコードID"""
        payload = {"id": 0,
                   "columns": {k: _py_to_col(v) for k,v in columns.items()},
                   "labels": labels or []}
        rid = self._lib.db_insert(self._ptr, json.dumps(payload).encode())
        if rid == 0: raise RuntimeError("db_insert() failed")
        return int(rid)

    def get(self, record_id: int) -> Optional[dict]:
        """IDでレコードを取得。存在しなければ None"""
        raw = self._read_free(self._lib.db_get(self._ptr, record_id))
        return _to_py(json.loads(raw)) if raw else None

    def delete(self, record_id: int) -> bool:
        """IDでレコードを削除。成功すれば True"""
        return bool(self._lib.db_delete(self._ptr, record_id))

    def update(self, record_id: int, columns: dict, labels: list = None) -> bool:
        """IDでレコードを更新。成功すれば True"""
        payload = {"id": 0,
                   "columns": {k: _py_to_col(v) for k,v in columns.items()},
                   "labels": labels or []}
        return bool(self._lib.db_update(self._ptr, record_id, json.dumps(payload).encode()))

    def get_by_label(self, label: str) -> list:
        """ラベルでレコードを検索"""
        raw = self._read_free(self._lib.db_get_by_label(self._ptr, label.encode()))
        return [_to_py(r) for r in json.loads(raw)] if raw else []

    def list_all(self) -> list:
        """全レコードを返す"""
        raw = self._read_free(self._lib.db_list_all(self._ptr))
        return [_to_py(r) for r in json.loads(raw)] if raw else []

    def count(self) -> int:
        """有効レコード数"""
        return int(self._lib.db_count(self._ptr))

    def list_labels(self) -> list:
        """全ラベル一覧（ソート済み）"""
        raw = self._read_free(self._lib.db_list_labels(self._ptr))
        return json.loads(raw) if raw else []

    def save(self, path: str) -> None:
        """ファイルに保存"""
        if not self._lib.db_save(self._ptr, path.encode()):
            raise IOError(f"db_save() failed: {path}")

    @classmethod
    def load(cls, path: str) -> "DbEngine":
        """ファイルから読み込む"""
        inst = object.__new__(cls)
        inst._lib = _load_lib()
        inst._ptr = inst._lib.db_load(path.encode())
        if not inst._ptr: raise IOError(f"db_load() failed: {path}")
        return inst
