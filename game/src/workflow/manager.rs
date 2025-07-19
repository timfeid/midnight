use futures::future::BoxFuture;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::Mutex;

use crate::workflow::WorkflowPredicate;

use super::server_action::{ServerActionContext, ServerActionHandler, ServerActionResult};
use super::service::WorkflowResource;
use super::{
    ActionType, CreateWorkflowDefinition, NodeCondition, UserWorkflowPreferences,
    WorkflowDefinition, WorkflowNode, WorkflowState,
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

#[derive(Debug, Clone)]
pub enum WorkflowEvent {
    WorkflowStarted { resource: WorkflowResource },
    WorkflowUpdated { resource: WorkflowResource },
}

#[derive(Debug)]
pub enum ActionProcessResult {
    ShowNode {
        node_id: String,
        // title: String,
        // description: Option<String>,
        // inputs: Vec<super::WorkflowInput>,
        // actions: Vec<super::WorkflowAction>,
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

pub struct EventManager {
    // Add new fields for managing events as needed
    callbacks:
        HashMap<String, Vec<Box<dyn Fn(WorkflowEvent) -> BoxFuture<'static, ()> + Send + Sync>>>,
}

impl EventManager {
    pub fn new() -> Self {
        EventManager {
            callbacks: HashMap::new(), // Initialize fields
        }
    }

    pub fn on_event(
        &mut self,
        callback: Box<dyn Fn(WorkflowEvent) -> BoxFuture<'static, ()> + Send + Sync>,
    ) -> String {
        let id = ulid::Ulid::new().to_string();
        self.callbacks.entry(id.clone()).or_default().push(callback);
        id
    }

    fn emit_event(&self, workflow_event: WorkflowEvent) {
        for callbacks in self.callbacks.values() {
            for cb in callbacks {
                tokio::spawn(cb(workflow_event.clone()));
            }
        }
    }

    pub fn workflow_started(&self, resource: WorkflowResource) {
        let event = WorkflowEvent::WorkflowStarted { resource };
        self.emit_event(event);
    }

    pub fn workflow_updated(&self, resource: WorkflowResource) {
        let event = WorkflowEvent::WorkflowUpdated { resource };
        self.emit_event(event);
    }
}

pub struct WorkflowManager {
    pub(crate) workflows: Arc<Mutex<HashMap<String, WorkflowDefinition>>>,
    pub(crate) active_workflows: Arc<Mutex<HashMap<String, WorkflowState>>>,
    user_preferences: Arc<Mutex<HashMap<(String, String), UserWorkflowPreferences>>>,
    server_action_handlers: Arc<Mutex<HashMap<String, ServerActionHandler>>>,
    waiting_for_response: Arc<Mutex<HashMap<String, (String, Option<String>)>>>,
    waiting_for_predicate: Arc<Mutex<HashMap<String, (WorkflowPredicate, Option<String>)>>>,
    pub(crate) external_server_actions: Arc<Mutex<HashSet<(String, String)>>>,
    pub event_manager: Arc<Mutex<EventManager>>, // Add event manager to WorkflowManager
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

    pub fn new() -> Self {
        WorkflowManager {
            workflows: Arc::new(Mutex::new(HashMap::new())),
            active_workflows: Arc::new(Mutex::new(HashMap::new())),
            user_preferences: Arc::new(Mutex::new(HashMap::new())),
            server_action_handlers: Arc::new(Mutex::new(HashMap::new())),
            external_server_actions: Arc::new(Mutex::new(HashSet::new())),
            waiting_for_response: Arc::new(Mutex::new(HashMap::new())),
            waiting_for_predicate: Arc::new(Mutex::new(HashMap::new())),
            event_manager: Arc::new(Mutex::new(EventManager::new())),
        }
    }

    pub async fn check_for_waiting(&self, instance_id: &str) {
        let resource = self
            .get_workflow_resource(instance_id)
            .await
            .expect("Not found??");

        if !resource.completed {
            return;
        }
        println!("checking for waiting within {:?}", resource);

        let response = self.waiting_for_response.lock().await.remove(instance_id);
        println!("Response: {:?}", response);
        if let Some((waiting_instance_id, input_key)) = response {
            {
                let resource = self
                    .get_workflow_resource(instance_id)
                    .await
                    .expect("Not found??");
                let state = {
                    if let Some(state) = self
                        .active_workflows
                        .lock()
                        .await
                        .get_mut(&waiting_instance_id)
                    {
                        state.waiting = false;

                        if let Some(key) = input_key {
                            let resource_value = serde_json::to_value(&resource.responses)
                                .expect("Failed to serialize WorkflowResource");
                            state.responses.insert(key, resource_value);
                        }

                        let workflow_definition = {
                            let wf = self.workflows.lock().await;
                            wf.get(&state.workflow_id)
                                .expect("Unable to find workflow definition")
                                .clone()
                        };
                        let current_node = workflow_definition
                            .nodes
                            .get(&state.current_node_id)
                            .expect("what");
                        let valid_child = self
                            .find_valid_child_node(&workflow_definition, current_node, &state)
                            .expect("No next child found");
                        state.node_history.push(state.current_node_id.clone());
                        state.current_node_id = valid_child.id.clone();
                        state.updated_at = chrono::Utc::now();

                        Some(state.clone())
                    } else {
                        None
                    }
                };

                if let Some(state) = state {
                    self.update_state(&waiting_instance_id, state)
                        .await
                        .expect("unable to update state");
                }
            }

            let resource = self
                .get_workflow_resource(&waiting_instance_id)
                .await
                .expect("Not found??");
            self.event_manager.lock().await.workflow_updated(resource);
            println!("Refreshed {waiting_instance_id}");
        }

        let response = self
            .waiting_for_predicate
            .lock()
            .await
            .clone()
            .into_iter()
            .find(|(key, (predicate, response_key))| match predicate {
                WorkflowPredicate::ByUserId(user_id) => &resource.user_id == user_id,
            });

        println!("found predicate? {:?}", response);

        if let Some((waiting_instance_id, (_, input_key))) = response {
            let state = {
                if let Some(state) = self
                    .active_workflows
                    .lock()
                    .await
                    .get_mut(&waiting_instance_id)
                {
                    state.waiting = false;

                    if let Some(key) = input_key {
                        let resource_value = serde_json::to_value(&resource.responses)
                            .expect("Failed to serialize WorkflowResource");
                        state.responses.insert(key, resource_value);
                    }

                    let workflow_definition = {
                        let wf = self.workflows.lock().await;
                        wf.get(&state.workflow_id)
                            .expect("Unable to find workflow definition")
                            .clone()
                    };
                    let current_node = workflow_definition
                        .nodes
                        .get(&state.current_node_id)
                        .expect("what");
                    let valid_child = self
                        .find_valid_child_node(&workflow_definition, current_node, &state)
                        .expect("No next child found");
                    state.node_history.push(state.current_node_id.clone());
                    state.current_node_id = valid_child.id.clone();
                    state.updated_at = chrono::Utc::now();

                    Some(state.clone())
                } else {
                    None
                }
            };
            if let Some(state) = state {
                self.update_state(&waiting_instance_id, state)
                    .await
                    .expect("unable to update state");
            }
            let resource = self
                .get_workflow_resource(&waiting_instance_id)
                .await
                .expect("Not found??");
            self.event_manager.lock().await.workflow_updated(resource);
            println!("Refreshed {waiting_instance_id}");
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
        println!("Registered workflow definition with ID {}", final_id);

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

    async fn update_state(
        &self,
        instance_id: &str,
        state: WorkflowState,
    ) -> Result<(), WorkflowError> {
        let mut workflows = self.active_workflows.lock().await;
        if let Some(old_state) = workflows.get_mut(instance_id) {
            *old_state = state.clone();
            Ok(())
        } else {
            Err(WorkflowError::WorkflowNotFound)
        }
    }

    pub async fn show_node(
        &self,
        instance_id: &str,
        target_node_id: &str,
    ) -> Result<(), WorkflowError> {
        let state = &mut {
            let mut active_workflows = self.active_workflows.lock().await;
            active_workflows
                .get_mut(instance_id)
                .ok_or(WorkflowError::WorkflowInstanceNotFound)?
                .clone()
        };

        let definition = self
            .workflows
            .lock()
            .await
            .get(&state.workflow_id)
            .ok_or(WorkflowError::NodeNotFound)?
            .clone();

        if definition.nodes.get(target_node_id).is_some() {
            state.current_node_id = target_node_id.to_string();
        } else {
            return Err(WorkflowError::ServerActionFailed(format!(
                "no node found: {target_node_id}"
            )));
        }

        self.update_state(instance_id, state.clone()).await?;

        self.event_manager.lock().await.workflow_updated(
            self.get_workflow_resource(instance_id)
                .await
                .ok_or(WorkflowError::WorkflowNotFound)?,
        );

        Ok(())
    }

    pub async fn start_workflow(
        &self,
        workflow_id: &str,
        user_id: &str,
        inputs: HashMap<String, serde_json::Value>,
    ) -> Result<String, WorkflowError> {
        let state = {
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
                waiting: false,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            };

            state
        };

        let instance_id = state.instance_id.clone();
        {
            let mut active_workflows = self.active_workflows.lock().await;
            active_workflows.insert(instance_id.clone(), state);
        }

        println!("workflow started");
        self.event_manager.lock().await.workflow_started(
            self.get_workflow_resource(&instance_id)
                .await
                .ok_or(WorkflowError::WorkflowNotFound)?,
        );

        Ok(instance_id)
    }

    pub async fn process_action(
        &self,
        instance_id: String,
        action_id: &str,
        inputs: HashMap<String, serde_json::Value>,
    ) -> Result<ActionProcessResult, WorkflowError> {
        let state = &mut {
            let mut active_workflows = self.active_workflows.lock().await;
            active_workflows
                .get_mut(&instance_id)
                .ok_or(WorkflowError::WorkflowInstanceNotFound)?
                .clone()
        };

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

        let response = match action.action_type {
            ActionType::NextNode => {
                if let Some(target_node_id) = &action.target {
                    if let Some(target_node) = workflow.nodes.get(target_node_id) {
                        state.node_history.push(state.current_node_id.clone());
                        state.current_node_id = target_node_id.clone();
                        state.updated_at = chrono::Utc::now();

                        Ok(ActionProcessResult::ShowNode {
                            node_id: target_node.id.clone(),
                        })
                    } else {
                        println!("lookn hre");
                        Err(WorkflowError::NodeNotFound)
                    }
                } else {
                    // Find first valid child node
                    let valid_child =
                        self.find_valid_child_node(&workflow, current_node, &state)?;
                    state.node_history.push(state.current_node_id.clone());
                    state.current_node_id = valid_child.id.clone();
                    state.updated_at = chrono::Utc::now();

                    Ok(ActionProcessResult::ShowNode {
                        node_id: valid_child.id.clone(),
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
                        node_id: previous_node.id.clone(),
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
        }?;

        self.update_state(&instance_id, state.clone()).await?;

        Ok(response)
    }

    fn evaluate_node_condition(&self, node: &WorkflowNode, state: &WorkflowState) -> bool {
        match &node.condition {
            Some(NodeCondition::ResponseExists(field)) => {
                Self::get_nested_value(&state.responses, field).is_some()
            }

            Some(NodeCondition::ResponseEquals { field, value }) => {
                match Self::get_nested_value(&state.responses, field) {
                    Some(response_value) => response_value == value,
                    None => {
                        println!("{field} not found within");
                        false
                    }
                }
            }

            Some(NodeCondition::ResponseListNotEmpty(field)) => {
                match Self::get_nested_value(&state.responses, field) {
                    Some(serde_json::Value::Array(arr)) => !arr.is_empty(),
                    _ => false,
                }
            }

            Some(NodeCondition::Always) | None => true,
        }
    }

    // pub async fn state_to_resource(&self, state: &WorkflowState) -> Option<WorkflowResource> {
    // }

    pub async fn get_workflow_resource(&self, instance_id: &str) -> Option<WorkflowResource> {
        let state = {
            let active_workflows = self.active_workflows.lock().await;
            active_workflows.get(instance_id)?.clone()
        };
        let workflow_id = state.workflow_id.clone();
        let current_node_id = state.current_node_id.clone();

        let (current_node, completed) = {
            let workflows = self.workflows.lock().await;
            let workflow_def = workflows.get(&workflow_id)?;
            let current_node = workflow_def.nodes.get(&current_node_id)?;
            let completed = if state.completed {
                true
            } else {
                current_node.actions.is_empty()
            };
            (current_node.clone(), completed)
        };

        Some(WorkflowResource {
            instance_id: state.instance_id.clone(),
            current_node_id: current_node.id.clone(),
            workflow_id: state.workflow_id.clone(),
            name: current_node.title.clone(),
            description: current_node.description.clone(),
            responses: state.responses.clone(),
            completed,
            complete_message: state.complete_message.clone(),
            inputs: current_node.inputs.clone(),
            actions: current_node.actions.clone(),
            displays: current_node.displays.clone(),
            layout: current_node.layout.clone(),
            user_id: state.user_id.clone(),
            waiting: state.waiting.clone(),
        })
    }

    pub async fn list_user_workflow_resources(&self, user_id: &str) -> Vec<WorkflowResource> {
        let active_workflows = self.active_workflows.lock().await;

        let mut resources = Vec::new();
        for state in active_workflows.values() {
            if state.user_id == user_id && !state.completed {
                if let Some(resource) = self.get_workflow_resource(&state.instance_id).await {
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
    ) -> Result<bool, WorkflowError> {
        let mut send_refresh = false;
        match result {
            ServerActionResult::WaitForPredicate {
                inject_response_as,
                predicate,
                on_complete,
            } => {
                println!(
                    "going to wait for {:?} before continuing with workflow {workflow_id}",
                    predicate
                );
                state.waiting = true;
                self.waiting_for_predicate.lock().await.insert(
                    workflow_id.to_string(),
                    (predicate.clone(), inject_response_as.clone()),
                );
                send_refresh = true;
            }
            ServerActionResult::StartAndWaitWorkflow {
                inject_response_as,
                inputs,
                definition_id: workflow_definition_id,
                on_complete,
            } => {
                match self
                    .start_workflow(workflow_definition_id, &state.user_id, inputs.clone())
                    .await
                {
                    Ok(started_workflow_id) => {
                        println!(
                            "going to wait for {started_workflow_id} to finish before continuing with workflow {workflow_id}"
                        );
                        state.waiting = true;
                        self.waiting_for_response.lock().await.insert(
                            started_workflow_id.to_string(),
                            (workflow_id.to_string(), inject_response_as.clone()),
                        );
                        send_refresh = true;
                    }
                    Err(e) => {
                        eprintln!("Unable to start workflow {:?}", e);
                        return Err(WorkflowError::WorkflowNotFound);
                    }
                }
            }
            ServerActionResult::NextPage { page_id } => {
                if let Some(node) = workflow_definition.nodes.get(page_id) {
                    state.node_history.push(state.current_node_id.clone());
                    state.current_node_id = page_id.clone();
                    send_refresh = true;
                } else {
                    println!("ooo");
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
                send_refresh = true;
            }
            ServerActionResult::CompleteWorkflow { message, responses } => {
                for (key, value) in responses {
                    state.responses.insert(key.clone(), value.clone());
                }
                state.complete_message = Some(message.clone());
                state.completed = true;
                send_refresh = true;
            }
            _ => {}
        }

        Ok(send_refresh)
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

        let state = &mut {
            let mut workflows = self.active_workflows.lock().await;
            workflows
                .get_mut(&instance_id)
                .ok_or(WorkflowError::WorkflowNotFound)?
                .clone()
        };
        println!("state {:?}", state);

        let context = ServerActionContext {
            action_id: action_id.to_string(),
            user_id: state.user_id.clone(),
            inputs: state.responses.clone(),
            workflow_id: state.workflow_id.clone(),
            instance_id: instance_id.clone(),
        };

        let result = handler(context)
            .await
            .map_err(|e| WorkflowError::ServerActionFailed(e.to_string()))?;

        // Process server action result

        state.updated_at = chrono::Utc::now();
        let send_refresh = self
            .process_server_action_results(&result, &workflow_definition, &instance_id, state)
            .await?;

        {
            let mut workflows = self.active_workflows.lock().await;
            if let Some(old_state) = workflows.get_mut(&instance_id) {
                *old_state = state.clone();
            } else {
                return Err(WorkflowError::WorkflowNotFound);
            }
        }

        if send_refresh {
            let resource = self
                .get_workflow_resource(&instance_id)
                .await
                .ok_or(WorkflowError::WorkflowNotFound)?;
            self.event_manager.lock().await.workflow_updated(resource);
        }

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
        println!("oh yeah we are here");

        // If no valid child found, return error
        Err(WorkflowError::NodeNotFound)
    }
}
