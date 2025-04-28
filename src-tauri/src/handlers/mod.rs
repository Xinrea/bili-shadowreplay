pub mod account;
pub mod config;
pub mod macros;
pub mod message;
pub mod recorder;
pub mod utils;
pub mod video;

use crate::database::account::AccountRow;

#[derive(serde::Serialize)]
pub struct AccountInfo {
    pub accounts: Vec<AccountRow>,
}

#[derive(serde::Serialize)]
pub struct DiskInfo {
    pub disk: String,
    pub total: u64,
    pub free: u64,
}
