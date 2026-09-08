use crate::shared::{ConnConfig, Database, QueryResult};


#[derive(Debug, Clone)]
pub struct MySql {
    pub config: ConnConfig,
}


impl MySql {
    pub fn new(config: &ConnConfig) -> Self {
        Self { config: config.clone() }
    }
}

impl Database for MySql {
    fn connect(&self) -> color_eyre::Result<()> {
        todo!()
    }
    fn execute(&self, query: &str) -> color_eyre::Result<QueryResult> {
        todo!()
    }
}
