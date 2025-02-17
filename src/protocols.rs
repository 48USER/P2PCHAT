use serde::{ Serialize, Deserialize };

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileOrMenuRequest {
    pub is_menu: bool,
    pub file_name: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileOrMenuResponse {
    pub is_menu: bool,
    pub menu: Vec<String>,
    pub file_name: String,
    pub file_data: Vec<u8>,
}
