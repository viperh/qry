use std::fmt::{Debug};
use serde::{Serialize, Deserialize};

pub trait Database: Debug{
    fn connect(&self) -> color_eyre::Result<()>;
    fn execute(&self, query: &str) -> color_eyre::Result<QueryResult>;
}



pub enum ConfigType {
    Conn(ConnConfig),
    File(FileConfig),
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub database: String,
}


#[derive(Debug, Clone)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileConfig {
    pub path: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub database: Option<String>,
}
