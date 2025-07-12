use futures::future::BoxFuture;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::Mutex;

use super::server_action::{ServerActionContext, ServerActionHandler, ServerActionResult};
use super::service::{WorkflowPlugin, WorkflowResource};
use super::{
    ActionType, CreateWorkflowDefinition, NodeCondition, ServerActionDefinition,
    UserWorkflowPreferences, WorkflowAction, WorkflowDefinition, WorkflowNode, WorkflowState,
};

#[derive(Debug, Error)]
pub enum WorkflowError {
    #[error("Workflow not found")]
    WorkflowNotFound,

    #[error("Workflow instance not found")]
    WorkflowInstanceNotFound,

    #[error("Node not found")]
    NodeNotFound,

    #[error("Action not found")]
    ActionNotFound,

    #[error("Server action not found")]
    ServerActionNotFound,

    #[error("Server action failed: {0}")]
    ServerActionFailed(String),

    #[error("Workflow already completed")]
    WorkflowAlreadyCompleted,

    #[error("Invalid state")]
    InvalidState,
}

#[derive(Debug)]
pub enum ActionProcessResult {
    ShowNode {
        id: String,
        title: String,
        description: Option<String>,
        inputs: Vec<super::WorkflowInput>,
        actions: Vec<super::WorkflowAction>,
    },
    ExternalServerActionStarted {
        id: String,
        workflow_id: String,
        action_id: String,
    },
    ServerActionStarted {
        workflow_id: String,
        action_id: String,
    },
    WorkflowCompleted(HashMap<String, serde_json::Value>),
    WorkflowCancelled,
    StartNewWorkflow {
        workflow_id: String,
        user_id: String,
    },
}

#[derive(Clone)]
pub struct WorkflowManager {
    pub(crate) workflows: Arc<Mutex<HashMap<String, WorkflowDefinition>>>,
    pub(crate) active_workflows: Arc<Mutex<HashMap<String, WorkflowState>>>,
    user_preferences: Arc<Mutex<HashMap<(String, String), UserWorkflowPreferences>>>,
    server_action_handlers: Arc<Mutex<HashMap<String, ServerActionHandler>>>,
    pub(crate) external_server_actions: Arc<Mutex<HashSet<(String, String)>>>,
}

impl std::fmt::Debug for WorkflowManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkflowManager")
            .field("workflows", &self.workflows)
            .field("active_workflows", &self.active_workflows)
            .field("user_preferences", &self.user_preferences)
            .field("external_server_actions", &self.external_server_actions)
            .finish()
    }
}

