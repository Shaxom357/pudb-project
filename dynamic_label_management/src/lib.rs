// src/lib.rs
// dynamic_label_management: ラベルの動的管理ライブラリ

pub mod error;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use db_engine::{Database, Record};
use error::LabelError;

/// ラベル名バリデーション: 英数字・[-:_/] のみ使用可能
fn validate_label(label: &str) -> Result<(), LabelError> {
    if label.is_empty() {
        return Err(LabelError::EmptyLabelName);
    }
    let invalid: Vec<char> = label
        .chars()
        .filter(|c| !c.is_alphanumeric() && !matches!(c, '-' | ':' | '_' | '/'))
        .collect();
    if !invalid.is_empty() {
        return Err(LabelError::InvalidLabelName(format!(
            "invalid chars {:?} in label '{}'", invalid, label
        )));
    }
    Ok(())
}

/// ラベルの統計情報
#[derive(Debug, Clone, PartialEq)]
pub struct LabelStats {
    pub label: String,
    pub record_count: usize,
}

/// ラベル動的管理の本体
pub struct LabelManager {
    db: Database,
}

impl LabelManager {
    pub fn new() -> Self {
        LabelManager { db: Database::new() }
    }
    pub fn from_db(db: Database) -> Self {
        LabelManager { db }
    }
    pub fn db(&self) -> &Database { &self.db }
    pub fn db_mut(&mut self) -> &mut Database { &mut self.db }

    /// レコードを挿入する（id=0 で自動採番）
    pub fn insert_record(&mut self, record: Record) -> Result<u64, LabelError> {
        self.db.insert(record).map_err(LabelError::from)
    }

    // -- 単一レコードのラベル操作 --

    /// ラベルを追加（冪等）
    pub fn add_label(&mut self, record_id: u64, label: &str) -> Result<(), LabelError> {
        validate_label(label)?;
        self.db.attach_label(record_id, label).map_err(LabelError::from)
    }

    /// ラベルを削除（戻り値: true=削除した, false=なかった）
    pub fn remove_label(&mut self, record_id: u64, label: &str) -> Result<bool, LabelError> {
        validate_label(label)?;
        self.db.detach_label(record_id, label).map_err(LabelError::from)
    }

    /// レコードのラベル一覧（ソート済み）
    pub fn list_labels(&self, record_id: u64) -> Result<Vec<String>, LabelError> {
        let record = self.db.get(record_id).map_err(LabelError::from)?;
        let mut labels = record.labels.clone();
        labels.sort();
        Ok(labels)
    }

    // -- 一括操作 --

    /// DB全体のラベル一覧（重複なし・ソート済み）
    pub fn list_all_labels(&self) -> Vec<String> {
        self.db.list_all_labels()
    }

    /// ラベルが付いているレコード数
    pub fn label_count(&self, label: &str) -> usize {
        self.db.get_by_label(label).len()
    }

    /// 全ラベルの統計情報（レコード数降順）
    pub fn label_stats(&self) -> Vec<LabelStats> {
        let mut stats: Vec<LabelStats> = self.db.list_all_labels()
            .into_iter()
            .map(|label| {
                let count = self.db.get_by_label(&label).len();
                LabelStats { label, record_count: count }
            })
            .collect();
        stats.sort_by(|a, b| {
            b.record_count.cmp(&a.record_count)
                .then(a.label.cmp(&b.label))
        });
        stats
    }

    // -- 検索 --

    /// 指定ラベルを持つレコードの ID 一覧
    pub fn find_by_label(&self, label: &str) -> Vec<u64> {
        self.db.get_by_label(label).iter().map(|r| r.id).collect()
    }

    /// 指定ラベルをすべて持つレコードの ID 一覧（AND 検索）
    pub fn find_by_labels_all(&self, labels: &[&str]) -> Vec<u64> {
        if labels.is_empty() {
            return self.db.list_all().iter().map(|r| r.id).collect();
        }
        let mut id_set: HashSet<u64> = self.db
            .get_by_label(labels[0]).iter().map(|r| r.id).collect();
        for &label in &labels[1..] {
            let next: HashSet<u64> = self.db
                .get_by_label(label).iter().map(|r| r.id).collect();
            id_set = id_set.intersection(&next).cloned().collect();
        }
        let mut ids: Vec<u64> = id_set.into_iter().collect();
        ids.sort();
        ids
    }

