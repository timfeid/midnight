use async_trait::async_trait;
use futures::lock::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use std::{collections::HashMap, path::PathBuf, sync::Arc};

use crate::{
    error::{AppResult, ServicesError},
    kafka::service::KafkaService,
    workflow::server_action::ServerActionHandler,
};

use super::{
    CreateWorkflowDefinition, WorkflowAction, WorkflowDisplay, WorkflowInput,
    manager::{ActionProcessResult, WorkflowManager},
    server_action::ServerActionResult,
};

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct WorkflowResource {
    pub instance_id: String,
    pub workflow_id: String,
    pub name: String,
    pub description: Option<String>,
    pub responses: HashMap<String, serde_json::Value>,
    pub completed: bool,
    pub complete_message: Option<String>,
    pub inputs: Vec<WorkflowInput>,
    pub actions: Vec<WorkflowAction>,
    pub displays: Vec<WorkflowDisplay>,
    pub layout: Option<String>,
    pub user_id: String,
    pub current_node_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct WorkflowRespondServerActionArgs {
    token: String,
    result: ServerActionResult,
}
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ProcessWorkflowActionArgs {
    pub instance_id: String,
    pub action_id: String,
    pub inputs: HashMap<String, serde_json::Value>,
}

impl ProcessWorkflowActionArgs {
    pub fn new(
        instance_id: String,
        action_id: String,
        inputs: HashMap<String, serde_json::Value>,
    ) -> ProcessWorkflowActionArgs {
        ProcessWorkflowActionArgs {
            instance_id,
            action_id,
            inputs,
        }
    }
}

#[derive(Debug)]
pub struct WorkflowService {
    pub(crate) manager: Arc<WorkflowManager>,
    pub(crate) kafka: Arc<KafkaService>,
    external_action_responses:
        Arc<tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<serde_json::Value>>>>,
}

#[async_trait]
pub trait WorkflowPlugin: Send + Sync {
    async fn register(service: Arc<WorkflowService>) -> Result<Arc<Self>, ServicesError>
    where
        Self: Sized;
}

impl WorkflowService {
    pub async fn new(kafka: Arc<KafkaService>) -> Self {
        let manager = WorkflowManager::new();

        Self {
            manager: Arc::new(manager),
            kafka,
            external_action_responses: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        }
    }

    pub async fn register_server_action(
        &self,
        action_id: &str,
        action: ServerActionHandler,
    ) -> AppResult<()> {
        self.manager
            .register_server_action(action_id, action)
            .await?;

        Ok(())
    }

    pub async fn register_external_server_action(
        &self,
        user_id: &str,
        action_id: &str,
    ) -> AppResult<String> {
        let response = self
            .manager
            .register_external_server_action(user_id, action_id)
            .await?;

        Ok(response)
    }

    pub async fn register_workflow_definition(
        &self,
        user_id: &str,
        definition: CreateWorkflowDefinition,
    ) -> AppResult<String> {
        let response = self
            .manager
            .register_workflow_definition(user_id, definition)
            .await?;

        Ok(response)
    }

    pub async fn start_command_workflow(
        &self,
        workflow_id: &str,
        user_id: &str,
        inputs: HashMap<String, Value>,
    ) -> AppResult<WorkflowResource> {
        let id = self
            .manager
            .start_workflow(workflow_id, user_id, inputs)
            .await
            .map_err(ServicesError::from)?;

        let workflow = self.get_workflow_resource(&id).await?;
        self.kafka
            .workflows
            .create_workflow(workflow.clone())
            .await
            .ok();

        Ok(workflow)
    }

    pub async fn start_workflow(
        &self,
        workflow_id: &str,
        user_id: &str,
        inputs: HashMap<String, Value>,
    ) -> AppResult<WorkflowResource> {
        let id = self
            .manager
            .start_workflow(workflow_id, user_id, inputs)
            .await
            .map_err(ServicesError::from)?;

        let workflow = self.get_workflow_resource(&id).await?;
        println!("hello");
        self.kafka
            .workflows
            .create_workflow(workflow.clone())
            .await
            .ok();

        Ok(workflow)
    }

    pub async fn get_workflow_resource(&self, instance_id: &str) -> AppResult<WorkflowResource> {
        let workflows = self.manager.active_workflows.lock().await;
        let state = workflows.get(instance_id).ok_or(ServicesError::NotFound(
            "Workflow instance not found".into(),
        ))?;

        self.manager
            .state_to_resource(state)
            .await
            .ok_or(ServicesError::InternalError("Failed to create workflow resource".into()).into())
    }

    pub async fn handle_external_action_response(
        &self,
        token: &str,
        response: serde_json::Value,
    ) -> AppResult<()> {
        let tx = {
            let mut response_channels = self.external_action_responses.lock().await;
            response_channels.remove(token)
        };

        if let Some(tx) = tx {
            let _ = tx.send(response);
        }

        Ok(())
    }

    pub async fn process_action(
        &self,
        _user_id: &str,
        args: ProcessWorkflowActionArgs,
    ) -> AppResult<WorkflowResource> {
        // Check if this is an external server action

        let action = self
            .manager
            .process_action(args.instance_id.clone(), &args.action_id, args.inputs)
            .await
            .map_err(ServicesError::from)?;

        match action {
            ActionProcessResult::ExternalServerActionStarted { action_id, id, .. } => {
                let resource = self.get_workflow_resource(&args.instance_id).await?;
                self.handle_external_server_action(
                    id,
                    args.instance_id.clone(),
                    resource.clone(),
                    action_id,
                );
            }
            ActionProcessResult::StartNewWorkflow {
                workflow_id,
                user_id,
            } => {
                let workflow = self
                    .start_workflow(&workflow_id, &user_id, HashMap::new())
                    .await?;
                self.kafka.workflows.create_workflow(workflow).await.ok();
            }
            ActionProcessResult::ServerActionStarted {
                workflow_id,
                action_id,
            } => {
                self.manager
                    .execute_server_action(args.instance_id.clone(), &workflow_id, &action_id)
                    .await?;
            }
            // Handle other action types as needed
            _ => {}
        }

        let resource = self.get_workflow_resource(&args.instance_id).await?;
        self.kafka
            .workflows
            .update_workflow(resource.clone())
            .await
            .ok();

        Ok(resource)
    }

    pub async fn respond_server_action(
        &self,
        user_id: &str,
        args: WorkflowRespondServerActionArgs,
    ) -> AppResult<()> {
        // Find the channel associated with this token
        let tx = {
            let mut response_channels = self.external_action_responses.lock().await;
            response_channels.remove(&args.token)
        };

        // If we found a waiting channel, send the response
        if let Some(tx) = tx {
            // Convert the HashMap to a JSON Value
            let response_value = serde_json::to_value(args.result).map_err(|e| {
                ServicesError::InternalError(format!("JSON serialization error: {}", e))
            })?;

            // Send the response through the channel
            let _ = tx.send(response_value);
            Ok(())
        } else {
            // No waiting receiver found for this token
            Err(
                ServicesError::NotFound(format!("No pending action with token {}", args.token))
                    .into(),
            )
        }
    }

    fn handle_external_server_action(
        &self,
        token: String,
        instance_id: String,
        workflow: WorkflowResource,
        action_id: String,
    ) {
        // Clone what we need from self
        let external_action_responses = self.external_action_responses.clone();
        let manager = self.manager.clone();
        let kafka = self.kafka.clone();
        println!("Looking for action id {action_id}");

        tokio::spawn(async move {
            // Create a new oneshot channel
            let (tx, rx) = tokio::sync::oneshot::channel();

            // Store the sender
            {
                let mut response_channels = external_action_responses.lock().await;
                response_channels.insert(token.clone(), tx);
            }

            // Set up the timeout
            let timeout_future = tokio::time::timeout(std::time::Duration::from_secs(10), rx);
            kafka
                .workflows
                .request_server_action_request(token.clone(), workflow.clone(), action_id)
                .await
                .ok();

            match timeout_future.await {
                Ok(Ok(result)) => {
                    println!("0");
                    if let Ok(result) = serde_json::from_value::<ServerActionResult>(result) {
                        println!("1");
                        let workflow_definition = manager
                            .workflows
                            .lock()
                            .await
                            .get(&workflow.workflow_id)
                            .unwrap()
                            .clone();

                        println!("2");
                        if let Some(mut state) = {
                            let mut active_workflows = manager.active_workflows.lock().await;
                            active_workflows.remove(&instance_id)
                        } {
                            println!("3");
                            match manager
                                .process_server_action_results(
                                    &result,
                                    &workflow_definition,
                                    &instance_id,
                                    &mut state,
                                )
                                .await
                            {
                                Ok(_) => println!("success"),
                                Err(e) => println!("uh oh!!! {}", e),
                            }

                            println!("4");
                            let mut active_workflows = manager.active_workflows.lock().await;
                            active_workflows.insert(instance_id.clone(), state);
                        }
                    }
                }
                Ok(Err(_)) => {
                    // Handle error from response handling
                    eprintln!("Error processing external action response");
                }
                Err(_) => {
                    // Timeout occurred
                    eprintln!("Timeout waiting for external action response: {}", token);
                }
            }

            let updated = manager.get_workflow_resource(&instance_id).await.unwrap();
            {
                let active_workflows = manager.active_workflows.lock().await;
                let wf = active_workflows.get(&instance_id);
                println!(
                    "sending update for instance id {}, current node id: {:?}",
                    instance_id,
                    wf.and_then(|w| Some(w.current_node_id.clone()))
                );
            };
            kafka.workflows.update_workflow(updated).await.ok();
        });
    }
}
