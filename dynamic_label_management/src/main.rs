use db_engine::{DataType, Record};
use dynamic_label_management::LabelManager;

fn main() {
    println!("=== Dynamic Label Management デモ ===");
    let mut mgr = LabelManager::new();

    // レコードを挿入
    let mut r1 = Record::new(0);
    r1.set("name", DataType::Text("Alice".to_string()));
    r1.add_label("dept:engineering"); r1.add_label("role:backend"); r1.add_label("env:prod");
    let id1 = mgr.insert_record(r1).unwrap();

    let mut r2 = Record::new(0);
    r2.set("name", DataType::Text("Bob".to_string()));
    r2.add_label("dept:engineering"); r2.add_label("role:frontend"); r2.add_label("env:prod");
    let id2 = mgr.insert_record(r2).unwrap();

    let mut r3 = Record::new(0);
    r3.set("name", DataType::Text("Carol".to_string()));
    r3.add_label("dept:sales"); r3.add_label("env:prod");
    let id3 = mgr.insert_record(r3).unwrap();

    let mut r4 = Record::new(0);
    r4.set("name", DataType::Text("Dave".to_string()));
    r4.add_label("dept:engineering"); r4.add_label("role:backend"); r4.add_label("env:staging");
    let id4 = mgr.insert_record(r4).unwrap();

    println!("\n[全ラベル一覧]");
    for l in mgr.list_all_labels() { println!("  {}", l); }

    println!("\n[ラベル統計]");
    for s in mgr.label_stats() {
        println!("  {:25} : {} 件", s.label, s.record_count);
    }

    println!("\n[AND検索: backend AND env:prod]");
    for id in mgr.find_by_labels_all(&["role:backend", "env:prod"]) {
        println!("  id={}", id);
    }

    println!("\n[OR検索: role:backend OR role:frontend]");
    for id in mgr.find_by_labels_any(&["role:backend", "role:frontend"]) {
        println!("  id={}", id);
    }

    println!("\n[ラベル追加: id={} に senior を付与]", id1);
    mgr.add_label(id1, "role:senior").unwrap();
    println!("  id={} のラベル: {:?}", id1, mgr.list_labels(id1).unwrap());

    println!("\n[ラベル削除: id={} から env:prod を削除]", id2);
    mgr.remove_label(id2, "env:prod").unwrap();
    println!("  id={} のラベル: {:?}", id2, mgr.list_labels(id2).unwrap());

    println!("\n[ラベルリネーム: env:prod -> environment:production]");
    let n = mgr.rename_label("env:prod", "environment:production").unwrap();
    println!("  {} 件変更", n);
    println!("  environment:production の件数: {}", mgr.label_count("environment:production"));

    println!("\n[ラベルコピー: id={} -> id={}]", id1, id3);
    let copied = mgr.copy_labels(id1, id3).unwrap();
    println!("  {} 個コピー", copied);
    println!("  id={} のラベル: {:?}", id3, mgr.list_labels(id3).unwrap());

    println!("\n[ラベル差分: id={} vs id={}]", id1, id4);
    let (only_a, only_b, both) = mgr.diff_labels(id1, id4).unwrap();
    println!("  id={} のみ: {:?}", id1, only_a);
    println!("  id={} のみ: {:?}", id4, only_b);
    println!("  共通:     {:?}", both);

    println!("\n[グルーピング]");
    let mut groups: Vec<_> = mgr.group_by_label().into_iter().collect();
    groups.sort_by_key(|(k, _)| k.clone());
    for (label, ids) in groups {
        println!("  {:30} -> {:?}", label, ids);
    }
}