    /// 指定ラベルのいずれかを持つレコードの ID 一覧（OR 検索）
    pub fn find_by_labels_any(&self, labels: &[&str]) -> Vec<u64> {
        let mut id_set: HashSet<u64> = HashSet::new();
        for &label in labels {
            for r in self.db.get_by_label(label) {
                id_set.insert(r.id);
            }
        }
        let mut ids: Vec<u64> = id_set.into_iter().collect();
        ids.sort();
        ids
    }

    // -- ラベルのリネーム --

    /// DB全体で old_label を new_label にリネームする。戻り値: 変更レコード数
    pub fn rename_label(&mut self, old_label: &str, new_label: &str) -> Result<usize, LabelError> {
        validate_label(old_label)?;
        validate_label(new_label)?;
        let target_ids: Vec<u64> = self.db
            .get_by_label(old_label).iter().map(|r| r.id).collect();
        let count = target_ids.len();
        for id in target_ids {
            self.db.detach_label(id, old_label).map_err(LabelError::from)?;
            // 対象レコードが元々 new_label も併せ持っていた場合は付与済みなので付け直さない
            // （attach_label は重複付与をエラーにするため）
            let already_has_new = self.db.get(id).map_err(LabelError::from)?
                .labels.contains(&new_label.to_string());
            if !already_has_new {
                self.db.attach_label(id, new_label).map_err(LabelError::from)?;
            }
        }
        Ok(count)
    }

    // -- ラベルのコピー --

    /// src_id のラベルを dst_id にすべてコピーする。dst_id が既に持っているラベルはスキップする
    /// （attach_label は重複付与をエラーにするため）。戻り値: 新たにコピーした数
    pub fn copy_labels(&mut self, src_id: u64, dst_id: u64) -> Result<usize, LabelError> {
        let src_labels: Vec<String> = self.db
            .get(src_id).map_err(LabelError::from)?.labels.clone();
        let dst_labels: HashSet<String> = self.db
            .get(dst_id).map_err(LabelError::from)?.labels.iter().cloned().collect();
        let mut copied = 0;
        for label in src_labels {
            if dst_labels.contains(&label) { continue; }
            self.db.attach_label(dst_id, &label).map_err(LabelError::from)?;
            copied += 1;
        }
        Ok(copied)
    }

    // -- ラベルの差分 --

    /// 2つのレコードのラベル差分。戻り値: (only_in_a, only_in_b, in_both)
    pub fn diff_labels(
        &self, id_a: u64, id_b: u64,
    ) -> Result<(Vec<String>, Vec<String>, Vec<String>), LabelError> {
        let labels_a: HashSet<String> = self.db
            .get(id_a).map_err(LabelError::from)?
            .labels.iter().cloned().collect();
        let labels_b: HashSet<String> = self.db
            .get(id_b).map_err(LabelError::from)?
            .labels.iter().cloned().collect();
        let mut only_a: Vec<String> = labels_a.difference(&labels_b).cloned().collect();
        let mut only_b: Vec<String> = labels_b.difference(&labels_a).cloned().collect();
        let mut both:   Vec<String> = labels_a.intersection(&labels_b).cloned().collect();
        only_a.sort(); only_b.sort(); both.sort();
        Ok((only_a, only_b, both))
    }

    // -- グルーピング --

    /// 「ラベル -> レコードIDリスト」のマップを返す
    pub fn group_by_label(&self) -> HashMap<String, Vec<u64>> {
        self.db.list_all_labels().into_iter().map(|label| {
            let ids: Vec<u64> = self.db
                .get_by_label(&label).iter().map(|r| r.id).collect();
            (label, ids)
        }).collect()
    }
}

impl Default for LabelManager {
    fn default() -> Self { Self::new() }
}
