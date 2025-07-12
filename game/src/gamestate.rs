use std::collections::HashMap;
use std::sync::Arc;

use futures::lock::Mutex;

use crate::{
    error::AppResult,
    kafka::service::KafkaService,
    roles::{RoleAbilitySpec, RoleCard, WorkflowDefinitionWithInput},
    workflow::{
        manager::WorkflowManager,
        service::{WorkflowResource, WorkflowService},
    },
};

#[derive(Clone, Debug)]
pub struct Player {
    pub id: String,
    pub name: String,
    pub role_card: Arc<RoleCard>,
    pub copied_role_card: Option<Arc<RoleCard>>,
    pub is_alive: bool,
}
impl Player {
    pub fn new(id: &str, name: &str, role_card: Arc<RoleCard>) -> Player {
        Player {
            id: id.to_owned(),
            name: name.to_owned(),
            role_card,
            copied_role_card: None,
            is_alive: true,
        }
    }
    pub fn effective_role_card(&self) -> Arc<RoleCard> {
        self.copied_role_card
            .clone()
            .unwrap_or_else(|| self.role_card.clone())
    }

    pub fn get_night_ability(&self) -> Option<RoleAbilitySpec> {
        if let Some(copied) = &self.copied_role_card {
            copied.night_ability.clone()
        } else {
            self.role_card.night_ability.clone()
        }
    }
}

#[derive(Clone, Debug)]
pub enum ActionTarget {
    Player(String),
    CenterCard(usize),
}

#[derive(Clone, Debug)]
pub struct RoleContext {
    pub game: Arc<Mutex<GameState>>,
    pub actor: String,
    pub targets: Vec<ActionTarget>,
}

impl RoleContext {
    /// Create a new RoleContext
    pub fn new(
        game: Arc<Mutex<GameState>>,
        actor: impl Into<String>,
        targets: Vec<ActionTarget>,
    ) -> Self {
        Self {
            game,
            actor: actor.into(),
            targets,
        }
    }

    /// Get a reference to the actor player from the game
    pub async fn get_player(&self) -> Option<Player> {
        let game = self.game.lock().await;
        game.players.get(&self.actor).cloned()
    }

    /// Return a new RoleContext with updated targets
    pub fn with_targets(&self, new_targets: Vec<ActionTarget>) -> Self {
        Self {
            game: Arc::clone(&self.game),
            actor: self.actor.clone(),
            targets: new_targets,
        }
    }

    /// Return a clone of the shared game Arc
    pub fn get_game(&self) -> Arc<Mutex<GameState>> {
        Arc::clone(&self.game)
    }
}

#[derive(Debug, Clone)]
pub struct GameState {
    pub players: HashMap<String, Player>,
    pub middles: HashMap<String, Player>,
    pub workflow: Arc<WorkflowService>,
}

impl GameState {
    pub async fn new(players: Vec<Player>, middles: Vec<Player>, kafka: Arc<KafkaService>) -> Self {
        let mut middles_map = HashMap::new();
        for player in middles {
            middles_map.insert(player.id.clone(), player);
        }
        let mut map = HashMap::new();
        for player in players {
            map.insert(player.id.clone(), player);
        }
        GameState {
            players: map,
            middles: middles_map,
            workflow: Arc::new(WorkflowService::new(kafka).await),
        }
    }

    pub async fn start_workflow(
        &mut self,
        player_id: &str,
        config: WorkflowDefinitionWithInput,
    ) -> AppResult<WorkflowResource> {
        let workflow_definition_id = self
            .workflow
            .register_workflow_definition("bot", config.definition)
            .await?;

        let resource = self
            .workflow
            .start_workflow(&workflow_definition_id, player_id, config.input)
            .await?;

        Ok(resource)
    }
}
