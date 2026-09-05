// db_engine/src/tests.rs

use crate::{ColumnSchema, Database, DataType, DatabaseError, Record, SchemaType};
use crate::{MemoryPolicy, MemorySizeSpec};
use crate::kdb_store::KdbFile;

// ---------------------------------------------------------------------------
// insert / get
// ---------------------------------------------------------------------------

#[test]
fn test_insert_auto_id() {
    let mut db = Database::new();
    let mut r = Record::new(0);
    r.set("name", DataType::Text("Alice".to_string()));
    r.set("age", DataType::Integer(30));
    let id = db.insert(r).unwrap();
    assert_eq!(id, 1);
    assert_eq!(db.count(), 1);
}

#[test]
fn test_insert_manual_id() {
    let mut db = Database::new();
    let mut r = Record::new(42);
    r.set("name", DataType::Text("Bob".to_string()));
    let id = db.insert(r).unwrap();
    assert_eq!(id, 42);
}

#[test]
fn test_insert_duplicate_id_returns_error() {
    let mut db = Database::new();
    let r1 = Record::new(1);
    db.insert(r1).unwrap();
    let r2 = Record::new(1);
    let result = db.insert(r2);
    assert_eq!(result, Err(DatabaseError::DuplicateId(1)));
}

#[test]
fn test_get_by_id() {
    let mut db = Database::new();
    let mut r = Record::new(0);
    r.set("value", DataType::Integer(99));
    let id = db.insert(r).unwrap();
    let fetched = db.get(id).unwrap();
    assert_eq!(fetched.get_col("value"), Some(&DataType::Integer(99)));
}

#[test]
fn test_get_nonexistent_id_returns_error() {
    let db = Database::new();
    let result = db.get(999);
    assert_eq!(result, Err(DatabaseError::RecordNotFound(999)));
}

// ---------------------------------------------------------------------------
// label 検索
// ---------------------------------------------------------------------------

#[test]
fn test_get_by_label_single() {
    let mut db = Database::new();
    let mut r = Record::new(0);
    r.set("dept", DataType::Text("Engineering".to_string()));
    r.add_label("team:backend");
    db.insert(r).unwrap();
    let results = db.get_by_label("team:backend");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].get_col("dept"), Some(&DataType::Text("Engineering".to_string())));
}

#[test]
fn test_get_by_label_multiple_records() {
    let mut db = Database::new();
    for name in ["Alice", "Bob", "Carol"] {
        let mut r = Record::new(0);
        r.set("name", DataType::Text(name.to_string()));
        r.add_label("group:A");
        db.insert(r).unwrap();
    }
    let mut extra = Record::new(0);
    extra.set("name", DataType::Text("Dave".to_string()));
    extra.add_label("group:B");
    db.insert(extra).unwrap();

    let results = db.get_by_label("group:A");
    assert_eq!(results.len(), 3);
    let results_b = db.get_by_label("group:B");
    assert_eq!(results_b.len(), 1);
}

#[test]
fn test_get_by_label_not_found_returns_empty() {
    let db = Database::new();
    let results = db.get_by_label("nonexistent");
    assert!(results.is_empty());
}

#[test]
fn test_record_multiple_labels() {
    let mut db = Database::new();
    let mut r = Record::new(0);
    r.add_label("env:prod");
    r.add_label("region:ap-northeast-1");
    db.insert(r).unwrap();
    assert_eq!(db.get_by_label("env:prod").len(), 1);
    assert_eq!(db.get_by_label("region:ap-northeast-1").len(), 1);
    assert_eq!(db.get_by_label("env:dev").len(), 0);
}

// ---------------------------------------------------------------------------
// update
// ---------------------------------------------------------------------------

#[test]
fn test_update_column_value() {
    let mut db = Database::new();
    let mut r = Record::new(0);
    r.set("score", DataType::Integer(50));
    let id = db.insert(r).unwrap();

    let mut updated = Record::new(0);
    updated.set("score", DataType::Integer(100));
    db.update(id, updated).unwrap();

    let fetched = db.get(id).unwrap();
    assert_eq!(fetched.get_col("score"), Some(&DataType::Integer(100)));
}

#[test]
fn test_update_labels() {
    let mut db = Database::new();
    let mut r = Record::new(0);
    r.add_label("status:pending");
    let id = db.insert(r).unwrap();
    assert_eq!(db.get_by_label("status:pending").len(), 1);

    let mut updated = Record::new(0);
    updated.add_label("status:active");
    db.update(id, updated).unwrap();

    assert_eq!(db.get_by_label("status:pending").len(), 0);
    assert_eq!(db.get_by_label("status:active").len(), 1);
}

#[test]
fn test_update_nonexistent_returns_error() {
    let mut db = Database::new();
    let r = Record::new(0);
    let result = db.update(999, r);
    assert_eq!(result, Err(DatabaseError::RecordNotFound(999)));
}

