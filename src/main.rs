use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use std::fs;
use std::env;

#[derive(Serialize, Deserialize, Debug)]
struct Label {
    name: String,
    value: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct LabelSchema {
    labels: HashMap<String, Label>,
}

#[derive(Serialize, Deserialize, Debug)]
struct SystemCatalog {
    schemas: HashMap<String, LabelSchema>,
}

fn main() {
    let mut catalog = SystemCatalog {
        schemas: HashMap::new(),
    };

    let mut schema = LabelSchema {
        labels: HashMap::new(),
    };

    // Adding a new label dynamically
    schema.labels.insert("label1".to_string(), Label {
        name: "label1".to_string(),
        value: "value1".to_string(),
    });

    // Adding the schema to the system catalog
    catalog.schemas.insert("schema1".to_string(), schema);

    // Serializing the catalog to a file
    let serialized_catalog = serde_json::to_string(&catalog).unwrap();
    fs::write("catalog.json", serialized_catalog).unwrap();

    println!("Catalog saved to catalog.json");

    // Printing the current working directory
    let current_dir = env::current_dir().unwrap();
    println!("Current working directory: {:?}", current_dir);
}