use std::pin::Pin;
use std::sync::Arc;
use std::{collections::HashMap, future::Future};

use futures::lock::Mutex;
use serde_json::json;

use crate::roles::{Alliance, RoleCard, WorkflowDefinitionWithInput};
use crate::workflow::server_action::{ServerActionContext, ServerActionResult};
use crate::{
    gamestate::{GameState, RoleContext},
    workflow::CreateWorkflowDefinition,
};

async fn register_seer_workflow_definition(game: Arc<Mutex<GameState>>) {
    let workflow: CreateWorkflowDefinition = serde_json::from_str(include_str!("./seer.json"))
        .expect("Failed to parse seer.json workflow definition");

    game.lock()
        .await
        .register_workflow_definition(workflow)
        .await
        .expect("Failed to register seer.json workflow");
}

async fn register_reveal_player_action(game: Arc<Mutex<GameState>>) {
    let game_clone = Arc::clone(&game);

    game.lock()
        .await
        .register_server_action(
            "reveal_player",
            Box::new(move |state| {
                let game = Arc::clone(&game_clone);
                Box::pin(async move {
                    tracing::debug!("Getting role for user_id: {}", state.user_id);
                    let role = game
                        .lock()
                        .await
                        .get_user_effective_role(&state.user_id)
                        .await?;
                    tracing::debug!("Got role: {}", role.name);

                    if role.name == "Witch" {
                        tracing::info!("Witch is sabotaging the Seer's workflow.");
                        game.lock()
                            .await
                            .set_sabotage_inputs("seer", &state.workflow_id, state.inputs.clone())
                            .await;

                        return Ok(ServerActionResult::CompleteWorkflow {
                            message: "Sabotage complete.".to_string(),
                            responses: HashMap::new(),
                        });
                    }

                    // Possibly sabotaged
                    let effective_inputs = game
                        .lock()
                        .await
                        .get_sabotage_inputs("seer", &state.workflow_id)
                        .await
                        .unwrap_or_else(|| state.inputs.clone());

                    tracing::debug!("Effective inputs: {:?}", effective_inputs);

                    let user_id = ServerActionContext::get_required_nested_value_as_str(
                        &effective_inputs,
                        "selected_card.Player.id",
                    )?;

                    let user = {
                        let game_lock = game.lock().await;
                        game_lock.get_player(user_id).await?
                    };

                    let mut response = HashMap::new();
                    response.insert(
                        "reveal_player".to_string(),
                        json!([{
                            "name": user.name,
                            "card": &*user.role_card,
                        }]),
                    );

                    Ok(ServerActionResult::UpdateResponses(response))
                })
            }),
        )
        .await
        .expect("Failed to register reveal_player server action");
}

fn register_workflows(game: Arc<Mutex<GameState>>) -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async move {
        register_reveal_player_action(game.clone()).await;
        register_seer_workflow_definition(game.clone()).await;
    })
}

pub fn seer_card() -> RoleCard {
    RoleCard {
        name: "Seer".to_string(),
        priority: 50,
        alliance: Alliance::Villager,
        register: Some(Arc::new(register_workflows)),
        night_ability: Some(Arc::new(|_ctx: RoleContext| {
            Box::pin(async move {
                Some(WorkflowDefinitionWithInput {
                    definition: "user-bot-wf-seer_ability_workflow".to_string(),
                    input: HashMap::new(),
                })
            })
        })),
    }
}
