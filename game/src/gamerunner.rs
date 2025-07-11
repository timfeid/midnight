use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;
use tokio::time::sleep;

use tokio::sync::broadcast;

use crate::gamestate::{ActionTarget, GameState, RoleContext};
use crate::roles::RoleAbilitySpec;
use crate::workflow::service::WorkflowResource;
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
        drop(pending); // release the lock early

        // Immediately execute the action
        self.play_ability(&player_id, PlayableAbility::NightAbility)
            .await?;

        Ok(())
    }

    pub async fn play_ability(&self, player_id: &str, kind: PlayableAbility) -> Result<(), String> {
        let action = {
            let mut pending = self.pending_actions.lock().await;
            pending.remove(player_id)
        };

        if let Some((ability, targets)) = action {
            let ctx = RoleContext::new(Arc::clone(&self.game), player_id.to_string(), targets);
            (&ability.ability)(ctx).await;

            self.event_sender
                .send(GameEvent::AbilityExecuted {
                    player_id: player_id.to_string(),
                    description: ability.description.clone(),
                })
                .ok();
        }

        Ok(())
    }

    pub async fn run(runner: Arc<Mutex<Self>>) {
        loop {
            let (player_id, ability) = {
                let mut guard = runner.lock().await;
                match guard.stages.pop_front() {
                    Some(stage) => stage,
                    None => return,
                }
            };

            println!("⏳ It's {}'s turn: {}", player_id, ability.description);

            runner
                .lock()
                .await
                .event_sender
                .send(GameEvent::TurnStarted {
                    player_id: player_id.clone(),
                    ability: ability.clone(),
                })
                .ok();

            let ctx = RoleContext::new(
                Arc::clone(&runner.lock().await.game),
                player_id.clone(),
                vec![],
            );

            if !runner.lock().await.should_execute(&ctx, &ability).await {
                println!("❌ Skipping {} (conditions not met)", player_id);
                continue;
            }

            let duration = Duration::from_secs(ability.duration_secs as u64);
            println!(
                "🔔 Waiting {}s for {} to act...",
                ability.duration_secs, player_id
            );
            runner.lock().await.wait_for_turn(duration).await;

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
    use super::*;
    use crate::gamestate::Player;
    use crate::roles::{doppelganger_card, seer_card, villager_card, witch_card};

    #[tokio::test]
    async fn test_runner_with_dopple_and_witch() {
        let mut dopple = doppelganger_card();
        let mut witch = witch_card();
        let villager1 = villager_card();
        let villager2 = villager_card();
        let seer = seer_card();

        if let Some(ability) = dopple.night_ability.as_mut() {
            ability.duration_secs = 1;
        }
        if let Some(ability) = witch.night_ability.as_mut() {
            ability.duration_secs = 1;
        }

        let players = vec![
            Player::new("dopple", "Dopple Dan", Arc::new(dopple)),
            Player::new("witch", "Witch Wanda", Arc::new(witch)),
            Player::new("villager1", "Vince", Arc::new(villager1)),
            Player::new("villager2", "Violet", Arc::new(villager2)),
            Player::new("seer", "Seer Sam", Arc::new(seer)),
        ];

        let state = GameState::new(players);
        let (tx, mut rx) = broadcast::channel(16);
        let runner = GameRunner::new(state, tx.clone()).await;
        let runner_inner = runner.clone();

        tokio::spawn(async move {
            while let Ok(event) = rx.recv().await {
                println!("event: {:?}", event);
                match event {
                    GameEvent::TurnStarted { player_id, ability } if player_id == "dopple" => {
                        let _ = runner_inner
                            .lock()
                            .await
                            .submit_action(
                                player_id.clone(),
                                ability.clone(),
                                vec![ActionTarget::Player("witch".into())],
                            )
                            .await;

                        let _ = runner_inner
                            .lock()
                            .await
                            .play_ability(&player_id, PlayableAbility::NightAbility)
                            .await;
                    }
                    GameEvent::TurnStarted { player_id, ability } if player_id == "witch" => {
                        let _ = runner_inner
                            .lock()
                            .await
                            .submit_action(
                                player_id.clone(),
                                ability.clone(),
                                vec![
                                    ActionTarget::Player("seer".into()),
                                    ActionTarget::Player("villager2".into()),
                                ],
                            )
                            .await;

                        let _ = runner_inner
                            .lock()
                            .await
                            .play_ability(&player_id, PlayableAbility::NightAbility)
                            .await;
                    }
                    GameEvent::TurnStarted { player_id, ability } if player_id == "dopple-seer" => {
                        let _ = runner_inner
                            .lock()
                            .await
                            .submit_action(
                                player_id.clone(),
                                ability.clone(),
                                vec![ActionTarget::Player("seer".into())],
                            )
                            .await;

                        let _ = runner_inner
                            .lock()
                            .await
                            .play_ability(&player_id, PlayableAbility::NightAbility)
                            .await;
                    }
                    _ => {}
                }
            }
        });

        GameRunner::run(runner.clone()).await;
    }
}
