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

impl ServerActionContext {
    fn get_nested_value<'a>(
        map: &'a HashMap<String, serde_json::Value>,
        dotted_path: &str,
    ) -> Option<&'a serde_json::Value> {
        let mut current = map.get(dotted_path.split('.').next()?)?;

        for key in dotted_path.split('.').skip(1) {
            match current {
                serde_json::Value::Object(obj) => {
                    current = obj.get(key)?;
                }
                _ => return None,
            }
        }

        Some(current)
    }
    pub fn get_input(&self, path: &str) -> Option<&Value> {
        Self::get_nested_value(&self.inputs, path)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub enum ServerActionResult {
    NextPage {
        page_id: String,
    },
    UpdateResponses(HashMap<String, Value>),
    CompleteWorkflow {
        responses: HashMap<String, Value>,
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
