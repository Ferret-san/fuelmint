use fuel_core::{database::Database, executor::Executor as FuelExecutor, service::Config};
use std::{fmt, fmt::Debug};
/// Create a wrapper around fuel's Executor
pub struct Executor {
    pub producer: FuelExecutor,
}

impl Executor {
    pub fn new(producer: FuelExecutor) -> Self {
        Self { producer: producer }
    }
}

impl Debug for Executor {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Executor")
            .field("database", &self.producer.database)
            .field("config", &self.producer.config)
            .finish()
    }
}

impl Clone for Executor {
    fn clone(&self) -> Executor {
        Executor {
            producer: FuelExecutor {
                database: self.producer.database.clone(),
                config: self.producer.config.clone(),
            },
        }
    }
}

impl Default for Executor {
    fn default() -> Self {
        Executor::new(FuelExecutor {
            database: Database::default(),
            config: Config::local_node(),
        })
    }
}

impl From<FuelExecutor> for Executor {
    fn from(producer: FuelExecutor) -> Self {
        Executor::new(producer)
    }
}
