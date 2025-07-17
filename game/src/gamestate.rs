use std::collections::HashMap;
use std::sync::Arc;

use futures::{future::BoxFuture, lock::Mutex};

use crate::{
    error::AppResult,
    kafka::service::KafkaService,
    roles::{RoleAbility, RoleAbilitySpec, RoleCard, WorkflowDefinitionWithInput},
    workflow::{
        CreateWorkflowDefinition,
        manager::{WorkflowEvent, WorkflowManager},
        server_action::{ServerActionContext, ServerActionHandler, ServerActionResult},
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

    pub fn get_original_role_card(&self) -> Arc<RoleCard> {
        self.role_card.clone()
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
    pub user_id: String,
}

impl RoleContext {
    /// Create a new RoleContext
    pub fn new(game: Arc<Mutex<GameState>>, actor: impl Into<String>) -> Self {
        Self {
            game,
            user_id: actor.into(),
        }
    }

    /// Get a reference to the actor player from the game
    pub async fn get_player(&self) -> Option<Player> {
        let game = self.game.lock().await;
        game.players.get(&self.user_id).cloned()
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
    pub role_contexts: Arc<Mutex<HashMap<String, RoleContext>>>,
}

impl GameState {
    pub async fn new(players: Vec<Player>, middles: Vec<Player>) -> Self {
        let mut middles_map = HashMap::new();
        for player in middles {
            middles_map.insert(player.id.clone(), player);
        }
        let mut map = HashMap::new();
        for player in players {
            map.insert(player.id.clone(), player);
        }

        let workflow_service = WorkflowService::new().await;
        let workflow = Arc::new(workflow_service);

        GameState {
            role_contexts: Arc::new(Mutex::new(HashMap::new())),
            players: map,
            middles: middles_map,
            workflow,
        }
    }

    pub fn all_cards(&self) -> Vec<Arc<RoleCard>> {
        let mut all_cards = {
            let mut all_cards: Vec<_> = self
                .players
                .iter()
                .map(|p| p.1.get_original_role_card())
                .collect();
            all_cards.extend(self.middles.iter().map(|p| p.1.get_original_role_card()));
            all_cards
        };
        all_cards.sort_by_key(|card| card.priority);
        all_cards
    }

    pub async fn register_workflow_definition(
        &self,
        definition: CreateWorkflowDefinition,
    ) -> AppResult<String> {
        self.workflow
            .register_workflow_definition("bot", definition)
            .await
    }
    pub async fn set_context(&self, player_id: String, ctx: RoleContext) {
        self.role_contexts.lock().await.insert(player_id, ctx);
    }

    pub async fn get_context(&self, player_id: &str) -> Option<RoleContext> {
        self.role_contexts.lock().await.get(player_id).cloned()
    }

    pub async fn register_server_action(
        &self,
        action_id: &str,
        handler: ServerActionHandler,
    ) -> AppResult<()> {
        self.workflow
            .register_server_action(action_id, handler)
            .await
    }
}
