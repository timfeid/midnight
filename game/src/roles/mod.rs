use std::pin::Pin;
use std::sync::Arc;
use std::{collections::HashMap, future::Future};

use futures::lock::Mutex;
use serde::{Deserialize, Serialize};

use crate::gamestate::{GameState, RoleContext};

pub mod seer;
pub mod spy;
pub mod werewolf;
pub mod witch;

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

pub fn villager_card() -> RoleCard {
    RoleCard {
        priority: 0,
        register: None,
        name: "Villager".to_string(),
        night_ability: None,
        alliance: Alliance::Villager,
    }
}

pub fn doppelganger_card() -> RoleCard {
    RoleCard {
        priority: 5,
        alliance: Alliance::Villager,
        register: None,
        name: "Doppelgänger".to_string(),
        night_ability: Some(Arc::new(|ctx: RoleContext| Box::pin(async move { None }))),
    }
}
