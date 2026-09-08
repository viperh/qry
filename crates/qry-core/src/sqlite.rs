use crate::shared::{Database, FileConfig, QueryResult};

#[derive(Debug, Clone)]
pub struct Sqlite {
    config: FileConfig,
}


impl Sqlite {
    pub fn new(config: &FileConfig) -> Self {
        Self { config: config.clone() }
    }
}

impl Database for Sqlite {
    fn connect(&self) -> color_eyre::Result<()> {
        todo!()
    }
    fn execute(&self, query: &str) -> color_eyre::Result<QueryResult> {
        todo!()
    }
}
