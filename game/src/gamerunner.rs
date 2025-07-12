use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use futures::lock::Mutex;
use serde_json::json;
use tokio::time::sleep;

use tokio::sync::broadcast;

use crate::gamestate::{ActionTarget, GameState, RoleContext};
use crate::roles::RoleAbilitySpec;
use crate::workflow::service::{ProcessWorkflowActionArgs, WorkflowResource};
use crate::workflow::{DisplayType, WorkflowState};

#[derive(Debug, Clone)]
pub enum GameEvent {
    TurnStarted {
        player_id: String,
        ability: RoleAbilitySpec,
    },
    AbilityExecuted {
        player_id: String,
        description: String,
    },
    TurnExpired {
        player_id: String,
    },
    StartWorkflow {
        player_id: String,
        workflow: WorkflowResource,
    },
    UpdateWorkflow {
        player_id: String,
        workflow: WorkflowResource,
    },
}

pub enum PlayableAbility {
    NightAbility,
    AbilityById(String),
}

pub type GameEventSender = broadcast::Sender<GameEvent>;
pub type GameEventReceiver = broadcast::Receiver<GameEvent>;

pub struct GameRunner {
    pub game: Arc<Mutex<GameState>>,
    pub stages: VecDeque<(String, RoleAbilitySpec)>,
    pub event_sender: GameEventSender,
    pub pending_actions: Arc<Mutex<HashMap<String, (RoleAbilitySpec, Vec<ActionTarget>)>>>,
}

impl GameRunner {
    pub async fn new(game: GameState, event_sender: GameEventSender) -> Arc<Mutex<Self>> {
        let game = Arc::new(Mutex::new(game));

        let mut all_abilities: Vec<(String, RoleAbilitySpec)> = {
            let g = game.lock().await;
            g.players
                .iter()
                .filter_map(|(id, player)| {
                    player
                        .get_night_ability()
                        .map(|ability| (id.clone(), ability))
                })
                .collect()
        };

        all_abilities.sort_by_key(|(_, a)| a.priority);
        let stages = VecDeque::from(all_abilities);

        Arc::new(Mutex::new(Self {
            game,
            stages,
            event_sender,
            pending_actions: Arc::new(Mutex::new(HashMap::new())),
        }))
    }

    pub async fn submit_action(
        &self,
        player_id: String,
        ability: RoleAbilitySpec,
        targets: Vec<ActionTarget>,
    ) -> Result<(), String> {
        let mut pending = self.pending_actions.lock().await;
        if pending.contains_key(&player_id) {
            return Err("Action already submitted".into());
        }
        pending.insert(player_id.clone(), (ability.clone(), targets));
        drop(pending);

        self.play_ability(&player_id, PlayableAbility::NightAbility)
            .await?;
        Ok(())
    }

    pub async fn update_workflow(&self, player_id: &str, workflow: WorkflowResource) {
        self.event_sender
            .send(GameEvent::UpdateWorkflow {
                player_id: player_id.to_string(),
                workflow,
            })
            .ok();
    }

    pub async fn process_workflow_action(
        &self,
        player_id: &str,
        args: ProcessWorkflowActionArgs,
    ) -> Result<(), String> {
        let mut game = self.game.lock().await;
        let instance = game
            .workflow
            .process_action(player_id, args)
            .await
            .map_err(|e| format!("Workflow processing error: {}", e))?;

        drop(game);

        self.update_workflow(player_id, instance).await;
        Ok(())
    }

    pub async fn play_ability(&self, player_id: &str, kind: PlayableAbility) -> Result<(), String> {
        let action = {
            let mut pending = self.pending_actions.lock().await;
            pending.remove(player_id)
        };

        if let Some((ability, targets)) = action {
            let ctx = RoleContext::new(Arc::clone(&self.game), player_id.to_string(), targets);
            if let Some(workflow_definition_with_input) = (&ability.ability)(ctx).await {
                self.game
                    .lock()
                    .await
                    .start_workflow(player_id, workflow_definition_with_input)
                    .await
                    .expect("hmm workflow problems");
            }

            self.event_sender
                .send(GameEvent::AbilityExecuted {
                    player_id: player_id.to_string(),
                    description: ability.description.clone(),
                })
                .ok();
        }

        Ok(())
    }

