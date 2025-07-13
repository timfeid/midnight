use std::pin::Pin;
use std::sync::Arc;
use std::{collections::HashMap, future::Future};

use futures::lock::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, from_value, json};

use crate::error::ServicesError;
use crate::workflow::server_action::ServerActionResult;
use crate::{
    gamestate::{ActionTarget, GameState, RoleContext},
    workflow::{CreateWorkflowDefinition, WorkflowDefinition},
};

pub struct WorkflowDefinitionWithInput {
    pub definition: String,
    pub input: HashMap<String, serde_json::Value>,
}

pub type RoleAbility = Arc<
    dyn Fn(RoleContext) -> Pin<Box<dyn Future<Output = Option<WorkflowDefinitionWithInput>> + Send>>
        + Send
        + Sync,
>;

pub type RoleValidator =
    Arc<dyn Fn(RoleContext) -> Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync>;

#[derive(Clone, Debug)]
pub enum AbilityPhaseScope {
    Night,
    Day,
    Any,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum Alliance {
    Werewolf,
    Villager,
}

#[derive(Clone, Debug)]
pub enum AbilityTurnScope {
    YourTurn,
    OtherTurn,
    SpecificRole(String),
}

#[derive(Clone)]
pub struct RoleAbilitySpec {
    pub ability: RoleAbility,
    pub target_selector: TargetSelector,
    pub validator: Option<RoleValidator>,
    pub description: String,
    pub priority: i32,
    pub allowed_turns: AbilityTurnScope,
    pub allowed_phases: AbilityPhaseScope,
    pub condition: Option<Arc<dyn Fn(&GameState) -> bool + Send + Sync>>,
    pub duration_secs: i32,
}

impl std::fmt::Debug for RoleAbilitySpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RoleAbilitySpec")
            .field("target_selector", &self.target_selector)
            .field("description", &self.description)
            .field("priority", &self.priority)
            .field("allowed_turns", &self.allowed_turns)
            .field("allowed_phases", &self.allowed_phases)
            .field("duration_secs", &self.duration_secs)
            .finish()
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub struct RoleCard {
    pub name: String,
    pub alliance: Alliance,
    pub priority: i32,

    #[serde(skip_serializing, skip_deserializing)]
    pub night_ability: Option<RoleAbility>,
    #[serde(skip_serializing, skip_deserializing)]
    pub register: Option<
        Arc<
            dyn Fn(Arc<Mutex<GameState>>) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync,
        >,
    >,
}

impl std::fmt::Debug for RoleCard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RoleCard")
            .field("name", &self.name)
            .finish()
    }
}

#[derive(Clone, Debug)]
pub enum TargetSelector {
    SinglePlayer,
    PlayerAndPlayer,
    None,
}

pub fn seer_card() -> RoleCard {
    RoleCard {
        name: "Seer".to_string(),
        priority: 50,
        alliance: Alliance::Villager,
        register: Some(Arc::new(|game: Arc<Mutex<GameState>>| {
            Box::pin(async move {
                // Note: take new clones for each move into each closure
                let game_for_reveal_player = Arc::clone(&game);
                game.lock()
                    .await
                    .register_server_action(
                        "reveal_player",
                        Box::new(move |state| {
                            let game_for_reveal_player = Arc::clone(&game_for_reveal_player);
                            Box::pin(async move {
                                let context = {
                                    game_for_reveal_player
                                        .lock()
                                        .await
                                        .get_context(&state.user_id)
                                        .await
                                        .ok_or(ServicesError::InternalError(
                                            "something".to_string(),
                                        ))?
                                };
                                let user_id = state
                                    .get_input("selected_card.Player.id")
                                    .and_then(|x| x.as_str())
                                    .ok_or(ServicesError::InternalError(
                                        "No player id supplied".to_string(),
                                    ))?;
                                println!("user_id: {user_id}",);
                                let user = {
                                    let game = context.game.lock().await;
                                    game.players
                                        .get(user_id)
                                        .ok_or(ServicesError::InternalError(format!(
                                            "Unable to find player with id {user_id}"
                                        )))?
                                        .clone()
                                };
                                let mut response = HashMap::new();
                                response.insert(
                                    "reveal_player".to_string(),
                                    json!([{"name": user.name, "card": &*user.role_card}]),
                                );
                                Ok(ServerActionResult::UpdateResponses(response))
                            })
                        }),
                    )
                    .await
                    .expect("unable to register reveal player workflow");

                game.lock()
                    .await
                    .register_workflow_definition(
                        serde_json::from_str::<CreateWorkflowDefinition>(include_str!(
                            "workflows/seer.json"
                        ))
                        .unwrap(),
                    )
                    .await
                    .expect("unable to register wf");
            })
        })),
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

pub fn werewolf_card() -> RoleCard {
    RoleCard {
        priority: 20,
        register: Some(Arc::new(|game: Arc<Mutex<GameState>>| {
            Box::pin(async move {
                // Note: take new clones for each move into each closure
                let game_for_reveal_cards = Arc::clone(&game);
                game.lock()
                    .await
                    .register_server_action(
                        "reveal_cards",
                        Box::new(move |state| {
                            let game_for_reveal_cards = Arc::clone(&game_for_reveal_cards);
                            Box::pin(async move {
                                println!(
                                    "context: {:?}",
                                    game_for_reveal_cards
                                        .lock()
                                        .await
                                        .get_context(&state.user_id)
                                        .await
                                );
                                let mut response = HashMap::new();
                                response.insert(
                                    "reveal_middle_one".to_string(),
                                    json!({"name": "test", "role": "test"}),
                                );
                                Ok(ServerActionResult::UpdateResponses(response))
                            })
                        }),
                    )
                    .await
                    .expect("unable to register reveal cards workflow");

                game.lock()
                    .await
                    .register_workflow_definition(
                        serde_json::from_str::<CreateWorkflowDefinition>(include_str!(
                            "workflows/werewolf.json"
                        ))
                        .unwrap(),
                    )
                    .await
                    .expect("unable to register wf");
            })
        })),
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

pub fn villager_card() -> RoleCard {
    RoleCard {
        priority: 0,
        register: None,
        name: "Villager".to_string(),
        night_ability: None,
        alliance: Alliance::Villager,
    }
}

pub fn witch_card() -> RoleCard {
    RoleCard {
        priority: 40,
        register: None,
        alliance: Alliance::Villager,
        name: "Witch".to_string(),
        night_ability: Some(Arc::new(|ctx: RoleContext| Box::pin(async move { None }))),
    }
}

pub fn doppelganger_card() -> RoleCard {
    RoleCard {
        priority: 0,
        alliance: Alliance::Villager,
        register: None,
        name: "Doppelgänger".to_string(),
        night_ability: Some(Arc::new(|ctx: RoleContext| Box::pin(async move { None }))),
    }
}
