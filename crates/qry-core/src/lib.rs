use std::sync::{Arc, Mutex};

use shared::{Database, ConnConfig, FileConfig, ConfigType, QueryResult};


use postgres::Postgres;
use sqlite::Sqlite;
use mysql::MySql;
use oracle::Oracle;


pub mod shared;
pub mod postgres;
pub mod sqlite;
pub mod mysql;
pub mod oracle;


#[derive(Debug, Clone)]
pub enum DatabaseType {
    Sqlite(FileConfig),
    Postgres(ConnConfig),
    Mysql(ConnConfig),
    Oracle(ConnConfig),
}

#[derive(Debug, Clone)]
pub struct Driver{
    db_type: Option<DatabaseType>,
    db: Option<Arc<Mutex<dyn Database + Send>>>,
}

impl Driver {
    pub fn new() -> Self {
        Self {
            db_type: None,
            db: None,
        }
    }
    pub fn connect(dbtype: &DatabaseType, cfg: ConfigType) -> Self {
        Self {
            db_type: Some(dbtype.clone()),
            db: match dbtype {
                DatabaseType::Sqlite(cfg) => Some(Arc::new(Mutex::new(Sqlite::new(cfg)))),
                DatabaseType::Postgres(cfg) => Some(Arc::new(Mutex::new(Postgres::new(cfg)))),
                DatabaseType::Mysql(cfg) => Some(Arc::new(Mutex::new(MySql::new(cfg)))),
                DatabaseType::Oracle(cfg) => Some(Arc::new(Mutex::new(Oracle::new(cfg)))),
            }
        }
    }

    pub fn query(&self, query: &str) -> Result<QueryResult, String> {
        if let Some(db) = &self.db {
            let results = db.lock().unwrap().execute(query);
            match results {
                Ok(results) => {
                    Ok(results)
                }
                Err(e) => {
                    Err(e.to_string())
                }
            }
        } else {
            Err("No database connected".to_string())
        }
    }
}
