// tests.rs

use super::data_model::{ColumnStore, DataRow, DataType};

#[test]
fn test_crud_functionality() {
    let mut column_store = ColumnStore::new();

    // Create a data row
    let data_row = DataRow {
        employee_id: Some(1),
        product_id: Some(2),
        department_id: Some(3),
        label1: DataType::String(Some("employee".to_string())),
        label2: DataType::String(Some("product".to_string())),
        label3: DataType::String(Some("department".to_string())),
    };
    column_store.create(data_row);

    // Read the data row
    let read_rows = column_store.read("employee");
    assert!(read_rows.is_some());
    let read_row = read_rows.unwrap().get(0).cloned();
    assert!(read_row.is_some());
    assert_eq!(read_row.unwrap().employee_id, Some(1));
    assert_eq!(read_row.unwrap().product_id, Some(2));
    assert_eq!(read_row.unwrap().department_id, Some(3));

    // Update the data row
    let updated_data_row = DataRow {
        employee_id: Some(4),
        product_id: Some(5),
        department_id: Some(6),
        label1: DataType::String(Some("updated_employee".to_string())),
        label2: DataType::String(Some("updated_product".to_string())),
        label3: DataType::String(Some("updated_department".to_string())),
    };
    column_store.update(0, updated_data_row);

    // Read the updated data row
    let read_rows = column_store.read("updated_employee");
    assert!(read_rows.is_some());
    let read_row = read_rows.unwrap().get(0).cloned();
    assert!(read_row.is_some());
    assert_eq!(read_row.unwrap().employee_id, Some(4));
    assert_eq!(read_row.unwrap().product_id, Some(5));
    assert_eq!(read_row.unwrap().department_id, Some(6));

    // Delete the data row
    column_store.delete(0);

    // Read the deleted data row
    let read_rows = column_store.read("updated_employee");
    assert!(read_rows.is_some());
    let read_row = read_rows.unwrap().get(0).cloned();
    assert!(read_row.is_none());
}

#[test]
fn test_empty_array() {
    let empty_array: Vec<i32> = Vec::new();
    assert_eq!(get_value(&empty_array, 1), None);
}
    let mut column_store = ColumnStore::new();

    // Create a data row
    let data_row = DataRow {
        employee_id: Some(1),
        product_id: Some(2),
        department_id: Some(3),
        label1: DataType::String(Some("employee".to_string())),
        label2: DataType::String(Some("product".to_string())),
        label3: DataType::String(Some("department".to_string())),
    };
    column_store.create(data_row);

    // Read the data row
    let read_rows = column_store.read("employee");
    assert!(read_rows.is_some());
    let read_row = read_rows.unwrap().get(0).cloned();
    assert!(read_row.is_some());
    assert_eq!(read_row.unwrap().employee_id, Some(1));
    assert_eq!(read_row.unwrap().product_id, Some(2));
    assert_eq!(read_row.unwrap().department_id, Some(3));

    // Update the data row
    let updated_data_row = DataRow {
        employee_id: Some(4),
        product_id: Some(5),
        department_id: Some(6),
        label1: DataType::String(Some("updated_employee".to_string())),
        label2: DataType::String(Some("updated_product".to_string())),
        label3: DataType::String(Some("updated_department".to_string())),
    };
    column_store.update(0, updated_data_row);

    // Read the updated data row
    let read_rows = column_store.read("updated_employee");
    assert!(read_rows.is_some());
    let read_row = read_rows.unwrap().get(0).cloned();
    assert!(read_row.is_some());
    assert_eq!(read_row.unwrap().employee_id, Some(4));
    assert_eq!(read_row.unwrap().product_id, Some(5));
    assert_eq!(read_row.unwrap().department_id, Some(6));

    // Delete the data row
    column_store.delete(0);

    // Read the deleted data row
    let read_rows = column_store.read("updated_employee");
    assert!(read_rows.is_some());
    let read_row = read_rows.unwrap().get(0).cloned();
    assert!(read_row.is_none());
}