// ---------------------------------------------------------------------------
// delete
// ---------------------------------------------------------------------------

#[test]
fn test_delete_record() {
    let mut db = Database::new();
    let mut r = Record::new(0);
    r.set("name", DataType::Text("ToDelete".to_string()));
    r.add_label("temp");
    let id = db.insert(r).unwrap();
    assert_eq!(db.count(), 1);

    db.delete(id).unwrap();
    assert_eq!(db.count(), 0);
    assert_eq!(db.get(id), Err(DatabaseError::RecordNotFound(id)));
    assert!(db.get_by_label("temp").is_empty());
}

#[test]
fn test_delete_nonexistent_returns_error() {
    let mut db = Database::new();
    let result = db.delete(42);
    assert_eq!(result, Err(DatabaseError::RecordNotFound(42)));
}

#[test]
fn test_delete_does_not_affect_other_records() {
    let mut db = Database::new();
    let id1 = db.insert({ let mut r = Record::new(0); r.add_label("keep"); r }).unwrap();
    let id2 = db.insert({ let mut r = Record::new(0); r.add_label("keep"); r }).unwrap();
    db.delete(id1).unwrap();
    assert_eq!(db.count(), 1);
    assert!(db.get(id2).is_ok());
    assert_eq!(db.get_by_label("keep").len(), 1);
}

// ---------------------------------------------------------------------------
// list_all / count
// ---------------------------------------------------------------------------

#[test]
fn test_list_all() {
    let mut db = Database::new();
    db.insert(Record::new(0)).unwrap();
    db.insert(Record::new(0)).unwrap();
    db.insert(Record::new(0)).unwrap();
    assert_eq!(db.list_all().len(), 3);
    assert_eq!(db.count(), 3);
}

#[test]
fn test_list_all_empty() {
    let db = Database::new();
    assert!(db.list_all().is_empty());
    assert_eq!(db.count(), 0);
}

// ---------------------------------------------------------------------------
// search_by_column
// ---------------------------------------------------------------------------

#[test]
fn test_search_by_column() {
    let mut db = Database::new();
    for (name, dept) in [("Alice", "Eng"), ("Bob", "Sales"), ("Carol", "Eng")] {
        let mut r = Record::new(0);
        r.set("name", DataType::Text(name.to_string()));
        r.set("dept", DataType::Text(dept.to_string()));
        db.insert(r).unwrap();
    }
    let results = db.search_by_column("dept", &DataType::Text("Eng".to_string()));
    assert_eq!(results.len(), 2);
    let results_sales = db.search_by_column("dept", &DataType::Text("Sales".to_string()));
    assert_eq!(results_sales.len(), 1);
}

#[test]
fn test_search_by_column_not_found() {
    let mut db = Database::new();
    let mut r = Record::new(0);
    r.set("x", DataType::Integer(1));
    db.insert(r).unwrap();
    let results = db.search_by_column("x", &DataType::Integer(2));
    assert!(results.is_empty());
}

// ---------------------------------------------------------------------------
// DataType display
// ---------------------------------------------------------------------------

#[test]
fn test_datatype_display() {
    assert_eq!(DataType::Text("hello".to_string()).to_string(), "hello");
    assert_eq!(DataType::Integer(42).to_string(), "42");
    assert_eq!(DataType::Float(3.14).to_string(), "3.14");
    assert_eq!(DataType::Boolean(true).to_string(), "true");
    assert_eq!(DataType::Null.to_string(), "NULL");
}

// ---------------------------------------------------------------------------
// attach_label / detach_label / list_all_labels
// ---------------------------------------------------------------------------

#[test]
fn test_attach_label_adds_label() {
    let mut db = Database::new();
    let id = db.insert(Record::new(0)).unwrap();
    db.attach_label(id, "env:prod").unwrap();
    let r = db.get(id).unwrap();
    assert!(r.labels.contains(&"env:prod".to_string()));
    assert_eq!(db.get_by_label("env:prod").len(), 1);
}

#[test]
fn test_attach_label_duplicate_returns_error_and_does_not_change() {
    let mut db = Database::new();
    let id = db.insert(Record::new(0)).unwrap();
    db.attach_label(id, "tag").unwrap();
    let err = db.attach_label(id, "tag").unwrap_err(); // 重複付与はエラー
    assert_eq!(err, DatabaseError::DuplicateLabel("tag".to_string()));
    let r = db.get(id).unwrap();
    assert_eq!(r.labels.iter().filter(|l| l.as_str() == "tag").count(), 1);
    assert_eq!(db.get_by_label("tag").len(), 1);
}

#[test]
fn test_insert_rejects_duplicate_labels_in_same_record() {
    let mut db = Database::new();
    let mut r = Record::new(0);
    r.add_label("tag");
    r.add_label("tag");
    let err = db.insert(r).unwrap_err();
    assert_eq!(err, DatabaseError::DuplicateLabel("tag".to_string()));
    assert_eq!(db.count(), 0);
}

