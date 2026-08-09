// src/tests.rs

use db_engine::{DataType, Record};
use crate::{LabelManager, LabelStats};
use crate::error::LabelError;

fn insert_named(mgr: &mut LabelManager, name: &str, labels: &[&str]) -> u64 {
    let mut r = Record::new(0);
    r.set("name", DataType::Text(name.to_string()));
    for &l in labels { r.add_label(l); }
    mgr.insert_record(r).unwrap()
}

// -- add_label / remove_label / list_labels --

#[test]
fn test_add_label_basic() {
    let mut mgr = LabelManager::new();
    let id = insert_named(&mut mgr, "Alice", &[]);
    mgr.add_label(id, "env:prod").unwrap();
    assert_eq!(mgr.list_labels(id).unwrap(), vec!["env:prod"]);
}

#[test]
fn test_add_label_idempotent() {
    let mut mgr = LabelManager::new();
    let id = insert_named(&mut mgr, "Alice", &[]);
    mgr.add_label(id, "tag").unwrap();
    mgr.add_label(id, "tag").unwrap();
    assert_eq!(mgr.list_labels(id).unwrap().len(), 1);
}

#[test]
fn test_add_label_invalid_name() {
    let mut mgr = LabelManager::new();
    let id = insert_named(&mut mgr, "Alice", &[]);
    assert!(matches!(mgr.add_label(id, ""), Err(LabelError::EmptyLabelName)));
    assert!(matches!(mgr.add_label(id, "bad label"), Err(LabelError::InvalidLabelName(_))));
    assert!(matches!(mgr.add_label(id, "bad!"), Err(LabelError::InvalidLabelName(_))));
}

#[test]
fn test_add_label_nonexistent_record() {
    let mut mgr = LabelManager::new();
    assert!(matches!(mgr.add_label(999, "tag"), Err(LabelError::RecordNotFound(999))));
}

#[test]
fn test_remove_label_basic() {
    let mut mgr = LabelManager::new();
    let id = insert_named(&mut mgr, "Bob", &["env:prod"]);
    let removed = mgr.remove_label(id, "env:prod").unwrap();
    assert!(removed);
    assert!(mgr.list_labels(id).unwrap().is_empty());
}

#[test]
fn test_remove_label_not_present_returns_false() {
    let mut mgr = LabelManager::new();
    let id = insert_named(&mut mgr, "Bob", &[]);
    assert!(!mgr.remove_label(id, "noexist").unwrap());
}

#[test]
fn test_remove_label_invalid_name() {
    let mut mgr = LabelManager::new();
    let id = insert_named(&mut mgr, "Bob", &[]);
    assert!(matches!(mgr.remove_label(id, ""), Err(LabelError::EmptyLabelName)));
}

#[test]
fn test_list_labels_sorted() {
    let mut mgr = LabelManager::new();
    let id = insert_named(&mut mgr, "Carol", &["z-label", "a-label", "m-label"]);
    assert_eq!(mgr.list_labels(id).unwrap(), vec!["a-label", "m-label", "z-label"]);
}

#[test]
fn test_list_labels_nonexistent_record() {
    let mgr = LabelManager::new();
    assert!(matches!(mgr.list_labels(999), Err(LabelError::RecordNotFound(999))));
}

// -- list_all_labels / label_count / label_stats --

#[test]
fn test_list_all_labels() {
    let mut mgr = LabelManager::new();
    insert_named(&mut mgr, "Alice", &["env:prod", "region:ap"]);
    insert_named(&mut mgr, "Bob",   &["env:dev"]);
    assert_eq!(mgr.list_all_labels(), vec!["env:dev", "env:prod", "region:ap"]);
}

#[test]
fn test_label_count() {
    let mut mgr = LabelManager::new();
    insert_named(&mut mgr, "Alice", &["team:a"]);
    insert_named(&mut mgr, "Bob",   &["team:a"]);
    insert_named(&mut mgr, "Carol", &["team:b"]);
    assert_eq!(mgr.label_count("team:a"), 2);
    assert_eq!(mgr.label_count("team:b"), 1);
    assert_eq!(mgr.label_count("team:c"), 0);
}

#[test]
fn test_label_stats_order() {
    let mut mgr = LabelManager::new();
    insert_named(&mut mgr, "Alice", &["popular", "rare"]);
    insert_named(&mut mgr, "Bob",   &["popular"]);
    insert_named(&mut mgr, "Carol", &["popular"]);
    let stats = mgr.label_stats();
    assert_eq!(stats[0], LabelStats { label: "popular".to_string(), record_count: 3 });
    assert_eq!(stats[1], LabelStats { label: "rare".to_string(),    record_count: 1 });
}

// -- find_by_label / find_by_labels_all / find_by_labels_any --

#[test]
fn test_find_by_label() {
    let mut mgr = LabelManager::new();
    let id1 = insert_named(&mut mgr, "Alice", &["group:A"]);
    let id2 = insert_named(&mut mgr, "Bob",   &["group:A"]);
    insert_named(&mut mgr, "Carol", &["group:B"]);
    let mut result = mgr.find_by_label("group:A");
    result.sort();
    assert_eq!(result, vec![id1, id2]);
}

