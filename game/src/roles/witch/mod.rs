use std::pin::Pin;
use std::sync::Arc;
use std::{collections::HashMap, future::Future};

use futures::lock::Mutex;
use serde_json::json;

use crate::roles::{Alliance, RoleCard, WorkflowDefinitionWithInput};
use crate::workflow::server_action::ServerActionResult;
use crate::{
    gamestate::{GameState, RoleContext},
    workflow::CreateWorkflowDefinition,
};

async fn register_show_sabotaged_results(game: Arc<Mutex<GameState>>) {
    let game_clone = game.clone();
    game.lock()
        .await
        .register_server_action(
            "show_sabotaged_results_workflow",
            Box::new(move |_state| {
                let _game = game_clone.clone();
                Box::pin(async move {
                    Ok(ServerActionResult::UpdateResponses(HashMap::from([(
                        "results".to_string(),
                        json!("results"),
                    )])))
                })
            }),
        )
        .await
        .expect("unable to register show_sabotaged_results_workflow");
}

async fn register_start_sabotaged_role_workflow(game: Arc<Mutex<GameState>>) {
    let game_clone = game.clone();
    game.lock()
        .await
        .register_server_action(
            "start_sabotaged_role_workflow",
            Box::new(move |state| {
                let game = game_clone.clone();
                Box::pin(async move {
                    let game_lock = game.lock().await;

                    let candidates = game_lock
                        .get_sabotage_candidates(&["Witch"], Some("Seer"))
                        .await;

                    let Some(selected) = game_lock.pick_random_role(&candidates).await else {
                        tracing::warn!("No valid sabotage target found.");
                        return Ok(ServerActionResult::CompleteWorkflow {
                            responses: HashMap::new(),
                            message: "No role to sabotage.".into(),
                        });
                    };

                    let Some(night_ability) = &selected.night_ability else {
                        tracing::warn!("Selected role has no night ability");
                        return Ok(ServerActionResult::CompleteWorkflow {
                            responses: HashMap::new(),
                            message: "Role lacks night ability.".into(),
                        });
                    };

                    let ctx = RoleContext::new(game.clone(), state.user_id.clone());
                    let Some(workflow) = night_ability(ctx).await else {
                        tracing::warn!("Night ability did not return a workflow");
                        return Ok(ServerActionResult::CompleteWorkflow {
                            responses: HashMap::new(),
                            message: "Failed to launch sabotage.".into(),
                        });
                    };

                    tracing::info!(role = %selected.name, "Launching sabotage workflow");
                    Ok(ServerActionResult::StartAndWaitWorkflow {
                        definition_id: workflow.definition,
                        inputs: workflow.input,
                        inject_workflow_as: None,
                        on_complete: None,
                    })
                })
            }),
        )
        .await
        .expect("unable to register start_sabotaged_role_workflow");
}

async fn register_witch_workflow_definition(game: Arc<Mutex<GameState>>) {
    game.lock()
        .await
        .register_workflow_definition(
            serde_json::from_str::<CreateWorkflowDefinition>(include_str!("./sabotage.json"))
                .unwrap(),
        )
        .await
        .unwrap();
}
fn register_witch_workflows(
    game: Arc<Mutex<GameState>>,
) -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async move {
        register_show_sabotaged_results(game.clone()).await;
        register_start_sabotaged_role_workflow(game.clone()).await;
        register_witch_workflow_definition(game.clone()).await;
    })
}

pub fn witch_card() -> RoleCard {
    RoleCard {
        priority: 0,
        register: Some(Arc::new(register_witch_workflows)),

        alliance: Alliance::Villager,
        name: "Witch".to_string(),
        night_ability: Some(Arc::new(|_ctx: RoleContext| {
            Box::pin(async move {
                Some(WorkflowDefinitionWithInput {
                    definition: "user-bot-wf-witch_sabotage_workflow".to_string(),
                    input: HashMap::new(),
                })
            })
        })),
    }
}
