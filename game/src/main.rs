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
async fn main() {
    let players = vec![
        Player {
            id: "seer".into(),
            name: "Seer Sam".into(),
            role_card: Arc::new(seer_card()),
            copied_role_card: None,
            is_alive: true,
        },
        Player {
            id: "witch".into(),
            name: "Witch Wendy".into(),
            role_card: Arc::new(witch_card()),
            copied_role_card: None,
            is_alive: true,
        },
        Player {
            id: "wolf".into(),
            name: "Lonely Larry".into(),
            role_card: Arc::new(werewolf_card()),
            copied_role_card: None,
            is_alive: true,
        },
        Player {
            id: "dopple".into(),
            name: "Joe".into(),
            role_card: Arc::new(doppelganger_card()),
            copied_role_card: None,
            is_alive: true,
        },
        Player {
            id: "villager".into(),
            name: "Vince".into(),
            role_card: Arc::new(villager_card()),
            copied_role_card: None,
            is_alive: true,
        },
    ];

    let game = Arc::new(Mutex::new(GameState::new(players)));

    let dopple_ability = {
        let state = game.lock().await;
        let dopple = &state.players["dopple"];
        dopple.role_card.night_ability.clone()
    };
    let context = RoleContext {
        actor: "dopple".into(),
        targets: vec![ActionTarget::Player("seer".into())],
        game: Arc::clone(&game),
    };
    (dopple_ability.unwrap().ability)(context).await;
}

mod tests {
    use std::sync::Arc;
    use tokio::sync::Mutex;

    use super::{
        gamestate::{ActionTarget, GameState, Player, RoleContext},
        roles::{doppelganger_card, seer_card, villager_card, werewolf_card, witch_card},
    };

    #[tokio::test]
    async fn test_doppelganger_clones_and_acts() {
        // Setup game state
        let players = vec![
            Player {
                id: "seer".into(),
                name: "Seer Sam".into(),
                role_card: Arc::new(seer_card()),
                copied_role_card: None,
                is_alive: true,
            },
            Player {
                id: "witch".into(),
                name: "Witch Wendy".into(),
                role_card: Arc::new(witch_card()),
                copied_role_card: None,
                is_alive: true,
            },
            Player {
                id: "wolf".into(),
                name: "Lonely Larry".into(),
                role_card: Arc::new(werewolf_card()),
                copied_role_card: None,
                is_alive: true,
            },
            Player {
                id: "dopple".into(),
                name: "Joe".into(),
                role_card: Arc::new(doppelganger_card()),
                copied_role_card: None,
                is_alive: true,
            },
            Player {
                id: "villager".into(),
                name: "Vince".into(),
                role_card: Arc::new(villager_card()),
                copied_role_card: None,
                is_alive: true,
            },
        ];

        let game = Arc::new(Mutex::new(GameState::new(players)));

        // Stage 1: Doppelgänger copies Seer
        let dopple_ability = {
            let state = game.lock().await;
            let dopple = &state.players["dopple"];
            dopple.role_card.night_ability.clone()
        };

        let context = RoleContext {
            actor: "dopple".into(),
            targets: vec![ActionTarget::Player("seer".into())],
            game: Arc::clone(&game),
        };
        (dopple_ability.unwrap().ability)(context).await;

        // Doppelgänger should now have copied Seer
        let copied_role = {
            let state = game.lock().await;
            state
                .players
                .get("dopple")
                .unwrap()
                .copied_role_card
                .as_ref()
                .unwrap()
                .clone()
        };

        assert_eq!(copied_role.name, "Seer");

        // Stage 2: Doppelgänger (as Seer) uses Seer's ability
        let seer_like_ability = copied_role.night_ability.clone();
        let context = RoleContext {
            actor: "dopple".into(),
            targets: vec![ActionTarget::Player("villager".into())],
            game: Arc::clone(&game),
        };
        (seer_like_ability.unwrap().ability)(context).await;

        // Stage 3: Seer uses their ability
        let seer_ability = {
            let state = game.lock().await;
            let seer = &state.players["seer"];
            seer.role_card.night_ability.clone()
        };
        let context = RoleContext {
            actor: "seer".into(),
            targets: vec![ActionTarget::Player("witch".into())],
            game: Arc::clone(&game),
        };
        (seer_ability.unwrap().ability)(context).await;

        // Stage 4: Werewolf (if solo) acts
        let wolf_ability = {
            let state = game.lock().await;
            let wolf = &state.players["wolf"];
            wolf.role_card.night_ability.clone()
        };
        let context = RoleContext {
            actor: "wolf".into(),
            targets: vec![ActionTarget::Player("seer".into())],
            game: Arc::clone(&game),
        };
        (wolf_ability.unwrap().ability)(context).await;

        // Stage 5: Witch redirects
        let witch_ability = {
            let state = game.lock().await;
            let witch = &state.players["witch"];
            witch.role_card.night_ability.clone()
        };
        let context = RoleContext {
            actor: "witch".into(),
            targets: vec![
                ActionTarget::Player("seer".into()),
                ActionTarget::Player("villager".into()),
            ],
            game: Arc::clone(&game),
        };
        (witch_ability.unwrap().ability)(context).await;

        // Final assert or logging would go here to verify game behavior
    }
}
