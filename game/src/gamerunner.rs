use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use futures::lock::Mutex;
use serde_json::json;
use tokio::time::sleep;

use tokio::sync::broadcast;

use crate::gamestate::{ActionTarget, GameState, RoleContext};
use crate::roles::{RoleAbility, RoleAbilitySpec, RoleCard};
use crate::workflow::manager::WorkflowEvent;
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

        let runner = Arc::new(Mutex::new(Self {
            game: game.clone(),
            stages,
            event_sender,
            pending_actions: Arc::new(Mutex::new(HashMap::new())),
        }));

        {
            let game = game.lock().await;
            let _workflow_inner = Arc::clone(&game.workflow);

            let mut event_manager = game.workflow.manager.event_manager.lock().await;

            let runner_inner = Arc::clone(&runner);
            event_manager.on_event(Box::new(move |event| {
                let event = event.clone();
                let runner_inner = runner_inner.clone();
                Box::pin(async move {
                    match &event {
                        WorkflowEvent::WorkflowUpdated { resource } => {
                            runner_inner
                                .lock()
                                .await
                                .event_sender
                                .send(GameEvent::UpdateWorkflow {
                                    player_id: resource.user_id.clone(),
                                    workflow: resource.clone(),
                                })
                                .ok();
                        }
                        WorkflowEvent::WorkflowStarted { resource } => {
                            runner_inner
                                .lock()
                                .await
                                .event_sender
                                .send(GameEvent::UpdateWorkflow {
                                    player_id: resource.user_id.clone(),
                                    workflow: resource.clone(),
                                })
                                .ok();
                        }
                        _ => {}
                    }
                })
            }));
        }

        runner
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
        println!("hi");
        workflow
            .process_action(player_id, args)
            .await
            .map_err(|e| format!("Workflow processing error: {}", e))
    }

    pub async fn play_ability(&self, player_id: &str, kind: PlayableAbility) -> Result<(), String> {
        let action = {
            let mut pending = self.pending_actions.lock().await;
            pending.remove(player_id)
        };

        if let Some(ability) = action {
            let ctx = RoleContext::new(Arc::clone(&self.game), player_id.to_string());
            if let Some(workflow_definition_with_input) = (&ability)(ctx).await {
                self.game
                    .lock()
                    .await
                    .workflow
                    .manager
                    .start_workflow(
                        &workflow_definition_with_input.definition,
                        player_id,
                        workflow_definition_with_input.input,
                    )
                    .await
                    .expect("hmm workflow problems");
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
        let all_cards = self.game.lock().await.all_cards();
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
        println!("beforeloop {:?}", runner);

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
                let workflow = game_arc.lock().await.workflow.clone();
                workflow
                    .manager
                    .start_workflow(&input.definition, &player_id, input.input)
                    .await
                    .expect("workflow start failed");
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
