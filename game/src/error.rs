use thiserror::Error;

use crate::workflow::manager::WorkflowError;

#[derive(Error, Debug)]
pub enum ServicesError {
    #[error("Configuration error: {0}")]
    Config(#[from] std::env::VarError),

    #[error("Something went wrong: {0}")]
    InternalError(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Workflow error: {0}")]
    WorkflowError(WorkflowError),

    #[error("SQL Error: {0}")]
    SQLError(String),
}

pub type AppResult<T> = Result<T, ServicesError>;

impl From<WorkflowError> for ServicesError {
    fn from(value: WorkflowError) -> Self {
        eprintln!("Workflow Error: {:?}", value);
        ServicesError::WorkflowError(value)
    }
}