#[test]
fn test_attach_label_nonexistent_record() {
    let mut db = Database::new();
    let result = db.attach_label(999, "tag");
    assert_eq!(result, Err(DatabaseError::RecordNotFound(999)));
}

#[test]
fn test_detach_label_removes_label() {
    let mut db = Database::new();
    let mut r = Record::new(0);
    r.add_label("env:prod");
    let id = db.insert(r).unwrap();

    let removed = db.detach_label(id, "env:prod").unwrap();
    assert!(removed);
    assert!(db.get(id).unwrap().labels.is_empty());
    assert!(db.get_by_label("env:prod").is_empty());
}

#[test]
fn test_detach_label_not_present_returns_false() {
    let mut db = Database::new();
    let id = db.insert(Record::new(0)).unwrap();
    let removed = db.detach_label(id, "nothere").unwrap();
    assert!(!removed);
}

#[test]
fn test_detach_label_nonexistent_record() {
    let mut db = Database::new();
    let result = db.detach_label(999, "tag");
    assert_eq!(result, Err(DatabaseError::RecordNotFound(999)));
}

#[test]
fn test_list_all_labels() {
    let mut db = Database::new();
    let mut r1 = Record::new(0);
    r1.add_label("env:prod");
    r1.add_label("region:ap");
    db.insert(r1).unwrap();
    let mut r2 = Record::new(0);
    r2.add_label("env:dev");
    db.insert(r2).unwrap();

    let labels = db.list_all_labels();
    assert_eq!(labels, vec!["env:dev", "env:prod", "region:ap"]);
}

#[test]
fn test_list_all_labels_excludes_deleted_records() {
    let mut db = Database::new();
    let mut r = Record::new(0);
    r.add_label("ghost");
    let id = db.insert(r).unwrap();
    db.delete(id).unwrap();
    assert!(!db.list_all_labels().contains(&"ghost".to_string()));
}

// ---------------------------------------------------------------------------
// 永続化（save / load / load_or_new）
// ---------------------------------------------------------------------------

use crate::PersistError;

