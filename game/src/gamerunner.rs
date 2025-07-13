use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use futures::lock::Mutex;
use serde_json::json;
use tokio::time::sleep;

use tokio::sync::broadcast;

use crate::gamestate::{ActionTarget, GameState, RoleContext};
use crate::roles::{RoleAbility, RoleAbilitySpec, RoleCard};
use crate::workflow::service::{ProcessWorkflowActionArgs, WorkflowResource};
use crate::workflow::{DisplayType, WorkflowState};

#[derive(Debug, Clone)]
pub enum GameEvent {
    TurnStarted {
        player_id: String,
        role: RoleCard,
    },
    AbilityExecuted {
        player_id: String,
    },
    TurnExpired {
        player_id: String,
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
    pub stages: VecDeque<(String, RoleCard)>,
    pub event_sender: GameEventSender,
    pub pending_actions: Arc<Mutex<HashMap<String, RoleAbility>>>,
}

impl GameRunner {
    pub async fn new(game: GameState, event_sender: GameEventSender) -> Arc<Mutex<Self>> {
        let game = Arc::new(Mutex::new(game));

        // Collect all (player_id, night ability role card) pairs into Vec<(String, RoleCard)>
        let mut all_abilities: Vec<(String, RoleCard)> = {
            let g = game.lock().await;
            g.players
                .iter()
                .filter_map(|(id, player)| {
                    if let Some(_night_ability) =
                        player.get_original_role_card().night_ability.as_ref()
                    {
                        Some((id.clone(), (*player.get_original_role_card()).clone()))
                    } else {
                        None
                    }
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

    // pub async fn submit_action(
    //     &self,
    //     player_id: String,
    //     ability: RoleAbilitySpec,
    //     targets: Vec<ActionTarget>,
    // ) -> Result<(), String> {
    //     let mut pending = self.pending_actions.lock().await;
    //     if pending.contains_key(&player_id) {
    //         return Err("Action already submitted".into());
    //     }
    //     pending.insert(player_id.clone(), (ability.clone(), targets));
    //     drop(pending);

    //     self.play_ability(&player_id, PlayableAbility::NightAbility)
    //         .await?;
    //     Ok(())
    // }

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
        let workflow = { self.game.lock().await.workflow.clone() };
        let instance = workflow
            .process_action(player_id, args)
            .await
            .map_err(|e| format!("Workflow processing error: {}", e))?;

        self.update_workflow(player_id, instance).await;
        Ok(())
    }

    pub async fn play_ability(&self, player_id: &str, kind: PlayableAbility) -> Result<(), String> {
        let action = {
            let mut pending = self.pending_actions.lock().await;
            pending.remove(player_id)
        };

        if let Some(ability) = action {
            let ctx = RoleContext::new(Arc::clone(&self.game), player_id.to_string());
            if let Some(workflow_definition_with_input) = (&ability)(ctx).await {
                let workflow = self
                    .game
                    .lock()
                    .await
                    .start_workflow(player_id, workflow_definition_with_input)
                    .await
                    .expect("hmm workflow problems");
                self.event_sender
                    .send(GameEvent::UpdateWorkflow {
                        player_id: player_id.to_string(),
                        workflow,
                    })
                    .ok();
            }

            self.event_sender
                .send(GameEvent::AbilityExecuted {
                    player_id: player_id.to_string(),
                })
                .ok();
        }

        Ok(())
    }

    pub async fn register_cards(&self) {
        let all_cards = {
            let game = self.game.lock().await;
            let mut all_cards: Vec<_> = game
                .players
                .iter()
                .map(|p| p.1.get_original_role_card())
                .collect();
            all_cards.extend(game.middles.iter().map(|p| p.1.get_original_role_card()));
            all_cards
        };
        for player in all_cards.iter() {
            if let Some(register) = &player.register {
                println!("registering {}", player.name);
                (register)(self.game.clone()).await;
            }
        }
    }

    pub async fn run(runner: Arc<Mutex<Self>>) {
        {
            // Register cards up front — safe
            let runner_guard = runner.lock().await;
            runner_guard.register_cards().await;
        }

        loop {
            // STEP 1: Pop stage
            let (player_id, ability, game_arc) = {
                let mut guard = runner.lock().await;
                match guard.stages.pop_front() {
                    Some((pid, ab)) => (pid.clone(), ab.clone(), Arc::clone(&guard.game)),
                    None => return,
                }
            };

            println!("⏳ It's {}'s turn: {}", player_id, ability.name);

            // STEP 2: Check condition and set context — minimal lock time
            let (should_execute, duration, ctx) = {
                let mut game = game_arc.lock().await;
                let ctx = RoleContext::new(Arc::clone(&game_arc), player_id.clone());
                // let should = match &ability.condition {
                //     Some(cond) => cond(&*game),
                //     None => true,
                // };
                let duration = Duration::from_secs(1 as u64);

                // if should {
                game.set_context(player_id.clone(), ctx.clone()).await;
                // }

                (true, duration, ctx)
            };

            if !should_execute {
                println!("❌ Skipping {} (conditions not met)", player_id);
                continue;
            }

            // STEP 3: Emit TurnStarted
            {
                runner
                    .lock()
                    .await
                    .event_sender
                    .send(GameEvent::TurnStarted {
                        player_id: player_id.clone(),
                        role: ability.clone(),
                    })
                    .ok();
            }

            // STEP 4: Generate workflow input (no locks held)

            let mut workflow_input = None;
            if let Some(ability) = &ability.night_ability {
                workflow_input = (ability)(ctx.clone()).await;
            }

            // STEP 5: Start workflow if needed
            if let Some(input) = workflow_input {
                let mut game = game_arc.lock().await;
                let wf = game
                    .start_workflow(&player_id, input)
                    .await
                    .expect("workflow start failed");

                runner
                    .lock()
                    .await
                    .event_sender
                    .send(GameEvent::UpdateWorkflow {
                        player_id: player_id.clone(),
                        workflow: wf,
                    })
                    .ok();
            }

            // STEP 6: Sleep with no locks
            println!(
                "🔔 Waiting {}s for {} to act...",
                duration.as_secs(),
                player_id
            );
            sleep(duration).await;

            // STEP 7: Emit TurnExpired
            {
                runner
                    .lock()
                    .await
                    .event_sender
                    .send(GameEvent::TurnExpired {
                        player_id: player_id.clone(),
                    })
                    .ok();
            }
        }
    }

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
                                    json!({"type": "Player", "Player": {"id": "seer"}}),
                                );
                                println!("sending input back");
                                ProcessWorkflowActionArgs::new(
                                    workflow.instance_id,
                                    "next".into(),
                                    input,
                                )
                            }
                            "prompt_player_reveal" => {
                                // Simulate selecting a card to view
                                let input = HashMap::new();
                                println!("want to send next again");
                                ProcessWorkflowActionArgs::new(
                                    workflow.instance_id,
                                    "next".into(),
                                    input,
                                )
                            }
                            _ => continue,
                        };

                        let runner_clone = Arc::clone(&runner_inner);
                        tokio::spawn(async move {
                            println!("runner {:?}", runner_clone);
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
