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

async fn register_workflow(game: Arc<Mutex<GameState>>) {
    game.lock()
        .await
        .register_workflow_definition(
            serde_json::from_str::<CreateWorkflowDefinition>(include_str!("./werewolf.json"))
                .unwrap(),
        )
        .await
        .expect("unable to register wf");
}

async fn register_reveal_cards(game: Arc<Mutex<GameState>>) {
    let game_clone = game.clone();
    game.lock()
        .await
        .register_server_action(
            "reveal_cards",
            Box::new(move |state| {
                let game = Arc::clone(&game_clone);

                Box::pin(async move {
                    let key = "selected_card.Middle.id";
                    let middle_id_1 = state.get_required_input_as_str(key)?;
                    let key = "selected_card_2.Middle.id";
                    let middle_id_2 = state.get_required_input_as_str(key).ok();

                    let (middle1, middle2) = {
                        let game = game.lock().await;
                        let middle1 = game.get_player(middle_id_1).await?;
                        let middle2 = if let Some(middle_id_2) = middle_id_2 {
                            game.get_player(middle_id_2).await.ok()
                        } else {
                            None
                        };

                        (middle1, middle2)
                    };

                    let mut response = HashMap::new();
                    response.insert(
                        "reveal_middle_one".to_string(),
                        json!({"name": middle1.name, "card": &*middle1.role_card}),
                    );

                    if let Some(middle2) = middle2 {
                        response.insert(
                            "reveal_middle_two".to_string(),
                            json!({"name": middle2.name, "card": &*middle2.role_card}),
                        );
                    }

                    Ok(ServerActionResult::UpdateResponses(response))
                })
            }),
        )
        .await
        .expect("unable to register reveal cards workflow");
}

fn register_server_actions(
    game: Arc<Mutex<GameState>>,
) -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async move {
        register_reveal_cards(game.clone()).await;
        register_workflow(game.clone()).await;
    })
}

pub fn werewolf_card() -> RoleCard {
    RoleCard {
        priority: 20,
        register: Some(Arc::new(register_server_actions)),
        alliance: Alliance::Werewolf,
        name: "Werewolf".to_string(),
        night_ability: Some(Arc::new(|ctx: RoleContext| {
            Box::pin(async move {
                let werewolves = ctx
                    .game
                    .lock()
                    .await
                    .players
                    .values()
                    .filter(|x| x.effective_role_card().alliance == Alliance::Werewolf)
                    .count();

                if werewolves == 1 {
                    Some(WorkflowDefinitionWithInput {
                        definition: "user-bot-wf-werewolf_ability_workflow".to_string(),
                        input: HashMap::new(),
                    })
                } else {
                    None
                }
            })
        })),
    }
}
