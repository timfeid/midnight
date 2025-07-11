use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, fmt};

use crate::workflow::service::WorkflowResource;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkflowTopicMessage {
    Created {
        workflow: WorkflowResource,
    },
    Updated {
        workflow: WorkflowResource,
    },
    ServerActionRequest {
        id: String,
        workflow: WorkflowResource,
        action_id: String,
    },
}

#[derive(Debug, Clone)]
pub enum KafkaTopic {
    Workflows,
}

impl KafkaTopic {
    pub fn topic_name(&self) -> &'static str {
        match self {
            KafkaTopic::Workflows => "workflows",
        }
    }
}

impl fmt::Display for KafkaTopic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.topic_name())
    }
}
