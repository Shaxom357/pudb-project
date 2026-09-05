// src/bootstrap.rs
// ディスクから `Database` を組み立て直す共通処理。
// トランザクションの `ROLLBACK` は「`.kdb`/JSON はトランザクション中に一切書き換えていない」
// という前提のもと、単にディスクから読み直すことで実現する（サイドカーの索引・スキーマ定義・
// 「全データオンメモリ」設定もあわせて復元する）。

use db_engine::Database;

/// `db_path` の拡張子で `.kdb`（暗号化バイナリ）か JSON かを判定して読み込み、
/// サイドカーファイルの二次インデックス定義・任意スキーマ定義・「全データオンメモリ」設定を
/// 再適用した `Database` を返す。
pub fn reload_database(db_path: &str) -> Result<Database, String> {
    let is_kdb = db_path.ends_with(".kdb");

    let mut db = if is_kdb {
        Database::load_or_new_kdb(db_path).map_err(|e| format!("KDB load failed: {}", e))?
    } else {
        Database::load_or_new(db_path).map_err(|e| format!("DB load failed: {}", e))?
    };

    crate::index_sql::rebuild_indexes_from_sidecar(db_path, &mut db);
    crate::schema_sql::rebuild_schemas_from_sidecar(db_path, &mut db);

    if is_kdb {
        if let Some(policy) = crate::memory_settings::load(db_path) {
            db.set_memory_policy(policy);
        }
    }

    Ok(db)
}
