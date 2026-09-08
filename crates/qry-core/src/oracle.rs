use crate::shared::{ConnConfig, Database, QueryResult};

#[derive(Debug, Clone)]
pub struct Oracle {
   config: ConnConfig
}

impl Oracle {
    pub fn new(config: &ConnConfig) -> Self {
        Self { config: config.clone() }
    }
}

impl Database for Oracle {
    fn connect(&self) -> color_eyre::Result<()> {
        todo!()
    }
    fn execute(&self, query: &str) -> color_eyre::Result<QueryResult> {
        todo!()
    }
}
