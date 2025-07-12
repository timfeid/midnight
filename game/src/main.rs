use std::sync::Arc;
use tokio::sync::Mutex;

use crate::{
    gamestate::{ActionTarget, GameState, Player, RoleContext},
    roles::{doppelganger_card, seer_card, villager_card, werewolf_card, witch_card},
};

pub mod error;
pub mod gamerunner;
pub mod gamestate;
mod kafka;
pub mod roles;
pub mod workflow;

#[tokio::main]
async fn main() {}
