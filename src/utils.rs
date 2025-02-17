use std::fs;
use std::io;
use std::io::Error as IoError;
use std::path::Path;
use colored::*;

pub fn errprintln(message: &str) {
    println!("{}", format!("/!/ {}", message).red());
}

pub fn sysprintln(message: &str) {
    println!("{}", format!("/~/ {}", message).cyan());
}

pub fn read_file_as_bytes(file_path: &str) -> Result<Vec<u8>, IoError> {
    let content = fs::read(file_path)?;
    Ok(content)
}

pub fn save_bytes_to_file(file_path: &str, data: Vec<u8>) -> Result<(), io::Error> {
    fs::write(file_path, data)
}

pub fn ensure_directory_exists(dir_path: &str) -> Result<(), IoError> {
    let path = Path::new(dir_path);
    if !path.exists() {
        fs::create_dir_all(path)?;
        sysprintln(&format!("Public directory is now located here: {}", dir_path));
    } else if !path.is_dir() {
        errprintln(&format!("Path exists, but it's not a directory: {}", dir_path));
        return Err(IoError::new(io::ErrorKind::Other, "Path exists, but it's not a directory"));
    }
    Ok(())
}

pub fn list_files_in_directory(dir_path: &str) -> io::Result<Vec<String>> {
    let path = Path::new(dir_path);
    let mut file_names = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let entry_path = entry.path();
        if entry_path.is_file() {
            if let Some(file_name) = entry_path.file_name().and_then(|name| name.to_str()) {
                file_names.push(file_name.to_string());
            }
        }
    }
    Ok(file_names)
}