    pub async fn register_cards(&self) {
        let all_cards = {
            let game = self.game.lock().await;
            let mut all_cards = game.players.clone();
            all_cards.extend(game.middles.clone());
            all_cards
        };
        for card in all_cards.values() {
            if let Some(register) = &card.role_card.register {
                (register)(self.game.clone()).await;
            }
        }
    }

    pub async fn run(runner: Arc<Mutex<Self>>) {
        {
            // Only hold the lock while registering cards
            let runner_guard = runner.lock().await;
            runner_guard.register_cards().await;
        }

        loop {
            // STEP 1: Pop the next stage
            let (player_id, ability) = {
                let mut runner_guard = runner.lock().await;
                match runner_guard.stages.pop_front() {
                    Some(stage) => stage,
                    None => return,
                }
            };

            println!("⏳ It's {}'s turn: {}", player_id, ability.description);

            // STEP 2: Emit TurnStarted event
            {
                let runner_guard = runner.lock().await;
                runner_guard
                    .event_sender
                    .send(GameEvent::TurnStarted {
                        player_id: player_id.clone(),
                        ability: ability.clone(),
                    })
                    .ok();
            }

            // STEP 3: Prepare RoleContext and check should_execute
            let (should_execute, duration) = {
                let runner_guard = runner.lock().await;
                let ctx =
                    RoleContext::new(Arc::clone(&runner_guard.game), player_id.clone(), vec![]);

                let should_execute = runner_guard.should_execute(&ctx, &ability).await;
                let duration = Duration::from_secs(ability.duration_secs as u64);

                (should_execute, duration)
            };

            if !should_execute {
                println!("❌ Skipping {} (conditions not met)", player_id);
                continue;
            }

            // STEP 4: Wait for the turn duration without holding any lock
            println!(
                "🔔 Waiting {}s for {} to act...",
                ability.duration_secs, player_id
            );
            sleep(duration).await;

            // STEP 5: Emit TurnExpired event
            {
                let runner_guard = runner.lock().await;
                runner_guard
                    .event_sender
                    .send(GameEvent::TurnExpired {
                        player_id: player_id.clone(),
                    })
                    .ok();
            }
        }
    }

    // pub async fn run(runner: Arc<Mutex<Self>>) {
    //     {
    //         runner.lock().await.register_cards().await;
    //     }
    //     loop {
    //         let (player_id, ability) = {
    //             let mut guard = runner.lock().await;
    //             match guard.stages.pop_front() {
    //                 Some(stage) => stage,
    //                 None => return,
    //             }
    //         };

    //         println!("⏳ It's {}'s turn: {}", player_id, ability.description);

    //         runner
    //             .lock()
    //             .await
    //             .event_sender
    //             .send(GameEvent::TurnStarted {
    //                 player_id: player_id.clone(),
    //                 ability: ability.clone(),
    //             })
    //             .ok();

    //         let ctx = RoleContext::new(
    //             Arc::clone(&runner.lock().await.game),
    //             player_id.clone(),
    //             vec![],
    //         );

    //         if !runner.lock().await.should_execute(&ctx, &ability).await {
    //             println!("❌ Skipping {} (conditions not met)", player_id);
    //             continue;
    //         }

    //         let duration = Duration::from_secs(ability.duration_secs as u64);
    //         println!(
    //             "🔔 Waiting {}s for {} to act...",
    //             ability.duration_secs, player_id
    //         );
    //         runner.lock().await.wait_for_turn(duration).await;

    //         runner
    //             .lock()
    //             .await
    //             .event_sender
    //             .send(GameEvent::TurnExpired {
    //                 player_id: player_id.clone(),
    //             })
    //             .ok();
    //     }
    // }