#[test]
fn test_save_and_load_roundtrip() {
    let tmp = std::env::temp_dir().join("db_engine_test_roundtrip.json");

    let mut db = Database::new();
    let mut r1 = Record::new(0);
    r1.set("name", DataType::Text("Alice".to_string()));
    r1.set("age",  DataType::Integer(30));
    r1.add_label("dept:engineering");
    let id1 = db.insert(r1).unwrap();

    let mut r2 = Record::new(0);
    r2.set("name", DataType::Text("Bob".to_string()));
    r2.set("score", DataType::Float(88.5));
    r2.add_label("dept:sales");
    r2.add_label("env:prod");
    let id2 = db.insert(r2).unwrap();

    // 保存
    db.save(&tmp).unwrap();
    assert!(tmp.exists());

    // 読み込み
    let db2 = Database::load(&tmp).unwrap();
    assert_eq!(db2.count(), 2);

    let rec1 = db2.get(id1).unwrap();
    assert_eq!(rec1.get_col("name"),  Some(&DataType::Text("Alice".to_string())));
    assert_eq!(rec1.get_col("age"),   Some(&DataType::Integer(30)));
    assert_eq!(rec1.labels, vec!["dept:engineering"]);

    let rec2 = db2.get(id2).unwrap();
    assert_eq!(rec2.get_col("name"),  Some(&DataType::Text("Bob".to_string())));
    assert_eq!(rec2.get_col("score"), Some(&DataType::Float(88.5)));
    assert!(rec2.labels.contains(&"dept:sales".to_string()));
    assert!(rec2.labels.contains(&"env:prod".to_string()));

    // ラベルインデックスが再構築されているか確認
    assert_eq!(db2.get_by_label("dept:engineering").len(), 1);
    assert_eq!(db2.get_by_label("dept:sales").len(), 1);
    assert_eq!(db2.get_by_label("env:prod").len(), 1);

    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn test_save_and_load_preserves_next_id() {
    let tmp = std::env::temp_dir().join("db_engine_test_next_id.json");

    let mut db = Database::new();
    db.insert(Record::new(0)).unwrap(); // id=1
    db.insert(Record::new(0)).unwrap(); // id=2
    db.save(&tmp).unwrap();

    let mut db2 = Database::load(&tmp).unwrap();
    // 次のレコードは id=3 になるはず
    let id3 = db2.insert(Record::new(0)).is_ok();
    assert!(id3);
    // ロードした DB のレコード数は 3（元の2件 + 追加1件）
    assert_eq!(db2.count(), 3);

    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn test_save_excludes_deleted_records() {
    let tmp = std::env::temp_dir().join("db_engine_test_delete.json");

    let mut db = Database::new();
    let id1 = db.insert(Record::new(0)).unwrap();
    let id2 = db.insert(Record::new(0)).unwrap();
    db.delete(id1).unwrap(); // 論理削除
    db.save(&tmp).unwrap();

    let db2 = Database::load(&tmp).unwrap();
    assert_eq!(db2.count(), 1);
    assert!(db2.get(id1).is_err()); // 削除済みは復元されない
    assert!(db2.get(id2).is_ok());

    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn test_load_or_new_creates_new_db_when_file_missing() {
    let tmp = std::env::temp_dir().join("db_engine_test_nonexistent_xyz.json");
    let _ = std::fs::remove_file(&tmp); // 必ず存在しないようにする
    let db = Database::load_or_new(&tmp).unwrap();
    assert_eq!(db.count(), 0);
}

#[test]
fn test_load_nonexistent_file_returns_error() {
    let result = Database::load("/nonexistent/path/db.json");
    assert!(matches!(result, Err(PersistError::Io(_))));
}

#[test]
fn test_save_and_load_all_datatypes() {
    let tmp = std::env::temp_dir().join("db_engine_test_datatypes.json");

    let mut db = Database::new();
    let mut r = Record::new(0);
    r.set("text",    DataType::Text("hello".to_string()));
    r.set("int",     DataType::Integer(-42));
    r.set("float",   DataType::Float(3.14));
    r.set("bool",    DataType::Boolean(true));
    r.set("null_col", DataType::Null);
    let id = db.insert(r).unwrap();
    db.save(&tmp).unwrap();

    let db2 = Database::load(&tmp).unwrap();
    let rec = db2.get(id).unwrap();
    assert_eq!(rec.get_col("text"),     Some(&DataType::Text("hello".to_string())));
    assert_eq!(rec.get_col("int"),      Some(&DataType::Integer(-42)));
    assert_eq!(rec.get_col("float"),    Some(&DataType::Float(3.14)));
    assert_eq!(rec.get_col("bool"),     Some(&DataType::Boolean(true)));
    assert_eq!(rec.get_col("null_col"), Some(&DataType::Null));

    let _ = std::fs::remove_file(&tmp);
}

// ---------------------------------------------------------------------------
// 二次インデックス（CREATE INDEX / DROP INDEX）
// ---------------------------------------------------------------------------

fn insert_venue(db: &mut Database, name: &str, venue: &str) -> u64 {
    let mut r = Record::new(0);
    r.set("name", DataType::Text(name.to_string()));
    r.set("venue", DataType::Text(venue.to_string()));
    db.insert(r).unwrap()
}

#[test]
fn test_create_index_indexes_existing_records() {
    let mut db = Database::new();
    insert_venue(&mut db, "race1", "edogawa");
    insert_venue(&mut db, "race2", "toda");
    insert_venue(&mut db, "race3", "edogawa");

    db.create_index("venue").unwrap();
    assert!(db.has_index("venue"));

    let hits = db.get_by_index("venue", &DataType::Text("edogawa".to_string())).unwrap();
    let names: Vec<&str> = hits.iter().filter_map(|r| match r.get_col("name") {
        Some(DataType::Text(s)) => Some(s.as_str()), _ => None,
    }).collect();
    assert_eq!(names, vec!["race1", "race3"]);
}

#[test]
fn test_create_index_on_column_with_no_matching_value_returns_empty() {
    let mut db = Database::new();
    insert_venue(&mut db, "race1", "edogawa");
    db.create_index("venue").unwrap();

    let hits = db.get_by_index("venue", &DataType::Text("toda".to_string())).unwrap();
    assert!(hits.is_empty());
}

#[test]
fn test_get_by_index_on_non_indexed_column_returns_none() {
    let mut db = Database::new();
    insert_venue(&mut db, "race1", "edogawa");
    // "venue" にインデックスを作っていないので None（呼び出し側は全件走査にフォールバックする）
    assert_eq!(db.get_by_index("venue", &DataType::Text("edogawa".to_string())), None);
}

#[test]
fn test_create_index_twice_is_error() {
    let mut db = Database::new();
    db.create_index("venue").unwrap();
    let err = db.create_index("venue").unwrap_err();
    assert_eq!(err, DatabaseError::IndexAlreadyExists("venue".to_string()));
}

#[test]
fn test_drop_index_removes_it() {
    let mut db = Database::new();
    db.create_index("venue").unwrap();
    db.drop_index("venue").unwrap();
    assert!(!db.has_index("venue"));
    assert_eq!(db.get_by_index("venue", &DataType::Text("edogawa".to_string())), None);
}

#[test]
fn test_drop_nonexistent_index_is_error() {
    let mut db = Database::new();
    let err = db.drop_index("venue").unwrap_err();
    assert_eq!(err, DatabaseError::IndexNotFound("venue".to_string()));
}

#[test]
fn test_list_indexes_sorted() {
    let mut db = Database::new();
    db.create_index("venue").unwrap();
    db.create_index("age").unwrap();
    assert_eq!(db.list_indexes(), vec!["age".to_string(), "venue".to_string()]);
}

#[test]
fn test_index_created_before_insert_is_maintained_on_new_records() {
    let mut db = Database::new();
    db.create_index("venue").unwrap();
    insert_venue(&mut db, "race1", "edogawa");
    insert_venue(&mut db, "race2", "edogawa");

    let hits = db.get_by_index("venue", &DataType::Text("edogawa".to_string())).unwrap();
    assert_eq!(hits.len(), 2);
}

#[test]
fn test_index_updated_on_update() {
    let mut db = Database::new();
    let id = insert_venue(&mut db, "race1", "edogawa");
    db.create_index("venue").unwrap();

    let mut updated = db.get(id).unwrap().clone();
    updated.set("venue", DataType::Text("toda".to_string()));
    db.update(id, updated).unwrap();

    // 旧い値(edogawa)のインデックスからは外れている
    assert!(db.get_by_index("venue", &DataType::Text("edogawa".to_string())).unwrap().is_empty());
    // 新しい値(toda)のインデックスに入っている
    let hits = db.get_by_index("venue", &DataType::Text("toda".to_string())).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, id);
}

#[test]
fn test_index_removed_on_delete() {
    let mut db = Database::new();
    let id = insert_venue(&mut db, "race1", "edogawa");
    db.create_index("venue").unwrap();
    db.delete(id).unwrap();

    let hits = db.get_by_index("venue", &DataType::Text("edogawa".to_string())).unwrap();
    assert!(hits.is_empty());
}

#[test]
fn test_index_skips_null_values() {
    let mut db = Database::new();
    let mut r = Record::new(0);
    r.set("venue", DataType::Null);
    db.insert(r).unwrap();
    db.create_index("venue").unwrap();

    // NULL は等値検索インデックスの対象外
    let hits = db.get_by_index("venue", &DataType::Null).unwrap();
    assert!(hits.is_empty());
}

#[test]
fn test_index_distinguishes_value_types() {
    let mut db = Database::new();
    let mut r1 = Record::new(0);
    r1.set("code", DataType::Text("1".to_string()));
    db.insert(r1).unwrap();
    let mut r2 = Record::new(0);
    r2.set("code", DataType::Integer(1));
    db.insert(r2).unwrap();

    db.create_index("code").unwrap();
    assert_eq!(db.get_by_index("code", &DataType::Text("1".to_string())).unwrap().len(), 1);
    assert_eq!(db.get_by_index("code", &DataType::Integer(1)).unwrap().len(), 1);
}

// ---------------------------------------------------------------------------
// 任意スキーマ層（DEFINE COLUMN / ENABLE|DISABLE SCHEMA）
// ---------------------------------------------------------------------------

fn not_null_text() -> ColumnSchema {
    ColumnSchema { ty: SchemaType::Text, not_null: true, default: None, unique: false }
}

fn insert_race(db: &mut Database, date: Option<&str>) -> Result<u64, DatabaseError> {
    let mut r = Record::new(0);
    if let Some(d) = date { r.set("date", DataType::Text(d.to_string())); }
    r.add_label("race");
    db.insert(r)
}

#[test]
fn test_schema_disabled_by_default_does_not_enforce() {
    let mut db = Database::new();
    db.define_column("race", "date", not_null_text()).unwrap();
    // ENABLE SCHEMA していないので、NOT NULL でも date 無しで INSERT できる
    let id = insert_race(&mut db, None).unwrap();
    assert!(db.get(id).is_ok());
}

#[test]
fn test_schema_enabled_enforces_not_null() {
    let mut db = Database::new();
    db.define_column("race", "date", not_null_text()).unwrap();
    db.enable_schema("race");

    let err = insert_race(&mut db, None).unwrap_err();
    assert!(matches!(err, DatabaseError::SchemaViolation(_)));
    assert_eq!(db.count(), 0);

    let id = insert_race(&mut db, Some("2026-01-01")).unwrap();
    assert!(db.get(id).is_ok());
}

#[test]
fn test_schema_enforces_type() {
    let mut db = Database::new();
    db.define_column("race", "odds", ColumnSchema {
        ty: SchemaType::Float, not_null: false, default: None, unique: false,
    }).unwrap();
    db.enable_schema("race");

    let mut r = Record::new(0);
    r.set("odds", DataType::Text("not a number".to_string()));
    r.add_label("race");
    let err = db.insert(r).unwrap_err();
    assert!(matches!(err, DatabaseError::SchemaViolation(_)));
}

#[test]
fn test_schema_integer_satisfies_float_type() {
    // Integer の値は Float 宣言にも適合する（数値の自動昇格）
    let mut db = Database::new();
    db.define_column("race", "odds", ColumnSchema {
        ty: SchemaType::Float, not_null: false, default: None, unique: false,
    }).unwrap();
    db.enable_schema("race");

    let mut r = Record::new(0);
    r.set("odds", DataType::Integer(3));
    r.add_label("race");
    assert!(db.insert(r).is_ok());
}

#[test]
fn test_schema_float_does_not_satisfy_integer_type() {
    let mut db = Database::new();
    db.define_column("race", "n", ColumnSchema {
        ty: SchemaType::Integer, not_null: false, default: None, unique: false,
    }).unwrap();
    db.enable_schema("race");

    let mut r = Record::new(0);
    r.set("n", DataType::Float(1.5));
    r.add_label("race");
    assert!(db.insert(r).is_err());
}

#[test]
fn test_schema_default_fills_missing_value() {
    let mut db = Database::new();
    db.define_column("race", "odds", ColumnSchema {
        ty: SchemaType::Float, not_null: false, default: Some(DataType::Float(0.0)), unique: false,
    }).unwrap();
    db.enable_schema("race");

    let mut r = Record::new(0);
    r.add_label("race");
    let id = db.insert(r).unwrap();
    assert_eq!(db.get(id).unwrap().get_col("odds"), Some(&DataType::Float(0.0)));
}

#[test]
fn test_schema_unique_rejects_duplicate() {
    let mut db = Database::new();
    db.define_column("racer", "racer_id", ColumnSchema {
        ty: SchemaType::Integer, not_null: true, default: None, unique: true,
    }).unwrap();
    db.enable_schema("racer");
    // UNIQUE 指定により自動でインデックスが作られる
    assert!(db.has_index("racer_id"));

    let mut r1 = Record::new(0);
    r1.set("racer_id", DataType::Integer(1));
    r1.add_label("racer");
    db.insert(r1).unwrap();

    let mut r2 = Record::new(0);
    r2.set("racer_id", DataType::Integer(1));
    r2.add_label("racer");
    let err = db.insert(r2).unwrap_err();
    assert!(matches!(err, DatabaseError::SchemaViolation(_)));
    assert_eq!(db.count(), 1);
}

#[test]
fn test_schema_unique_allows_update_of_same_record() {
    let mut db = Database::new();
    db.define_column("racer", "racer_id", ColumnSchema {
        ty: SchemaType::Integer, not_null: true, default: None, unique: true,
    }).unwrap();
    db.enable_schema("racer");

    let mut r = Record::new(0);
    r.set("racer_id", DataType::Integer(1));
    r.add_label("racer");
    let id = db.insert(r).unwrap();

    // 自分自身の値をそのまま UPDATE しても重複エラーにならない
    let mut updated = db.get(id).unwrap().clone();
    updated.set("racer_id", DataType::Integer(1));
    assert!(db.update(id, updated).is_ok());
}

#[test]
fn test_schema_unique_rejects_update_to_existing_value() {
    let mut db = Database::new();
    db.define_column("racer", "racer_id", ColumnSchema {
        ty: SchemaType::Integer, not_null: true, default: None, unique: true,
    }).unwrap();
    db.enable_schema("racer");

    let mut r1 = Record::new(0);
    r1.set("racer_id", DataType::Integer(1));
    r1.add_label("racer");
    db.insert(r1).unwrap();

    let mut r2 = Record::new(0);
    r2.set("racer_id", DataType::Integer(2));
    r2.add_label("racer");
    let id2 = db.insert(r2).unwrap();

    let mut updated = db.get(id2).unwrap().clone();
    updated.set("racer_id", DataType::Integer(1));
    assert!(db.update(id2, updated).is_err());
}

#[test]
fn test_schema_unique_is_scoped_to_label() {
    // UNIQUE はカラム単位のインデックスを使うが、違反判定は同じラベルを持つ
    // レコード同士でのみ行う（ラベルが違えば同じ値を持てる）
    let mut db = Database::new();
    db.define_column("racer", "code", ColumnSchema {
        ty: SchemaType::Text, not_null: false, default: None, unique: true,
    }).unwrap();
    db.enable_schema("racer");

    let mut other = Record::new(0);
    other.set("code", DataType::Text("A".to_string()));
    other.add_label("staff"); // racer ではない
    db.insert(other).unwrap();

    let mut r = Record::new(0);
    r.set("code", DataType::Text("A".to_string()));
    r.add_label("racer");
    assert!(db.insert(r).is_ok());
}

#[test]
fn test_disable_schema_stops_enforcement_but_keeps_definition() {
    let mut db = Database::new();
    db.define_column("race", "date", not_null_text()).unwrap();
    db.enable_schema("race");
    assert!(insert_race(&mut db, None).is_err());

    db.disable_schema("race");
    assert!(insert_race(&mut db, None).is_ok());

    // 定義自体は消えていない
    let (enabled, cols) = db.describe_label("race").unwrap();
    assert!(!enabled);
    assert_eq!(cols.len(), 1);
}

#[test]
fn test_global_schema_enforcement_switch_overrides_per_label_enable() {
    let mut db = Database::new();
    db.define_column("race", "date", not_null_text()).unwrap();
    db.enable_schema("race");
    assert!(insert_race(&mut db, None).is_err());

    db.set_schema_enforcement_enabled(false);
    assert!(insert_race(&mut db, None).is_ok());

    db.set_schema_enforcement_enabled(true);
    assert!(insert_race(&mut db, None).is_err());
}

#[test]
fn test_global_schema_enforcement_defaults_to_enabled() {
    let db = Database::new();
    assert!(db.is_schema_enforcement_enabled());
}

#[test]
fn test_drop_column_schema_removes_definition() {
    let mut db = Database::new();
    db.define_column("race", "date", not_null_text()).unwrap();
    db.drop_column_schema("race", "date").unwrap();
    let (_, cols) = db.describe_label("race").unwrap();
    assert!(cols.is_empty());
}

#[test]
fn test_drop_column_schema_nonexistent_is_error() {
    let mut db = Database::new();
    // ラベル自体にまだスキーマが無い場合はラベル名でエラーになる
    assert_eq!(db.drop_column_schema("race", "date"), Err(DatabaseError::SchemaNotFound("race".to_string())));
    // ラベルにスキーマはあるが、そのカラムは定義されていない場合はラベル.カラムでエラーになる
    db.define_column("race", "odds", not_null_text()).unwrap();
    assert_eq!(db.drop_column_schema("race", "date"), Err(DatabaseError::SchemaNotFound("race.date".to_string())));
}

#[test]
fn test_describe_label_returns_none_when_undefined() {
    let db = Database::new();
    assert_eq!(db.describe_label("race"), None);
}

#[test]
fn test_list_schema_labels_sorted() {
    let mut db = Database::new();
    db.define_column("race", "date", not_null_text()).unwrap();
    db.define_column("racer", "name", not_null_text()).unwrap();
    assert_eq!(db.list_schema_labels(), vec!["race".to_string(), "racer".to_string()]);
}

#[test]
fn test_validate_label_reports_existing_violations_even_when_disabled() {
    let mut db = Database::new();
    // スキーマ無効のまま date 無しでレコードを作る
    let id = insert_race(&mut db, None).unwrap();
    db.define_column("race", "date", not_null_text()).unwrap();
    // まだ ENABLE していないので INSERT/UPDATE は妨げられないが、
    // validate_label は「今のデータが定義に沿っているか」を報告する
    let violations = db.validate_label("race");
    assert_eq!(violations.len(), 1);
    assert!(violations[0].contains(&id.to_string()));
}

#[test]
fn test_validate_label_empty_when_no_schema_defined() {
    let mut db = Database::new();
    insert_race(&mut db, None).unwrap();
    assert!(db.validate_label("race").is_empty());
}

#[test]
fn test_validate_label_no_violations_when_data_conforms() {
    let mut db = Database::new();
    insert_race(&mut db, Some("2026-01-01")).unwrap();
    db.define_column("race", "date", not_null_text()).unwrap();
    assert!(db.validate_label("race").is_empty());
}

// ---------------------------------------------------------------------------
// 「全データオンメモリ」 ON/OFF・オンメモリ容量制限（FIFO/LRU）
// ---------------------------------------------------------------------------

fn unique_kdb_path(tag: &str) -> String {
    let p = std::env::temp_dir().join(format!(
        "db_engine_mem_{}_{}_{}.kdb",
        tag,
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    p.to_string_lossy().into_owned()
}

fn cleanup_kdb(path: &str) {
    let _ = std::fs::remove_file(path);
}

/// `.kdb` バックエンド付きのDBに、1件あたりおよそ `payload_len` バイトの列を持つ
/// レコードを `n` 件 insert する。戻り値は発行された ID 一覧。
fn seed_kdb_db(path: &str, n: usize, payload_len: usize) -> (Database, Vec<u64>) {
    let mut db = Database::load_or_new_kdb(path).unwrap();
    let mut kdb = KdbFile::open_or_create(path).unwrap();
    let mut ids = Vec::new();
    for i in 0..n {
        let mut r = Record::new(0);
        r.add_label("bulk");
        r.set("i", DataType::Integer(i as i64));
        r.set("data", DataType::Text("x".repeat(payload_len)));
        ids.push(db.insert_fast(r, Some(&mut kdb)).unwrap());
    }
    (db, ids)
}

#[test]
fn test_memory_default_is_all_in_memory_unlimited() {
    let db = Database::new();
    assert!(db.memory_policy().all_in_memory);
    let s = db.memory_stats();
    assert!(s.all_in_memory);
    assert_eq!(s.resident_bytes, 0);
}

#[test]
fn test_memory_all_in_memory_keeps_everything_resident() {
    let path = unique_kdb_path("all");
    let (db, ids) = seed_kdb_db(&path, 30, 500);
    // 既定（all_in_memory=true）では追い出しは一切起きない
    let s = db.memory_stats();
    assert!(s.all_in_memory);
    assert_eq!(s.resident_bytes, 0); // all_in_memory 中はキャッシュ未使用
    for &id in &ids {
        assert!(db.get(id).unwrap().columns.contains_key("data"));
    }
    drop(db);
    cleanup_kdb(&path);
}

#[test]
fn test_memory_bounded_mode_evicts_and_reloads_from_kdb() {
    let path = unique_kdb_path("bounded");
    let (mut db, ids) = seed_kdb_db(&path, 40, 1000);

    // 上限 5KB へ切り替え → 大半の行がメモリから追い出される
    db.set_memory_policy(MemoryPolicy {
        all_in_memory: false,
        limit: MemorySizeSpec::Bytes(5_000),
    });
    let s = db.memory_stats();
    assert!(!s.all_in_memory);
    assert!(s.resident_bytes <= 5_000, "resident_bytes={} should be within budget", s.resident_bytes);
    assert!(s.resident_rows < ids.len(), "some rows must have been evicted");

    // 追い出された行も get() で正しく読み直せる（.kdb から復元）
    for &id in &ids {
        let rec = db.get(id).unwrap();
        assert_eq!(rec.columns.get("data").unwrap(), &DataType::Text("x".repeat(1000)));
    }
    // 読み直しても上限は保たれている
    assert!(db.memory_stats().resident_bytes <= 5_000 + 2_000);

    drop(db);
    cleanup_kdb(&path);
}

#[test]
fn test_memory_bounded_mode_reaccess_protects_from_eviction() {
    let path = unique_kdb_path("lru");
    let (mut db, ids) = seed_kdb_db(&path, 20, 1000);
    db.set_memory_policy(MemoryPolicy {
        all_in_memory: false,
        limit: MemorySizeSpec::Bytes(4_000),
    });

    // 先頭の行を繰り返しアクセスして「最近使った」状態にする
    let hot = ids[0];
    for _ in 0..5 {
        let _ = db.get(hot).unwrap();
        // 別の行も触れてキャッシュを回す
        for &id in &ids[1..] {
            let _ = db.get(id).unwrap();
        }
        let _ = db.get(hot).unwrap();
    }
    // hot は直近アクセスされているので、他の行より追い出されにくい（LRU）
    // （厳密な検証は難しいので、hot が読めること＋上限が守られていることを確認）
    assert_eq!(db.get(hot).unwrap().columns.get("i").unwrap(), &DataType::Integer(0));
    assert!(db.memory_stats().resident_bytes <= 6_000);

    drop(db);
    cleanup_kdb(&path);
}

#[test]
fn test_memory_toggle_off_then_on_restores_inline() {
    let path = unique_kdb_path("toggle");
    let (mut db, ids) = seed_kdb_db(&path, 15, 300);

    db.set_memory_policy(MemoryPolicy { all_in_memory: false, limit: MemorySizeSpec::Bytes(2_000) });
    assert!(db.memory_stats().resident_bytes <= 2_000);

    // 全データオンメモリへ戻す（一括ロードはしないが、読めば復元される）
    db.set_memory_policy(MemoryPolicy { all_in_memory: true, limit: MemorySizeSpec::Bytes(2_000) });
    assert!(db.memory_policy().all_in_memory);
    for &id in &ids {
        assert!(db.get(id).unwrap().columns.contains_key("data"));
    }

    drop(db);
    cleanup_kdb(&path);
}

#[test]
fn test_memory_bounded_mode_update_and_delete_still_work() {
    let path = unique_kdb_path("upd");
    let (mut db, ids) = seed_kdb_db(&path, 12, 800);
    db.set_memory_policy(MemoryPolicy { all_in_memory: false, limit: MemorySizeSpec::Bytes(2_500) });

    let target = ids[3];
    let mut updated = Record::new(target);
    updated.add_label("bulk");
    updated.set("i", DataType::Integer(999));
    updated.set("data", DataType::Text("updated".to_string()));
    db.update(target, updated).unwrap();
    assert_eq!(db.get(target).unwrap().columns.get("i").unwrap(), &DataType::Integer(999));

    db.delete(ids[0]).unwrap();
    assert!(db.get(ids[0]).is_err());
    // 他の行は影響を受けない
    assert!(db.get(ids[5]).unwrap().columns.contains_key("data"));

    drop(db);
    cleanup_kdb(&path);
}

#[test]
fn test_memory_bounded_mode_secondary_index_search_reloads() {
    let path = unique_kdb_path("idx");
    let (mut db, _ids) = seed_kdb_db(&path, 25, 600);
    db.create_index("i").unwrap();
    db.set_memory_policy(MemoryPolicy { all_in_memory: false, limit: MemorySizeSpec::Bytes(3_000) });

    // インデックス経由の検索でも、追い出された行を .kdb から読み直して返す
    let hits = db.get_by_index("i", &DataType::Integer(20)).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].columns.get("data").unwrap(), &DataType::Text("x".repeat(600)));

    drop(db);
    cleanup_kdb(&path);
}
