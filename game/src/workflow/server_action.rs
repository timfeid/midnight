use std::collections::HashMap;

use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use crate::{
    error::{AppResult, ServicesError},
    workflow::{ActionType, WorkflowPredicate},
};

#[derive(Debug, Clone)]
pub struct ServerActionContext {
    pub workflow_id: String,
    pub action_id: String,
    pub instance_id: String,
    pub user_id: String,
    pub inputs: HashMap<String, Value>,
}

impl ServerActionContext {
    pub fn get_required_nested_value_as_str<'a>(
        map: &'a HashMap<String, serde_json::Value>,
        dotted_path: &str,
    ) -> AppResult<&'a str> {
        let value = Self::get_nested_value(map, dotted_path);

        return value
            .and_then(|x| x.as_str())
            .ok_or(ServicesError::InternalError(format!(
                "{dotted_path} not found in {:?}",
                map
            )));
    }

    pub fn get_nested_value<'a>(
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

    pub fn get_required_input_as_str(&self, key: &str) -> AppResult<&str> {
        self.get_input(key)
            .and_then(|v| v.as_str())
            .ok_or_else(|| ServicesError::InternalError(format!("Missing or invalid input: {key}")))
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
    StartAndWaitWorkflow {
        inputs: HashMap<String, Value>,
        definition_id: String,
        inject_workflow_as: Option<String>,
        on_complete: Option<ActionType>,
    },
    WaitForPredicate {
        predicate: WorkflowPredicate,
        inject_workflow_as: Option<String>,
        on_complete: Option<ActionType>,
    },
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