#[test]
fn test_find_by_labels_all_and_search() {
    let mut mgr = LabelManager::new();
    let id1 = insert_named(&mut mgr, "Alice", &["backend", "senior"]);
    insert_named(&mut mgr, "Bob",   &["backend"]);
    insert_named(&mut mgr, "Carol", &["frontend", "senior"]);
    assert_eq!(mgr.find_by_labels_all(&["backend", "senior"]), vec![id1]);
}

#[test]
fn test_find_by_labels_any_or_search() {
    let mut mgr = LabelManager::new();
    let id1 = insert_named(&mut mgr, "Alice", &["go"]);
    let id2 = insert_named(&mut mgr, "Bob",   &["rust"]);
    insert_named(&mut mgr, "Carol", &["java"]);
    let mut result = mgr.find_by_labels_any(&["go", "rust"]);
    result.sort();
    assert_eq!(result, vec![id1, id2]);
}

#[test]
fn test_find_by_labels_all_empty_returns_all() {
    let mut mgr = LabelManager::new();
    let id1 = insert_named(&mut mgr, "Alice", &[]);
    let id2 = insert_named(&mut mgr, "Bob",   &[]);
    let mut result = mgr.find_by_labels_all(&[]);
    result.sort();
    assert_eq!(result, vec![id1, id2]);
}

// -- rename_label --

#[test]
fn test_rename_label_basic() {
    let mut mgr = LabelManager::new();
    let id1 = insert_named(&mut mgr, "Alice", &["old-name"]);
    let id2 = insert_named(&mut mgr, "Bob",   &["old-name"]);
    let count = mgr.rename_label("old-name", "new-name").unwrap();
    assert_eq!(count, 2);
    assert!(mgr.find_by_label("old-name").is_empty());
    let mut renamed = mgr.find_by_label("new-name");
    renamed.sort();
    assert_eq!(renamed, vec![id1, id2]);
}

#[test]
fn test_rename_label_no_target_returns_zero() {
    let mut mgr = LabelManager::new();
    assert_eq!(mgr.rename_label("nonexistent", "new-name").unwrap(), 0);
}

#[test]
fn test_rename_label_invalid_new_name() {
    let mut mgr = LabelManager::new();
    assert!(matches!(
        mgr.rename_label("valid", "invalid name"),
        Err(LabelError::InvalidLabelName(_))
    ));
}

// -- copy_labels --

#[test]
fn test_copy_labels_basic() {
    let mut mgr = LabelManager::new();
    let src = insert_named(&mut mgr, "Template", &["role:admin", "env:prod"]);
    let dst = insert_named(&mut mgr, "New",      &[]);
    let copied = mgr.copy_labels(src, dst).unwrap();
    assert_eq!(copied, 2);
    let dst_labels = mgr.list_labels(dst).unwrap();
    assert!(dst_labels.contains(&"role:admin".to_string()));
    assert!(dst_labels.contains(&"env:prod".to_string()));
}

#[test]
fn test_copy_labels_nonexistent_src() {
    let mut mgr = LabelManager::new();
    let dst = insert_named(&mut mgr, "New", &[]);
    assert!(matches!(mgr.copy_labels(999, dst), Err(LabelError::RecordNotFound(999))));
}

// -- diff_labels --

#[test]
fn test_diff_labels() {
    let mut mgr = LabelManager::new();
    let id_a = insert_named(&mut mgr, "A", &["common", "only-a"]);
    let id_b = insert_named(&mut mgr, "B", &["common", "only-b"]);
    let (only_a, only_b, both) = mgr.diff_labels(id_a, id_b).unwrap();
    assert_eq!(only_a, vec!["only-a"]);
    assert_eq!(only_b, vec!["only-b"]);
    assert_eq!(both,   vec!["common"]);
}

#[test]
fn test_diff_labels_identical() {
    let mut mgr = LabelManager::new();
    let id_a = insert_named(&mut mgr, "A", &["x", "y"]);
    let id_b = insert_named(&mut mgr, "B", &["x", "y"]);
    let (only_a, only_b, both) = mgr.diff_labels(id_a, id_b).unwrap();
    assert!(only_a.is_empty());
    assert!(only_b.is_empty());
    assert_eq!(both, vec!["x", "y"]);
}

// -- group_by_label --

#[test]
fn test_group_by_label() {
    let mut mgr = LabelManager::new();
    let id1 = insert_named(&mut mgr, "Alice", &["team:a"]);
    let id2 = insert_named(&mut mgr, "Bob",   &["team:a", "team:b"]);
    let id3 = insert_named(&mut mgr, "Carol", &["team:b"]);
    let groups = mgr.group_by_label();
    let mut team_a = groups["team:a"].clone(); team_a.sort();
    let mut team_b = groups["team:b"].clone(); team_b.sort();
    assert_eq!(team_a, vec![id1, id2]);
    assert_eq!(team_b, vec![id2, id3]);
}
