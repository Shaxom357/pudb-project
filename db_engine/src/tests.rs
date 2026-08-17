// db_engine/src/tests.rs

use crate::{Database, DataType, DatabaseError, Record};

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
