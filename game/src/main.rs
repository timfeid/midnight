use std::sync::Arc;
use tokio::sync::Mutex;

use crate::{
    gamestate::{ActionTarget, GameState, Player, RoleContext},
    roles::{doppelganger_card, seer_card, villager_card, werewolf_card, witch_card},
    workflow::InputType,
};

pub mod error;
pub mod gamerunner;
pub mod gamestate;
mod kafka;
pub mod roles;
pub mod workflow;

use std::collections::HashMap;

use serde_json::json;
use tokio::sync::broadcast;

use crate::{
    gamerunner::{GameEvent, GameRunner},
    kafka::service::KafkaService,
    workflow::service::ProcessWorkflowActionArgs,
};

#[tokio::main]
async fn main() {
    let mut seer = seer_card();
    let mut dopple = doppelganger_card();
    let mut witch = witch_card();
    let villager1 = villager_card();
    let werewolf = werewolf_card();
    let villager2 = villager_card();

    let players = vec![
        Player::new("dopple", "Dopple Dan", Arc::new(dopple)),
        Player::new("witch", "Witch Wanda", Arc::new(witch)),
        Player::new("werewolf", "Vince", Arc::new(werewolf.clone())),
        Player::new("villager2", "Violet", Arc::new(villager2)),
        Player::new("seer", "Seer Sam", Arc::new(seer)),
    ];

    let middles = vec![
        Player::new("middle1", "middle 1", Arc::new(villager1.clone())),
        Player::new("middle2", "middle 2", Arc::new(villager1.clone())),
        Player::new("middle3", "middle 3", Arc::new(villager1.clone())),
    ];

    let state = GameState::new(players, middles).await;
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
                    if !workflow.waiting {
                        for input in workflow.inputs.iter() {
                            let runner_clone_inner = Arc::clone(&runner_inner);
                            let player_id = player_id.clone();
                            let workflow_instance_id = workflow.instance_id.clone();
                            if let InputType::ServerActionLoader { target } = &input.input_type {
                                let target = target.clone();
                                println!("Sending back a server loader {target}");
                                tokio::spawn(async move {
                                    runner_clone_inner
                                        .lock()
                                        .await
                                        .process_workflow_action(
                                            &player_id,
                                            ProcessWorkflowActionArgs::new(
                                                workflow_instance_id,
                                                target.into(),
                                                HashMap::new(),
                                            ),
                                        )
                                        .await
                                        .expect("workflow action failed");
                                });
                                // Now you can use the `target` variable here
                                // handle the case when found, e.g., println!("Found target: {:?}", target);
                            }
                        }
                    }

                    if &player_id == "witch" {
                        println!(
                            "🔁 Workflow updated for witch {} : {:?}",
                            player_id, workflow
                        );
                    }

                    if &player_id == "werewolf" {
                        let args = match workflow.current_node_id.as_str() {
                            "select_card_node" => {
                                // Simulate selecting a card to view
                                let mut input = HashMap::new();
                                input.insert(
                                    "selected_card".to_string(),
                                    json!({"type": "Middle", "Middle": {"id": "middle1"}}),
                                );
                                ProcessWorkflowActionArgs::new(
                                    workflow.instance_id.clone(),
                                    "next".into(),
                                    input,
                                )
                            }
                            _ => continue,
                        };

                        let runner_clone = Arc::clone(&runner_inner);
                        let player_id = player_id.clone();
                        tokio::spawn(async move {
                            runner_clone
                                .lock()
                                .await
                                .process_workflow_action(&player_id, args)
                                .await
                                .expect("workflow action failed");
                        });
                    }
                    if &workflow.workflow_id == "user-bot-wf-seer_ability_workflow" {
                        let args = match workflow.current_node_id.as_str() {
                            "select_card_node" => {
                                // Simulate selecting a card to view
                                let mut input = HashMap::new();
                                input.insert(
                                    "selected_card".to_string(),
                                    json!({"type": "Player", "Player": {"id": "witch"}}),
                                );
                                println!("sending input back");
                                ProcessWorkflowActionArgs::new(
                                    workflow.instance_id.clone(),
                                    "next".into(),
                                    input,
                                )
                            }
                            "prompt_player_reveal" => {
                                // Simulate selecting a card to view
                                let input = HashMap::new();
                                println!("want to send next again");
                                ProcessWorkflowActionArgs::new(
                                    workflow.instance_id.clone(),
                                    "next".into(),
                                    input,
                                )
                            }
                            _ => continue,
                        };

                        let runner_clone = Arc::clone(&runner_inner);
                        tokio::spawn(async move {
                            println!("processed action {:?}", args);
                            runner_clone
                                .lock()
                                .await
                                .process_workflow_action(&player_id, args)
                                .await
                                .expect("workflow action failed");
                        });
                    }
                }

                _ => {}
            }
        }
    });

    GameRunner::run(runner.clone()).await;
}
