use std::collections::HashMap;
use std::sync::Arc;

use futures::lock::Mutex;
use rand::{SeedableRng, seq::IndexedRandom};
use rand_chacha::ChaCha12Rng;
use serde_json::Value;

use crate::{
    error::{AppResult, ServicesError},
    roles::{Alliance, RoleCard},
    workflow::{
        CreateWorkflowDefinition, server_action::ServerActionHandler, service::WorkflowService,
    },
};

#[derive(Clone, Debug)]
pub struct Player {
    pub id: String,
    pub name: String,
    pub role_card: Arc<RoleCard>,
    pub copied_role_card: Option<Arc<RoleCard>>,
    pub is_alive: bool,
    pub middle_position: Option<usize>,
}
impl Player {
    pub fn new(
        id: &str,
        name: &str,
        role_card: Arc<RoleCard>,
        middle_position: Option<usize>,
    ) -> Player {
        Player {
            id: id.to_owned(),
            name: name.to_owned(),
            role_card,
            copied_role_card: None,
            is_alive: true,
            middle_position,
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
    pub async fn get_player(&self) -> AppResult<Player> {
        let game = self.game.lock().await;
        game.players
            .get(&self.user_id)
            .cloned()
            .ok_or(ServicesError::InternalError(format!(
                "no player {}",
                self.user_id
            )))
    }

    /// Return a clone of the shared game Arc
    pub fn get_game(&self) -> Arc<Mutex<GameState>> {
        Arc::clone(&self.game)
    }
}

#[derive(Debug, Clone)]
pub struct GameState {
    pub players: HashMap<String, Player>,
    pub workflow: Arc<WorkflowService>,
    pub role_contexts: Arc<Mutex<HashMap<String, RoleContext>>>,
    sabotaged_inputs: HashMap<(String, String), HashMap<String, Value>>,
}

impl GameState {
    pub async fn get_sabotage_candidates(
        &self,
        exclude_names: &[&str],
        only_name: Option<&str>,
    ) -> Vec<Arc<RoleCard>> {
        self.all_cards()
            .iter()
            .filter(|card| {
                card.night_ability.is_some()
                    && card.alliance != Alliance::Werewolf
                    && !exclude_names.contains(&card.name.as_str())
                    && only_name.map_or(true, |n| card.name == n)
            })
            .cloned()
            .collect()
    }

    pub async fn pick_random_role<'a>(
        &'a self,
        roles: &'a [Arc<RoleCard>],
    ) -> Option<&'a Arc<RoleCard>> {
        let mut rng = ChaCha12Rng::from_os_rng();
        roles.choose(&mut rng)
    }

    pub async fn new(players: Vec<Player>) -> Self {
        let mut map = HashMap::new();
        for player in players {
            map.insert(player.id.clone(), player);
        }

        let workflow_service = WorkflowService::new().await;
        let workflow = Arc::new(workflow_service);

        GameState {
            role_contexts: Arc::new(Mutex::new(HashMap::new())),
            players: map,
            workflow,
            sabotaged_inputs: HashMap::new(),
        }
    }

    pub async fn set_sabotage_inputs(
        &mut self,
        user_id: &str,
        workflow_id: &str,
        inputs: HashMap<String, Value>,
    ) {
        self.sabotaged_inputs
            .insert((user_id.to_string(), workflow_id.to_string()), inputs);
    }

    pub async fn get_sabotage_inputs(
        &self,
        user_id: &str,
        workflow_id: &str,
    ) -> Option<HashMap<String, Value>> {
        self.sabotaged_inputs
            .get(&(user_id.to_string(), workflow_id.to_string()))
            .cloned()
    }

    pub async fn get_user_effective_role(&self, user_id: &str) -> AppResult<Arc<RoleCard>> {
        if let Ok(player) = self.get_player(user_id).await {
            return Ok(player.effective_role_card());
        }

        Err(ServicesError::InternalError(format!(
            "Unable to find player or middle with id {user_id}"
        )))
    }

    pub async fn clear_sabotage_inputs(&mut self, user_id: &str, workflow_id: &str) {
        self.sabotaged_inputs
            .remove(&(user_id.to_string(), workflow_id.to_string()));
    }

    pub fn all_cards(&self) -> Vec<Arc<RoleCard>> {
        let mut all_cards: Vec<Arc<RoleCard>> = {
            self.players
                .iter()
                .map(|p| p.1.get_original_role_card())
                .collect()
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

    pub async fn get_player_by_role(&self, role: &str) -> AppResult<Player> {
        Ok(self
            .players
            .values()
            .into_iter()
            .find(|p| p.effective_role_card().name.as_str() == role)
            .ok_or(ServicesError::InternalError(format!(
                "Unable to find player with role {role}"
            )))?
            .clone())
    }

    pub async fn get_player(&self, player_id: &str) -> AppResult<Player> {
        Ok(self
            .players
            .get(player_id)
            .ok_or(ServicesError::InternalError(format!(
                "Unable to find player with id {player_id}"
            )))?
            .clone())
    }

    pub async fn get_context(&self, player_id: &str) -> AppResult<RoleContext> {
        self.role_contexts
            .lock()
            .await
            .get(player_id)
            .cloned()
            .ok_or(ServicesError::InternalError("something".to_string()))
    }

    pub async fn register_server_action(
        &self,
        action_id: &str,
        handler: ServerActionHandler,
    ) -> AppResult<()> {
        let response = self
            .workflow
            .register_server_action(action_id, handler)
            .await?;

        println!("Registered server action {action_id}");

        Ok(response)
    }
}
