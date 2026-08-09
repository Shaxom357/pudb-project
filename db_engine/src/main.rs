use db_engine::{Database, DataType, Record};

fn main() {
    println!("=== DB Engine デモ ===");

    let mut db = Database::new();

    // レコードを挿入
    let mut r1 = Record::new(0);
    r1.set("name", DataType::Text("Alice".to_string()));
    r1.set("age", DataType::Integer(30));
    r1.set("score", DataType::Float(95.5));
    r1.add_label("dept:engineering");
    r1.add_label("role:backend");
    let id1 = db.insert(r1).unwrap();
    println!("Inserted id={}", id1);

    let mut r2 = Record::new(0);
    r2.set("name", DataType::Text("Bob".to_string()));
    r2.set("age", DataType::Integer(25));
    r2.set("score", DataType::Float(88.0));
    r2.add_label("dept:engineering");
    r2.add_label("role:frontend");
    let id2 = db.insert(r2).unwrap();
    println!("Inserted id={}", id2);

    let mut r3 = Record::new(0);
    r3.set("name", DataType::Text("Carol".to_string()));
    r3.set("age", DataType::Integer(35));
    r3.add_label("dept:sales");
    let id3 = db.insert(r3).unwrap();
    println!("Inserted id={}", id3);

    // IDで取得
    let record = db.get(id1).unwrap();
    println!("\nget(id={}): name={:?}, age={:?}",
        id1,
        record.get_col("name"),
        record.get_col("age"));

    // ラベルで検索
    println!("\n[label: dept:engineering]");
    for r in db.get_by_label("dept:engineering") {
        println!("  id={} name={:?}", r.id, r.get_col("name"));
    }

    // カラム値で検索
    println!("\n[search_by_column: role=backend]");
    for r in db.search_by_column("role", &DataType::Text("backend".to_string())) {
        println!("  id={} name={:?}", r.id, r.get_col("name"));
    }

    // レコード更新
    let mut updated = Record::new(0);
    updated.set("name", DataType::Text("Alice (updated)".to_string()));
    updated.set("score", DataType::Float(99.9));
    updated.add_label("dept:engineering");
    updated.add_label("role:tech-lead");
    db.update(id1, updated).unwrap();
    println!("\nAfter update(id={}):", id1);
    let r = db.get(id1).unwrap();
    println!("  name={:?}, score={:?}", r.get_col("name"), r.get_col("score"));

    // レコード削除
    db.delete(id3).unwrap();
    println!("\nAfter delete(id={}): count={}", id3, db.count());

    println!("\n[全レコード]");
    for r in db.list_all() {
        println!("  id={} name={:?} labels={:?}", r.id, r.get_col("name"), r.labels);
    }
}
