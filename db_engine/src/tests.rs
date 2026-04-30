use db_engine::Database;

#[test]
fn test_database() {
    let mut db = Database::new();
    db.insert(1);
    db.insert(2);
    db.insert(3);

    assert_eq!(db.get(0), Some(1));
    assert_eq!(db.get(1), Some(2));
    assert_eq!(db.get(2), Some(3));
    assert_eq!(db.get(3), None);
}