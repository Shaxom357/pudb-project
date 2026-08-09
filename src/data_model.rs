// data_model.rs

// 異なるデータ型とオプション値を扱うためのDataType列挙型を定義します
#[derive(Debug, PartialEq)]
enum DataType {
    String(Option<String>),
    Integer(Option<i32>),
    Float(Option<f64>),
    // Add other data types as needed
}

// データベースの行を表すDataRow構造体を定義します
#[derive(Debug, PartialEq)]
struct DataRow {
    employee_id: Option<i32>,
    product_id: Option<i32>,
    department_id: Option<i32>,
    // Add other columns as needed
    label1: DataType,
    label2: DataType,
    label3: DataType,
}

// データベースの物理ストレージを表すColumnStore構造体を定義します
struct ColumnStore {
    employee_id_column: Vec<Option<i32>>,
    product_id_column: Vec<Option<i32>>,
    department_id_column: Vec<Option<i32>>,
    // Add other columns as needed
    label_index: std::collections::HashMap<String, Vec<usize>>, // Lableごとの行インデックス
}

impl ColumnStore {
    fn new() -> Self {
        ColumnStore {
            employee_id_column: Vec::new(),
            product_id_column: Vec::new(),
            department_id_column: Vec::new(),
            // Initialize other columns as needed
            label_index: std::collections::HashMap::new(),
        }
    }

    // Implement basic CRUD functionality here

impl ColumnStore {
    fn create(&mut self, data_row: DataRow) {
        // Add the data row to the appropriate columns
        self.employee_id_column.push(data_row.employee_id);
        self.product_id_column.push(data_row.product_id);
        self.department_id_column.push(data_row.department_id);
        // Add other columns as needed

        // Update the label index
        if let Some(label1) = match data_row.label1 {
            DataType::String(Some(label)) => Some(label),
            _ => None,
        } {
            self.label_index.entry(label1).or_insert_with(Vec::new).push(self.employee_id_column.len() - 1);
        }
        if let Some(label2) = match data_row.label2 {
            DataType::String(Some(label)) => Some(label),
            _ => None,
        } {
            self.label_index.entry(label2).or_insert_with(Vec::new).push(self.employee_id_column.len() - 1);
        }
        if let Some(label3) = match data_row.label3 {
            DataType::String(Some(label)) => Some(label),
            _ => None,
        } {
            self.label_index.entry(label3).or_insert_with(Vec::new).push(self.employee_id_column.len() - 1);
        }
    }

    fn read(&self, label: &str) -> Option<Vec<DataRow>> {
        self.label_index.get(label).map(|indices| {
            indices.iter().map(|&index| {
                DataRow {
                    employee_id: self.employee_id_column.get(index).cloned(),
                    product_id: self.product_id_column.get(index).cloned(),
                    department_id: self.department_id_column.get(index).cloned(),
                    // Add other columns as needed
                    label1: match self.employee_id_column.get(index) {
                        Some(id) => DataType::Integer(Some(*id)),
                        None => DataType::Integer(None),
                    },
                    label2: match self.product_id_column.get(index) {
                        Some(id) => DataType::Integer(Some(*id)),
                        None => DataType::Integer(None),
                    },
                    label3: match self.department_id_column.get(index) {
                        Some(id) => DataType::Integer(Some(*id)),
                        None => DataType::Integer(None),
                    },
                }
            }).collect()
        })
    }

    fn update(&mut self, index: usize, data_row: DataRow) {
        // Update the data row in the appropriate columns
        if index < self.employee_id_column.len() {
            self.employee_id_column[index] = data_row.employee_id;
            self.product_id_column[index] = data_row.product_id;
            self.department_id_column[index] = data_row.department_id;
            // Update other columns as needed

            // Update the label index
            self.label_index.retain(|_, v| {
                v.retain(|&i| i!= index);
                true
            });
            if let Some(label1) = match data_row.label1 {
                DataType::String(Some(label)) => Some(label),
                _ => None,
            } {
                self.label_index.entry(label1).or_insert_with(Vec::new).push(index);
            }
            if let Some(label2) = match data_row.label2 {
                DataType::String(Some(label)) => Some(label),
                _ => None,
            } {
                self.label_index.entry(label2).or_insert_with(Vec::new).push(index);
            }
            if let Some(label3) = match data_row.label3 {
                DataType::String(Some(label)) => Some(label),
                _ => None,
            } {
                self.label_index.entry(label3).or_insert_with(Vec::new).push(index);
            }
        }
    }

    fn delete(&mut self, index: usize) {
        if index < self.employee_id_column.len() {
            self.employee_id_column.remove(index);
            self.product_id_column.remove(index);
            self.department_id_column.remove(index);
            // Remove other columns as needed

            // Update the label index
            self.label_index.retain(|_, v| {
                v.retain(|&i| i!= index);
                true
            });
        }
    }
}

pub struct MainTable {
    pub rows: Vec<DataRow>,
}

impl MainTable {
    pub fn new() -> Self {
        MainTable { rows: Vec::new() }
    }

    pub fn insert(&mut self, row: DataRow) {
        self.rows.push(row);
    }

pub fn get_by_label(&self, label: &str) -> Vec<&DataRow> {
    self.rows
    .iter()
    .filter(|row| {
            if let DataType::String(Some(ref label_value)) = row.label1 {
                label_value == label
            } else {
                false
            }
        })
    .collect()
}
}
}