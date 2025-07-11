use std::collections::HashMap;

use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

#[derive(Debug, Clone)]
pub struct ServerActionContext {
    pub workflow_id: String,
    pub action_id: String,
    pub instance_id: String,
    pub user_id: String,
    pub inputs: HashMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub enum ServerActionResult {
    NextPage {
        page_id: String,
    },
    UpdateResponses(HashMap<String, Value>),
    CompleteWorkflow {
        message: String,
    },
    StartNewWorkflow {
        workflow_id: String,
        inputs: HashMap<String, Value>,
    },
    CancelWorkflow,
}

pub type ServerActionHandler = Box<
    dyn Fn(
            ServerActionContext,
        ) -> BoxFuture<
            'static,
            Result<ServerActionResult, Box<dyn std::error::Error + Send + Sync>>,
        > + Send
        + Sync,
>;
