use serde::{ Serialize, Deserialize };
use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::Path;

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Storage {
    pub keys: HashMap<String, String>,
    pub file_path: String,
}

impl Storage {
    pub fn new(file_path: &str) -> Result<Self, Box<dyn Error>> {
        let mut key_ring = Storage {
            keys: HashMap::new(),
            file_path: file_path.to_string(),
        };
        key_ring.load_from_file()?;
        Ok(key_ring)
    }

    pub fn add_key(&mut self, room_name: String, key: String) {
        self.keys.insert(room_name, key);
    }

    pub fn get_room(&self, key: &str) -> Option<&String> {
        for (room, cur_key) in &self.keys {
            if cur_key == key {
                return Some(room);
            }
        }
        None
    }

    pub fn remove_key(&mut self, room_name: &str) {
        self.keys.remove(room_name);
    }

    pub fn save_to_file(&self) -> Result<(), Box<dyn Error>> {
        let json = serde_json::to_string_pretty(self)?;
        fs::write(self.file_path.clone(), json)?;
        Ok(())
    }

    pub fn load_from_file(&mut self) -> Result<(), Box<dyn Error>> {
        let path = Path::new(&self.file_path);
        if path.exists() {
            let data = fs::read_to_string(self.file_path.clone())?;
            let storage: Storage = serde_json::from_str(&data)?;
            self.keys = storage.keys;
            self.file_path = storage.file_path;
        }
        Ok(())
    }
}
