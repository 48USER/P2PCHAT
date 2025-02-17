mod client;
mod protocols;
mod storage;
mod utils;

use std::error::Error;
use std::io;
use client::Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut kr_file_path = String::new();
    let mut pub_dir_path = String::new();
    println!("Please enter key-ring file path:");
    io::stdin().read_line(&mut kr_file_path).unwrap();
    println!("Please enter public directory file path:");
    io::stdin().read_line(&mut pub_dir_path).unwrap();
    let client = Client::new(kr_file_path.trim(), pub_dir_path.trim());
    let _ = client.expect("").run().await;
    Ok(())
}
