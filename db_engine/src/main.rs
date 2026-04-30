use db_engine::Database;

fn main() {
    let mut db = Database::new();
    db.insert(1);
    db.insert(2);
    db.insert(3);

    match db.get(1) {
        Some(value) => println!("Value at index 1: {}", value),
        None => println!("No value at index 1"),
    }
}