pub struct Database {
    data: Vec<u8>,
}

impl Database {
    pub fn new() -> Self {
        Database { data: Vec::new() }
    }

    pub fn insert(&mut self, value: u8) {
        self.data.push(value);
    }

    pub fn get(&self, index: usize) -> Option<u8> {
        self.data.get(index).cloned()
    }
}