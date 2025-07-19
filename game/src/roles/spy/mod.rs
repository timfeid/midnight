use std::pin::Pin;
use std::sync::Arc;
use std::{collections::HashMap, future::Future};

use futures::lock::Mutex;
use serde_json::json;

use crate::error::ServicesError;
use crate::roles::{Alliance, RoleCard, WorkflowDefinitionWithInput};
use crate::workflow::server_action::ServerActionResult;
use crate::workflow::{ActionType, WorkflowPredicate};
use crate::{
    gamestate::{GameState, RoleContext},
    workflow::CreateWorkflowDefinition,
};

async fn register_start_role_workflow(game: Arc<Mutex<GameState>>) {
    let game_for_observe = game.clone();
    game.lock()
        .await
        .register_server_action(
            "start_selected_role_workflow",
            Box::new(move |state| {
                let game = Arc::clone(&game_for_observe);
                Box::pin(async move {
                    let chosen_role_name = state
                        .get_input("chosen_role")
                        .and_then(|v| v.as_str())
                        .ok_or(ServicesError::InternalError(
                            "Missing role selection".into(),
                        ))?;

                    let lock = game.lock().await;

                    // Get all matching role cards
                    let selected = lock
                        .all_cards()
                        .iter()
                        .find(|r| r.name == chosen_role_name)
                        .cloned();

                    let Some(role) = &selected else {
                        return Ok(ServerActionResult::CompleteWorkflow {
                            message: "Invalid role selected.".into(),
                            responses: HashMap::new(),
                        });
                    };

                    let Some(ability) = &role.night_ability else {
                        return Ok(ServerActionResult::CompleteWorkflow {
                            message: "Selected role has no ability.".into(),
                            responses: HashMap::new(),
                        });
                    };

                    let context = RoleContext::new(Arc::clone(&game), state.user_id.clone());
                    let Some(workflow) = ability(context).await else {
                        return Ok(ServerActionResult::CompleteWorkflow {
                            message: "No workflow returned.".into(),
                            responses: HashMap::new(),
                        });
                    };

                    if let Some(player) = lock.get_player_by_role(chosen_role_name).await.ok() {
                        if player.middle_position.is_none() {
                            return Ok(ServerActionResult::WaitForPredicate {
                                predicate: WorkflowPredicate::ByUserId(player.id),
                                inject_response_as: Some("observed_results".to_string()),
                                on_complete: Some(ActionType::NextNode),
                            });
                        }
                    }

                    let mut input = HashMap::new();
                    input.insert(
                        "observed_results".to_string(),
                        json!("Better luck next time"),
                    );
                    Ok(ServerActionResult::UpdateResponses(input))
                })
            }),
        )
        .await
        .expect("Failed to register spy observer action");
}

async fn register_workflow_definition(game: Arc<Mutex<GameState>>) {
    game.lock()
        .await
        .register_workflow_definition(
            serde_json::from_str::<CreateWorkflowDefinition>(include_str!("./spy.json")).unwrap(),
        )
        .await
        .unwrap();
}

fn register(game: Arc<Mutex<GameState>>) -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async move {
        register_start_role_workflow(game.clone()).await;
        register_workflow_definition(game.clone()).await;
    })
}

pub fn spy_card() -> RoleCard {
    RoleCard {
        priority: 0,
        register: Some(Arc::new(register)),

        alliance: Alliance::Villager,
        name: "Spy".to_string(),
        night_ability: Some(Arc::new(|ctx: RoleContext| {
            Box::pin(async move {
                let options = ctx
                    .game
                    .lock()
                    .await
                    .all_cards()
                    .iter()
                    .filter(|c| {
                        c.night_ability.is_some()
                            && c.name != "Spy"
                            && c.alliance != Alliance::Werewolf
                    })
                    .map(|r| json!({ "label": r.name, "value": r.name }))
                    .collect::<Vec<_>>();

                let mut input = HashMap::new();
                input.insert("observe_role_options".to_string(), json!(options));
                Some(WorkflowDefinitionWithInput {
                    definition: "user-bot-wf-spy_observe_workflow".to_string(),
                    input,
                })
            })
        })),
    }
}
