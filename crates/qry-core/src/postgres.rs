use crate::shared::{ConnConfig, Database, QueryResult};


#[derive(Debug, Clone)]
pub struct Postgres {
    config: ConnConfig
}

impl Postgres {
    pub fn new(config: &ConnConfig) -> Self {
        Self { config: config.clone() }
    }
}

impl Database for Postgres {
    fn connect(&self) -> color_eyre::Result<()> {
        todo!()
    }
    fn execute(&self, query: &str) -> color_eyre::Result<QueryResult> {
        todo!()
    }
}
