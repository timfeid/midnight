use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::from_value;

use crate::{
    gamestate::{ActionTarget, GameState, RoleContext},
    workflow::{CreateWorkflowDefinition, WorkflowDefinition},
};

pub type RoleAbility = Arc<
    dyn Fn(RoleContext) -> Pin<Box<dyn Future<Output = Option<WorkflowDefinition>> + Send>>
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

#[derive(Clone, Debug)]
pub struct RoleCard {
    pub name: String,
    pub night_ability: Option<RoleAbilitySpec>,
}

#[derive(Clone, Debug)]
pub enum TargetSelector {
    SinglePlayer,
    PlayerAndPlayer,
    None,
}

pub fn seer_card() -> RoleCard {
    RoleCard {
        name: "Seer".to_string(),
        night_ability: Some(RoleAbilitySpec {
            ability: Arc::new(|ctx: RoleContext| {
                Box::pin(async move {
                    if let Some(ActionTarget::Player(pid)) = ctx.targets.get(0) {
                        let state = ctx.game.lock().await;
                        let player = &state.players[pid];

                        println!(
                            "🔮 Seer inspects {}: {}",
                            player.name, player.role_card.name
                        );

                        return Some(serde_json::from_str::<WorkflowDefinition>(
                            r#"{"id":"recipes_workflow","name":"Recipe Finder","description":"Find recipes based on main ingredients","initial_node_id":"ingredient_selection_node","nodes":{"ingredient_selection_node":{"id":"ingredient_selection_node","title":"Choose Your Main Ingredient","description":"Select the main ingredient you want to cook with","displays":[{"id":"recipe_search_base_select","display_type":{"Carousel":{"items_key":"bases","as":"base","content":[{"Card":{"title_key":"base.label","content":[],"layout":{"ActionImage":{"update_responses_key":"base","from_input_key":"base.value","action":"get_recipes","image_key":"recipe_details.image"}}}}]}}}],"inputs":[],"actions":[{"id":"get_recipes","label":"Get recipes","action_type":"RunServerAction","target":"test","style":null}],"layout":null,"condition":"Always","parent_id":null},"recipe_list_node":{"id":"recipe_list_node","title":"Recipe Results","description":"Choose from the recipes we found","displays":[{"id":"receipe_results","display_type":{"Carousel":{"items_key":"recipes","as":"recipe","content":[{"Card":{"title_key":"recipe.title","content":[{"Text":{"text_key":"recipe.description"}}],"layout":{"ActionImage":{"update_responses_key":"selected_recipe_slug","from_input_key":"recipe.slug","action":"get_recipe_details","image_key":"recipe.image"}}}}]}}}],"inputs":[],"actions":[{"id":"get_recipe_details","label":"View Recipe","action_type":"RunServerAction","target":"get_recipe_details","style":"primary"},{"id":"back_to_search","label":"Back to Search","action_type":"PreviousNode","target":null,"style":"secondary"}],"layout":null,"condition":{"ResponseExists":"recipes"},"parent_id":"ingredient_selection_node"},"recipe_details_node":{"id":"recipe_details_node","title":"Recipe Details","description":"Complete recipe information","displays":[{"id":"recipe_card","display_type":{"Page":{"title_key":"recipe_details.title","content":[{"CoverImage":{"src_set":[],"src_key":"recipe_details.image","display_width":null,"display_height":"500","alt_key":"recipe_details.title"}},{"Text":{"text_key":"recipe_details.description"}},{"GridList":{"items_key":"recipe_details.items","as":"item","layout":{"Md":2,"Xl":3},"content":[{"Flex":{"direction":"Row","content":[{"Image":{"src_set":[],"src_key":"item.image","display_width":"70","display_height":"70","alt_key":"item.name"}},{"Flex":{"direction":"Col","content":[{"Text":{"text_key":"item.name"}},{"Text":{"text_key":"item.amount"}}]}}]}}]}}]}}}],"inputs":[{"id":"ingredients_list","label":"Ingredients","input_type":{"Display":{"content":"recipe_details.ingredients"}},"default_value":null,"required":false,"width":"full"},{"id":"instructions","label":"Instructions","input_type":{"Display":{"content":"recipe_details.instructions"}},"default_value":null,"required":false,"width":"full"}],"actions":[{"id":"back_to_recipes","label":"Back to Recipes","action_type":"PreviousNode","target":null,"style":"secondary"},{"id":"save_recipe","label":"Save Recipe","action_type":"NextNode","target":null,"style":"primary"}],"layout":null,"condition":{"ResponseExists":"recipe_details"},"parent_id":"recipe_list_node"},"no_recipes_node":{"id":"no_recipes_node","title":"No Recipes Found","description":"No recipes match your criteria","displays":[],"inputs":[{"id":"no_results_message","label":"Sorry!","input_type":{"Display":{"content":"No recipes found for your selected criteria. Try adjusting your filters."}},"default_value":null,"required":false,"width":"full"}],"actions":[{"id":"try_again","label":"Try Different Ingredients","action_type":"PreviousNode","target":null,"style":"primary"}],"layout":null,"condition":{"ResponseEquals":{"field":"recipes","value":null}},"parent_id":"ingredient_selection_node"},"recipe_saved_node":{"id":"recipe_saved_node","title":"Recipe Saved","description":"Recipe has been saved to your collection","displays":[],"inputs":[{"id":"saved_message","label":"Success!","input_type":{"Display":{"content":"Recipe has been saved to your recipe collection."}},"default_value":null,"required":false,"width":"full"}],"actions":[{"id":"finish","label":"Done","action_type":"Submit","target":null,"style":"primary"},{"id":"find_another","label":"Find Another Recipe","action_type":"NextNode","target":null,"style":"secondary"}],"layout":null,"condition":"Always","parent_id":"recipe_details_node"}},"responses":{"tile_node_id":"recipe_list_node","tile_workflow_id":"recipe-finder-workflow","bases":[{"label":"Chicken","value":"chicken","image":""},{"label":"Beef","value":"beef","image":""},{"label":"Fish","value":"fish","image":""},{"label":"Pork","value":"pork","image":""},{"label":"Lamb","value":"lamb","image":""},{"label":"Clams","value":"clams","image":""},{"label":"Shrimp","value":"shrimp","image":""},{"label":"Crab","value":"crab","image":""},{"label":"Vegetables","value":"vegetables","image":""},{"label":"Pasta","value":"pasta","image":""},{"label":"Rice","value":"rice","image":""},{"label":"Beans","value":"beans","image":""}]},"server_actions":{"get_recipes":{"id":"test","name":"Fetch recipes by ingredient","description":null},"get_recipe_details":{"id":"test","name":"Get detailed recipe information","description":null}}}
                            "#,
                        ).unwrap());
                    }
                    None
                })
            }),
            target_selector: TargetSelector::SinglePlayer,
            validator: None,
            description: "Inspect a player's role.".to_string(),
            priority: 30,
            duration_secs: 10,
            allowed_phases: AbilityPhaseScope::Night,
            allowed_turns: AbilityTurnScope::YourTurn,
            condition: None,
        }),
    }
}

