use std::pin::Pin;
use std::sync::Arc;
use std::{collections::HashMap, future::Future};

use futures::lock::Mutex;
use serde_json::{Value, from_value, json};

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

#[derive(Clone)]
pub struct RoleCard {
    pub name: String,
    pub night_ability: Option<RoleAbilitySpec>,
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
            .field("night_ability", &self.night_ability)
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
        register: Some(Arc::new(|game: Arc<Mutex<GameState>>| {
            Box::pin(async move {
                // Note: take new clones for each move into each closure
                let game_for_reveal_cards = Arc::clone(&game);
                game
                    .lock()
                    .await
                    .register_server_action(
                        "reveal_cards",
                        Box::new(move |state| {
                            let game_for_reveal_cards = Arc::clone(&game_for_reveal_cards);
                            Box::pin(
                                async move {
                                    println!(
                                        "context: {:?}",
                                        game_for_reveal_cards.lock().await.get_context(&state.user_id).await
                                    );
                                    let mut response = HashMap::new();
                                    response.insert(
                                        "reveal_cards".to_string(),
                                        json!([{"name": "test", "role": "test"},{"name": "test", "role": "test"}]),
                                    );
                                    Ok(ServerActionResult::UpdateResponses(response))
                                }
                            )
                        }),
                    )
                    .await
                    .expect("unable to register reveal cards workflow");
                let game_for_reveal_player = Arc::clone(&game);
                game.lock()
                    .await
                    .register_server_action(
                        "reveal_player",
                        Box::new(move |state| {
                            let game_for_reveal_player = Arc::clone(&game_for_reveal_player);
                            Box::pin(async move {
                                println!(
                                    "context: {:?}",
                                    game_for_reveal_player
                                        .lock()
                                        .await
                                        .get_context(&state.user_id)
                                        .await
                                );
                                let mut response = HashMap::new();
                                response.insert(
                                    "reveal_player".to_string(),
                                    json!({"name": "test", "role": "test"}),
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
                        serde_json::from_str::<CreateWorkflowDefinition>(
                            r#"{
                      "id": "seer_ability_workflow",
                      "name": "Seer Ability",
                      "description": "Workflow for Seer to inspect a card",
                      "initial_node_id": "select_card_node",
                      "nodes": {
                        "select_card_node": {
                          "id": "select_card_node",
                          "title": "Select a Card",
                          "description": "Choose a player or middle card to inspect",
                          "displays": [],
                          "inputs": [
                            {
                              "id": "selected_card",
                              "label": "Which card do you want to inspect?",
                              "input_type": {
                                "SelectCard": {
                                  "filter": {
                                    "PlayerOrMiddle": {
                                      "allow_self": false
                                    }
                                  }
                                }
                              },
                              "default_value": null,
                              "required": true,
                              "width": "full"
                            }
                          ],
                          "actions": [
                            {
                              "id": "next",
                              "label": "Continue",
                              "action_type": "NextNode",
                              "target": null,
                              "style": "primary"
                            }
                          ],
                          "layout": null,
                          "condition": "Always",
                          "parent_id": null
                        },
                        "reveal_middle_prompt_node": {
                          "id": "reveal_middle_prompt_node",
                          "title": "Inspect Middle Cards",
                          "description": "Choose a second middle card to inspect",
                          "displays": [],
                          "inputs": [
                            {
                              "id": "selected_card_2",
                              "label": "Pick another middle card",
                              "input_type": {
                                "SelectCard": {
                                  "filter": "MiddleOnly"
                                }
                              },
                              "default_value": null,
                              "required": true,
                              "width": "full"
                            }
                          ],
                          "actions": [
                            {
                              "id": "next",
                              "label": "Reveal Cards",
                              "action_type": "NextNode",
                              "target": null,
                              "style": "primary"
                            }
                          ],
                          "layout": null,
                          "condition": {
                            "ResponseEquals": {
                              "field": "selected_card.type",
                              "value": "Middle"
                            }
                          },
                          "parent_id": "select_card_node"
                        },
                        "reveal_middle_node": {
                          "id": "reveal_middle_node",
                          "title": "Middle Cards Revealed",
                          "description": null,
                          "displays": [
                            {
                              "id": "middle_result",
                              "display_type": {
                                "Text": {
                                  "text_key": "reveal_cards"
                                }
                              }
                            }
                          ],
                          "inputs": [],
                          "actions": [],
                          "layout": null,
                          "condition": {
                            "ResponseExists": "reveal_cards"
                          },
                          "parent_id": "reveal_middle_prompt_node"
                        },
                        "player_result_node": {
                          "id": "player_result_node",
                          "title": "Player Card Seen",
                          "description": null,
                          "displays": [
                            {
                              "id": "player_result_text",
                              "display_type": {
                                "Text": {
                                  "text_key": "reveal_player"
                                }
                              }
                            }
                          ],
                          "inputs": [],
                          "actions": [],
                          "layout": null,
                          "condition": {
                            "ResponseExists": "player_seen"
                          },
                          "parent_id": "select_card_node"
                        }
                      },
                      "responses": {},
                      "server_actions": {
                        "reveal_player": {
                          "id": "reveal_player",
                          "name": "Reveal Player Role",
                          "description": "Resolves the selected player's role"
                        },
                        "reveal_cards": {
                          "id": "reveal_cards",
                          "name": "Reveal Middle Cards",
                          "description": "Resolves two middle cards"
                        }
                      }
                    }
                    "#,
                        )
                        .unwrap(),
                    )
                    .await
                    .expect("unable to register wf");
            })
        })),
        night_ability: Some(RoleAbilitySpec {
            target_selector: TargetSelector::SinglePlayer,
            validator: None,
            description: "Inspect a player's role.".to_string(),
            priority: 30,
            duration_secs: 10,
            allowed_phases: AbilityPhaseScope::Night,
            allowed_turns: AbilityTurnScope::YourTurn,
            condition: None,
            ability: Arc::new(|_ctx: RoleContext| {
                Box::pin(async move {
                    // The Seer opens a workflow - there is no normal input map here
                    Some(WorkflowDefinitionWithInput {
                        definition: "user-bot-wf-seer_ability_workflow".to_string(),
                        input: HashMap::new(),
                    })
                })
            }),
        }),
    }
}

pub fn werewolf_card() -> RoleCard {
    RoleCard {
        register: None,
        name: "Werewolf".to_string(),
        night_ability: Some(RoleAbilitySpec {
            duration_secs: 10,
            ability: Arc::new(|ctx: RoleContext| {
                Box::pin(async move {
                    if let Some(ActionTarget::Player(pid)) = ctx.targets.get(0) {
                        let state = ctx.game.lock().await;
                        let player = &state.players[pid];
                        println!(
                            "🐺 Werewolf inspects {}: {}",
                            player.name, player.role_card.name
                        );
                    }

                    None
                })
            }),
            target_selector: TargetSelector::SinglePlayer,
            validator: Some(Arc::new(|ctx: RoleContext| {
                Box::pin(async move {
                    let game = ctx.game.lock().await;
                    let werewolves = game
                        .players
                        .values()
                        .filter(|p| p.role_card.name == "Werewolf")
                        .count();
                    werewolves == 1
                })
            })),
            description: "Inspect a player (only if solo)".to_string(),
            priority: 40,
            allowed_phases: AbilityPhaseScope::Night,
            allowed_turns: AbilityTurnScope::YourTurn,
            condition: None,
        }),
    }
}

pub fn villager_card() -> RoleCard {
    RoleCard {
        register: None,
        name: "Villager".to_string(),
        night_ability: None,
    }
}

pub fn witch_card() -> RoleCard {
    RoleCard {
        register: None,
        name: "Witch".to_string(),
        night_ability: Some(RoleAbilitySpec {
            duration_secs: 10,
            ability: Arc::new(|ctx: RoleContext| {
                Box::pin(async move {
                    if let (Some(ActionTarget::Player(from)), Some(ActionTarget::Player(to))) =
                        (ctx.targets.get(0), ctx.targets.get(1))
                    {
                        println!("🧙 Witch redirects {}'s action to {}", from, to);
                    }
                    None
                })
            }),
            target_selector: TargetSelector::PlayerAndPlayer,
            validator: None,
            description: "Redirect another player's action.".to_string(),
            priority: 50,
            allowed_phases: AbilityPhaseScope::Night,
            allowed_turns: AbilityTurnScope::YourTurn,
            condition: None,
        }),
    }
}

pub fn doppelganger_card() -> RoleCard {
    RoleCard {
        register: None,
        name: "Doppelgänger".to_string(),
        night_ability: Some(RoleAbilitySpec {
            duration_secs: 15,
            ability: Arc::new(|ctx: RoleContext| {
                Box::pin(async move {
                    if let Some(ActionTarget::Player(pid)) = ctx.targets.get(0) {
                        let mut game = ctx.game.lock().await;
                        // First, clone the role_card arc while we have only immutable access
                        let target_role_card = game.players[pid].role_card.clone();
                        let target_role_card_name = game.players[pid].role_card.name.clone();
                        println!("🌀 Doppelgänger copied {}", target_role_card_name);
                        // Now, do the mutable borrow
                        if let Some(actor) = game.players.get_mut(&ctx.actor) {
                            actor.copied_role_card = Some(target_role_card);
                        }
                    }
                    None
                })
            }),
            target_selector: TargetSelector::SinglePlayer,
            validator: None,
            description: "Copy another player's role.".to_string(),
            priority: 10,
            allowed_phases: AbilityPhaseScope::Night,
            allowed_turns: AbilityTurnScope::YourTurn,
            condition: None,
        }),
    }
}