    async fn should_execute(&self, ctx: &RoleContext, ability: &RoleAbilitySpec) -> bool {
        if let Some(cond) = &ability.condition {
            if !cond(&*ctx.get_game().lock().await) {
                return false;
            }
        }
        if let Some(validator) = &ability.validator {
            return validator(ctx.clone()).await;
        }
        true
    }

    async fn wait_for_turn(&self, duration: Duration) {
        sleep(duration).await;
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use serde_json::json;
    use tokio::sync::broadcast;

    use crate::{
        gamerunner::{GameEvent, GameRunner},
        gamestate::{ActionTarget, GameState, Player},
        kafka::service::KafkaService,
        roles::{doppelganger_card, seer_card, villager_card, witch_card},
        workflow::service::ProcessWorkflowActionArgs,
    };

    #[tokio::test]
    async fn test_workflow_progression() {
        let mut dopple = doppelganger_card();
        let mut witch = witch_card();
        let villager1 = villager_card();
        let villager2 = villager_card();
        let mut seer = seer_card();

        if let Some(ability) = dopple.night_ability.as_mut() {
            ability.duration_secs = 1;
        }
        if let Some(ability) = seer.night_ability.as_mut() {
            ability.duration_secs = 1;
        }

        if let Some(ability) = witch.night_ability.as_mut() {
            ability.duration_secs = 1;
        }

        let players = vec![
            Player::new("dopple", "Dopple Dan", Arc::new(dopple)),
            Player::new("witch", "Witch Wanda", Arc::new(witch)),
            Player::new("villager1", "Vince", Arc::new(villager1.clone())),
            Player::new("villager2", "Violet", Arc::new(villager2)),
            Player::new("seer", "Seer Sam", Arc::new(seer)),
        ];

        let middles = vec![
            Player::new("middle1", "middle 1", Arc::new(villager1.clone())),
            Player::new("middle2", "middle 2", Arc::new(villager1.clone())),
            Player::new("middle3", "middle 3", Arc::new(villager1.clone())),
        ];

        let kafka = KafkaService::new("test");
        let state = GameState::new(players, middles, Arc::new(kafka)).await;
        let (tx, mut rx) = broadcast::channel(16);
        let runner = GameRunner::new(state, tx.clone()).await;
        let runner_inner = runner.clone();

        tokio::spawn(async move {
            while let Ok(event) = rx.recv().await {
                match event {
                    GameEvent::TurnStarted { player_id, ability } => {
                        let targets = match player_id.as_str() {
                            "dopple" => vec![ActionTarget::Player("witch".into())],
                            "witch" => vec![
                                ActionTarget::Player("seer".into()),
                                ActionTarget::Player("villager2".into()),
                            ],
                            "seer" => vec![ActionTarget::Player("dopple".into())],
                            _ => vec![],
                        };

                        let runner_clone = Arc::clone(&runner_inner);
                        tokio::spawn(async move {
                            let _ = runner_clone
                                .lock()
                                .await
                                .submit_action(player_id.clone(), ability.clone(), targets)
                                .await;
                            println!("submitted action");
                        });
                    }

                    GameEvent::UpdateWorkflow {
                        player_id,
                        workflow,
                    } => {
                        println!("🔁 Workflow updated for {} : {:?}", player_id, workflow);

                        let args = match workflow.current_node_id.as_str() {
                            "select_card_node" => {
                                // Simulate selecting a card to view
                                let mut input = HashMap::new();
                                input.insert(
                                    "selected_card".to_string(),
                                    json!({"Player": {"id": "seer"}}),
                                );
                                ProcessWorkflowActionArgs::new(
                                    workflow.instance_id,
                                    "NextNode".into(),
                                    input,
                                )
                            }
                            _ => continue,
                        };

                        let runner_clone = Arc::clone(&runner_inner);
                        tokio::spawn(async move {
                            runner_clone
                                .lock()
                                .await
                                .process_workflow_action(&player_id, args)
                                .await
                                .expect("workflow action failed");
                        });
                    }

                    _ => {}
                }
            }
        });

        GameRunner::run(runner.clone()).await;
    }
}