impl WorkflowManager {
    pub fn new() -> Self {
        WorkflowManager {
            workflows: Arc::new(Mutex::new(HashMap::new())),
            active_workflows: Arc::new(Mutex::new(HashMap::new())),
            user_preferences: Arc::new(Mutex::new(HashMap::new())),
            server_action_handlers: Arc::new(Mutex::new(HashMap::new())),
            external_server_actions: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub async fn process_external_server_action(
        &self,
        instance_id: String,
        action_id: &str,
    ) -> Result<(String, String), WorkflowError> {
        let mut active_workflows = self.active_workflows.lock().await;
        let state = active_workflows
            .get_mut(&instance_id)
            .ok_or(WorkflowError::WorkflowInstanceNotFound)?;

        // Generate a token for this action request
        let token = ulid::Ulid::new().to_string();

        Ok((token, state.user_id.clone()))
    }

    pub async fn register_workflow_definition(
        &self,
        user_id: &str,
        workflow: CreateWorkflowDefinition,
    ) -> Result<String, WorkflowError> {
        {
            let server_action_handlers = self.server_action_handlers.lock().await;
            let external_server_actions = self.external_server_actions.lock().await;

            for action in workflow.server_actions.values() {
                if !server_action_handlers.contains_key(&action.id)
                    && !external_server_actions.contains(&(user_id.to_string(), action.id.clone()))
                {
                    return Err(WorkflowError::ServerActionFailed(format!(
                        "Server action '{}' not registered.",
                        action.id
                    )));
                }
            }
        }

        let mut workflows = self.workflows.lock().await;
        let definition = WorkflowDefinition {
            id: workflow.id.clone(),
            responses: workflow.responses.clone(),
            owner_id: Some(user_id.to_string()),
            name: workflow.name.clone(),
            description: workflow.description.clone(),
            initial_node_id: workflow.initial_node_id.clone(),
            nodes: workflow.nodes.clone(),
            server_actions: workflow.server_actions.clone(),
        };

        let final_id = format!("user-{}-wf-{}", user_id, workflow.id);
        if let Some(workflow) = workflows.get_mut(&final_id) {
            if workflow.owner_id == Some(user_id.to_string()) {
                *workflow = definition;
            } else {
                return Err(WorkflowError::ServerActionFailed(
                    "You are not the owner of that workflow.".to_string(),
                ));
            }
        } else {
            workflows.insert(final_id.clone(), definition);
        }

        Ok(final_id)
    }

    pub async fn register_external_server_action(
        &self,
        user_id: &str,
        action_id: &str,
    ) -> Result<String, WorkflowError> {
        let mut external_server_actions = self.external_server_actions.lock().await;

        let final_id = format!("user-{}-sa-{}", user_id, action_id);
        external_server_actions.insert((user_id.to_string(), final_id.clone()));

        Ok(final_id)
    }

    pub async fn register_server_action(
        &self,
        action_id: &str,
        handler: ServerActionHandler,
    ) -> Result<(), WorkflowError> {
        self.server_action_handlers
            .lock()
            .await
            .insert(action_id.to_string(), handler);
        Ok(())
    }

    pub async fn start_workflow(
        &self,
        workflow_id: &str,
        user_id: &str,
        inputs: HashMap<String, serde_json::Value>,
    ) -> Result<String, WorkflowError> {
        let hash_map = self.workflows.lock().await;
        let workflow = hash_map
            .get(workflow_id)
            .ok_or(WorkflowError::WorkflowNotFound)?;

        let mut responses = workflow.responses.clone();
        responses.extend(inputs);

        let instance_id = ulid::Ulid::new().to_string();
        let state = WorkflowState {
            workflow_id: workflow_id.to_string(),
            instance_id: instance_id.clone(),
            user_id: user_id.to_string(),
            current_node_id: workflow.initial_node_id.clone(),
            node_history: Vec::new(),
            responses,
            message_id: None,
            complete_message: None,
            completed: false,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let mut active_workflows = self.active_workflows.lock().await;
        active_workflows.insert(instance_id.clone(), state);

        Ok(instance_id)
    }

    pub async fn process_action(
        &self,
        instance_id: String,
        action_id: &str,
        inputs: HashMap<String, serde_json::Value>,
    ) -> Result<ActionProcessResult, WorkflowError> {
        let mut active_workflows = self.active_workflows.lock().await;
        let state = active_workflows
            .get_mut(&instance_id)
            .ok_or(WorkflowError::WorkflowInstanceNotFound)?;

        if state.completed {
            return Err(WorkflowError::WorkflowAlreadyCompleted);
        }

        let workflow = {
            let workflows = self.workflows.lock().await;
            workflows
                .get(&state.workflow_id)
                .ok_or(WorkflowError::WorkflowNotFound)?
                .clone()
        };

        let current_node = workflow
            .nodes
            .get(&state.current_node_id)
            .ok_or(WorkflowError::NodeNotFound)?;

        let action = current_node
            .actions
            .iter()
            .find(|a| a.id == action_id)
            .ok_or(WorkflowError::ActionNotFound)?;

        // Save inputs to state
        for (key, value) in inputs {
            state.responses.insert(key, value);
        }

        match action.action_type {
            ActionType::NextNode => {
                if let Some(target_node_id) = &action.target {
                    if let Some(target_node) = workflow.nodes.get(target_node_id) {
                        state.node_history.push(state.current_node_id.clone());
                        state.current_node_id = target_node_id.clone();
                        state.updated_at = chrono::Utc::now();

                        Ok(ActionProcessResult::ShowNode {
                            id: target_node.id.clone(),
                            title: target_node.title.clone(),
                            description: target_node.description.clone(),
                            inputs: target_node.inputs.clone(),
                            actions: target_node.actions.clone(),
                        })
                    } else {
                        Err(WorkflowError::NodeNotFound)
                    }
                } else {
                    // Find first valid child node
                    let valid_child = self.find_valid_child_node(&workflow, current_node, state)?;
                    state.node_history.push(state.current_node_id.clone());
                    state.current_node_id = valid_child.id.clone();
                    state.updated_at = chrono::Utc::now();

                    Ok(ActionProcessResult::ShowNode {
                        id: valid_child.id.clone(),
                        title: valid_child.title.clone(),
                        description: valid_child.description.clone(),
                        inputs: valid_child.inputs.clone(),
                        actions: valid_child.actions.clone(),
                    })
                }
            }
            ActionType::PreviousNode => {
                if let Some(previous_node_id) = state.node_history.pop() {
                    let previous_node = workflow
                        .nodes
                        .get(&previous_node_id)
                        .ok_or(WorkflowError::NodeNotFound)?;

                    state.current_node_id = previous_node_id;
                    state.updated_at = chrono::Utc::now();

                    Ok(ActionProcessResult::ShowNode {
                        id: previous_node.id.clone(),
                        title: previous_node.title.clone(),
                        description: previous_node.description.clone(),
                        inputs: previous_node.inputs.clone(),
                        actions: previous_node.actions.clone(),
                    })
                } else {
                    // No previous node, this is an error
                    Err(WorkflowError::InvalidState)
                }
            }
            ActionType::Submit => {
                state.completed = true;
                state.updated_at = chrono::Utc::now();
                Ok(ActionProcessResult::WorkflowCompleted(
                    state.responses.clone(),
                ))
            }
            ActionType::Cancel => {
                state.completed = true;
                state.updated_at = chrono::Utc::now();
                Ok(ActionProcessResult::WorkflowCancelled)
            }
            ActionType::RunServerAction => {
                let state_json = serde_json::to_value(&state).map_err(|_| {
                    WorkflowError::ServerActionFailed(
                        "Failed to serialize state to JSON.".to_string(),
                    )
                })?;
                if let Some(action_id) = &action.target {
                    let external_server_actions = self.external_server_actions.lock().await;
                    let mut action_id = action_id.to_string();
                    if let Some(server_action) = workflow.server_actions.get(&action_id) {
                        action_id = server_action.id.clone();
                    }

                    if external_server_actions
                        .iter()
                        .any(|(_user_id, id)| *id == action_id)
                    {
                        let id = ulid::Ulid::new().to_string();

                        Ok(ActionProcessResult::ExternalServerActionStarted {
                            id,
                            workflow_id: state.workflow_id.clone(),
                            action_id: action_id.clone(),
                        })
                    } else if self
                        .server_action_handlers
                        .lock()
                        .await
                        .contains_key(&action_id)
                    {
                        Ok(ActionProcessResult::ServerActionStarted {
                            workflow_id: state.workflow_id.clone(),
                            action_id: action_id.clone(),
                        })
                    } else {
                        println!("looking for action id: {action_id}");
                        Err(WorkflowError::ServerActionNotFound)
                    }
                } else if workflow.server_actions.contains_key(&action.id) {
                    Ok(ActionProcessResult::ServerActionStarted {
                        workflow_id: state.workflow_id.clone(),
                        action_id: action.id.clone(),
                    })
                } else {
                    Err(WorkflowError::ServerActionNotFound)
                }
            }
            ActionType::StartWorkflow => {
                if let Some(target_workflow_id) = &action.target {
                    if self.workflows.lock().await.contains_key(target_workflow_id) {
                        Ok(ActionProcessResult::StartNewWorkflow {
                            workflow_id: target_workflow_id.clone(),
                            user_id: state.user_id.clone(),
                        })
                    } else {
                        Err(WorkflowError::WorkflowNotFound)
                    }
                } else {
                    Err(WorkflowError::WorkflowNotFound)
                }
            }
        }
    }

    fn evaluate_node_condition(&self, node: &WorkflowNode, state: &WorkflowState) -> bool {
        match &node.condition {
            Some(NodeCondition::ResponseExists(field)) => state.responses.contains_key(field),
            Some(NodeCondition::ResponseEquals { field, value }) => {
                if let Some(response_value) = state.responses.get(field) {
                    response_value == value
                } else {
                    false
                }
            }
            Some(NodeCondition::ResponseListNotEmpty(field)) => {
                if let Some(response_value) = state.responses.get(field) {
                    if let Some(array) = response_value.as_array() {
                        !array.is_empty()
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            Some(NodeCondition::Always) => true,
            None => true,
        }
    }

    pub async fn state_to_resource(&self, state: &WorkflowState) -> Option<WorkflowResource> {
        let workflows = self.workflows.lock().await;
        let workflow_def = workflows.get(&state.workflow_id)?;
        let current_node = workflow_def.nodes.get(&state.current_node_id)?;

        Some(WorkflowResource {
            instance_id: state.instance_id.clone(),
            current_node_id: state.current_node_id.clone(),
            workflow_id: state.workflow_id.clone(),
            name: current_node.title.clone(),
            description: current_node.description.clone(),
            responses: state.responses.clone(),
            completed: state.completed,
            complete_message: state.complete_message.clone(),
            inputs: current_node.inputs.clone(),
            actions: current_node.actions.clone(),
            displays: current_node.displays.clone(),
            layout: current_node.layout.clone(),
            user_id: state.user_id.clone(),
        })
    }

    pub async fn get_workflow_resource(&self, instance_id: &str) -> Option<WorkflowResource> {
        let state = {
            let active_workflows = self.active_workflows.lock().await;
            active_workflows.get(instance_id)?.clone()
        };

        self.state_to_resource(&state).await
    }

    pub async fn list_user_workflow_resources(&self, user_id: &str) -> Vec<WorkflowResource> {
        let active_workflows = self.active_workflows.lock().await;

        let mut resources = Vec::new();
        for state in active_workflows.values() {
            if state.user_id == user_id && !state.completed {
                if let Some(resource) = self.state_to_resource(&state).await {
                    resources.push(resource);
                }
            }
        }
        resources
    }

    pub async fn process_server_action_results(
        &self,
        result: &ServerActionResult,
        workflow_definition: &WorkflowDefinition,
        workflow_id: &str,
        state: &mut WorkflowState,
    ) -> Result<(), WorkflowError> {
        match result {
            ServerActionResult::NextPage { page_id } => {
                if let Some(node) = workflow_definition.nodes.get(page_id) {
                    state.node_history.push(state.current_node_id.clone());
                    state.current_node_id = page_id.clone();
                } else {
                    return Err(WorkflowError::NodeNotFound);
                }
            }
            ServerActionResult::UpdateResponses(responses) => {
                for (key, value) in responses {
                    state.responses.insert(key.clone(), value.clone());
                }

                let current_node = workflow_definition
                    .nodes
                    .get(&state.current_node_id)
                    .ok_or(WorkflowError::NodeNotFound)?;

                let valid_child =
                    self.find_valid_child_node(&workflow_definition, current_node, state)?;
                state.node_history.push(state.current_node_id.clone());
                state.current_node_id = valid_child.id.clone();
                println!("updated valid child {:?}", valid_child);
            }
            ServerActionResult::CompleteWorkflow { message } => {
                state.complete_message = Some(message.clone());
                state.completed = true;
            }
            _ => {}
        }
        Ok(())
    }

    pub async fn execute_server_action(
        &self,
        instance_id: String,
        workflow_id: &str,
        action_id: &str,
    ) -> Result<ServerActionResult, WorkflowError> {
        let handlers = self.server_action_handlers.lock().await;
        let handler = handlers
            .get(action_id)
            .ok_or(WorkflowError::ServerActionNotFound)?;

        let workflow_definition = self
            .workflows
            .lock()
            .await
            .get(workflow_id)
            .ok_or(WorkflowError::WorkflowNotFound)?
            .clone();

        let mut workflows = self.active_workflows.lock().await;
        let state = workflows
            .get_mut(&instance_id)
            .ok_or(WorkflowError::WorkflowNotFound)?;

        let context = ServerActionContext {
            action_id: action_id.to_string(),
            user_id: state.user_id.clone(),
            inputs: state.responses.clone(),
            workflow_id: state.workflow_id.clone(),
            instance_id,
        };

        let result = handler(context)
            .await
            .map_err(|e| WorkflowError::ServerActionFailed(e.to_string()))?;

        // Process server action result
        self.process_server_action_results(&result, &workflow_definition, workflow_id, state)
            .await?;

        state.updated_at = chrono::Utc::now();

        Ok(result)
    }

    pub async fn get_user_preferences(
        &self,
        user_id: &str,
        workflow_id: &str,
    ) -> Option<UserWorkflowPreferences> {
        let prefs = self.user_preferences.lock().await;
        prefs
            .get(&(user_id.to_string(), workflow_id.to_string()))
            .cloned()
    }

    pub async fn save_user_preferences(
        &self,
        user_id: &str,
        workflow_id: &str,
        responses: HashMap<String, serde_json::Value>,
    ) {
        let mut prefs = self.user_preferences.lock().await;
        let key = (user_id.to_string(), workflow_id.to_string());

        let pref = UserWorkflowPreferences {
            user_id: user_id.to_string(),
            workflow_id: workflow_id.to_string(),
            saved_responses: responses,
            updated_at: chrono::Utc::now(),
        };

        prefs.insert(key, pref);
    }

    pub async fn get_write_lock(&self) -> tokio::sync::MutexGuard<()> {
        // Add a mutex for write operations
        static WRITE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        WRITE_LOCK.lock().await
    }

    pub async fn has_workflow(&self, workflow_id: &str) -> bool {
        self.workflows.lock().await.contains_key(workflow_id)
    }

    pub async fn get_server_action_ids(&self, workflow_id: &str) -> Vec<String> {
        if let Some(workflow) = self.workflows.lock().await.get(workflow_id) {
            workflow.server_actions.keys().cloned().collect()
        } else {
            Vec::new()
        }
    }

    fn find_valid_child_node<'a>(
        &'a self,
        workflow: &'a WorkflowDefinition,
        parent_node: &'a WorkflowNode,
        state: &WorkflowState,
    ) -> Result<&'a WorkflowNode, WorkflowError> {
        for child_node in workflow.nodes.values() {
            if let Some(parent_id) = &child_node.parent_id {
                if parent_id == &parent_node.id && self.evaluate_node_condition(child_node, state) {
                    return Ok(child_node);
                }
            }
        }

        // If no valid child found, return error
        Err(WorkflowError::NodeNotFound)
    }
}