pub fn werewolf_card() -> RoleCard {
    RoleCard {
        name: "Werewolf".to_string(),
        night_ability: Some(RoleAbilitySpec {
            duration_secs: 10,
            ability: Arc::new(|ctx: RoleContext| {
                Box::pin(async move {
                    if let Some(ActionTarget::Player(pid)) = ctx.targets.get(0) {
                        let state = ctx.game.lock().await;
                        let player = &state.players[pid];
                        println!(
                            "🐺 Werewolf inspects {}: {}",
                            player.name, player.role_card.name
                        );
                    }

                    None
                })
            }),
            target_selector: TargetSelector::SinglePlayer,
            validator: Some(Arc::new(|ctx: RoleContext| {
                Box::pin(async move {
                    let game = ctx.game.lock().await;
                    let werewolves = game
                        .players
                        .values()
                        .filter(|p| p.role_card.name == "Werewolf")
                        .count();
                    werewolves == 1
                })
            })),
            description: "Inspect a player (only if solo)".to_string(),
            priority: 40,
            allowed_phases: AbilityPhaseScope::Night,
            allowed_turns: AbilityTurnScope::YourTurn,
            condition: None,
        }),
    }
}

pub fn villager_card() -> RoleCard {
    RoleCard {
        name: "Villager".to_string(),
        night_ability: None,
    }
}

pub fn witch_card() -> RoleCard {
    RoleCard {
        name: "Witch".to_string(),
        night_ability: Some(RoleAbilitySpec {
            duration_secs: 10,
            ability: Arc::new(|ctx: RoleContext| {
                Box::pin(async move {
                    if let (Some(ActionTarget::Player(from)), Some(ActionTarget::Player(to))) =
                        (ctx.targets.get(0), ctx.targets.get(1))
                    {
                        println!("🧙 Witch redirects {}'s action to {}", from, to);
                    }
                    None
                })
            }),
            target_selector: TargetSelector::PlayerAndPlayer,
            validator: None,
            description: "Redirect another player's action.".to_string(),
            priority: 50,
            allowed_phases: AbilityPhaseScope::Night,
            allowed_turns: AbilityTurnScope::YourTurn,
            condition: None,
        }),
    }
}

pub fn doppelganger_card() -> RoleCard {
    RoleCard {
        name: "Doppelgänger".to_string(),
        night_ability: Some(RoleAbilitySpec {
            duration_secs: 15,
            ability: Arc::new(|ctx: RoleContext| {
                Box::pin(async move {
                    if let Some(ActionTarget::Player(pid)) = ctx.targets.get(0) {
                        let mut game = ctx.game.lock().await;
                        // First, clone the role_card arc while we have only immutable access
                        let target_role_card = game.players[pid].role_card.clone();
                        let target_role_card_name = game.players[pid].role_card.name.clone();
                        println!("🌀 Doppelgänger copied {}", target_role_card_name);
                        // Now, do the mutable borrow
                        if let Some(actor) = game.players.get_mut(&ctx.actor) {
                            actor.copied_role_card = Some(target_role_card);
                        }
                    }
                    None
                })
            }),
            target_selector: TargetSelector::SinglePlayer,
            validator: None,
            description: "Copy another player's role.".to_string(),
            priority: 10,
            allowed_phases: AbilityPhaseScope::Night,
            allowed_turns: AbilityTurnScope::YourTurn,
            condition: None,
        }),
    }
}
