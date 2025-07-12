use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashMap;

// pub(crate) mod bot;
pub(crate) mod manager;
pub(crate) mod server_action;
pub mod service;

#[derive(Type, Debug, Clone, Serialize, Deserialize)]
pub enum CardFilter {
    PlayerOnly { allow_self: bool },
    MiddleOnly,
    PlayerOrMiddle { allow_self: bool },
}

#[derive(Type, Debug, Clone, Serialize, Deserialize)]
pub enum InputType {
    SelectCard { filter: CardFilter },
}

#[derive(Type, Debug, Clone, Serialize, Deserialize)]
pub struct SelectOption {
    pub value: String,
    pub label: String,
}

#[derive(Type, Debug, Clone, Serialize, Deserialize)]
pub enum CardLayout {
    Image {
        image_key: String,
    },
    ActionImage {
        image_key: String,
        update_responses_key: String,
        from_input_key: String,
        action: String,
    },
}

#[derive(Hash, Eq, PartialEq, Type, Debug, Clone, Serialize, Deserialize)]
pub enum Breakpoint {
    Sm,
    Md,
    Lg,
    Xl,
    Xxl,
}

#[derive(Hash, Eq, PartialEq, Type, Debug, Clone, Serialize, Deserialize)]
pub enum FlexDirection {
    Col,
    Row,
}

#[derive(Type, Debug, Clone, Serialize, Deserialize)]
pub struct SrcSet {
    value_key: String,
    media_key: String,
}

#[derive(Type, Debug, Clone, Serialize, Deserialize)]
pub struct TableHeader {
    pub label: String,
    pub key: String,
    pub width: Option<String>,
    pub align: Option<String>,
}

#[derive(Type, Debug, Clone, Serialize, Deserialize)]
pub enum DisplayType {
    Progress {
        progress_percentage_key: String,
        progress_status_key: String,
        progress_variant_key: String,
    },
    Badge {
        variant: String,
        text_key: String,
    },
    Text {
        text_key: String,
    },
    Page {
        title_key: String,
        content: Vec<DisplayType>,
    },
    Card {
        title_key: String,
        content: Vec<DisplayType>,
        layout: CardLayout,
    },
    CoverImage {
        src_set: Vec<SrcSet>,
        src_key: String,
        display_height: Option<String>,
        display_width: Option<String>,
        alt_key: String,
    },
    Image {
        src_set: Vec<SrcSet>,
        src_key: String,
        display_width: String,
        display_height: String,
        alt_key: String,
    },
    GridList {
        items_key: String,
        r#as: String,
        layout: HashMap<Breakpoint, u8>,
        content: Vec<DisplayType>,
    },
    Flex {
        direction: FlexDirection,
        content: Vec<DisplayType>,
    },
    Carousel {
        items_key: String,
        r#as: String,
        content: Vec<DisplayType>,
    },
    Table {
        items_key: String,
        r#as: String,
        headers: Vec<TableHeader>,
        row: Vec<DisplayType>,
    },
}

#[derive(Type, Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDisplay {
    pub id: String,
    pub display_type: DisplayType,
}

#[derive(Type, Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowInput {
    pub id: String,
    pub label: String,
    pub input_type: InputType,
    pub default_value: Option<serde_json::Value>,
    pub required: bool,
    pub width: Option<String>,
}

#[derive(Type, Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowAction {
    pub id: String,
    pub label: String,
    pub action_type: ActionType,
    pub target: Option<String>,
    pub style: Option<String>,
}

#[derive(Type, Debug, Clone, Serialize, Deserialize)]
pub enum ActionType {
    NextNode,
    PreviousNode,
    Submit,
    Cancel,
    RunServerAction,
    StartWorkflow,
}

#[derive(Type, Debug, Clone, Serialize, Deserialize)]
pub enum NodeCondition {
    // Check if response field exists
    ResponseExists(String),
    // Check if response field equals value
    ResponseEquals {
        field: String,
        value: serde_json::Value,
    },
    // Check if response list has items
    ResponseListNotEmpty(String),
    // Always true
    Always,
}

#[derive(Type, Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowNode {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub displays: Vec<WorkflowDisplay>,
    pub inputs: Vec<WorkflowInput>,
    pub actions: Vec<WorkflowAction>,
    pub layout: Option<String>,
    pub condition: Option<NodeCondition>,
    pub parent_id: Option<String>,
}

#[derive(Type, Debug, Clone, Serialize, Deserialize)]
pub struct CreateWorkflowDefinition {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub initial_node_id: String,
    pub nodes: HashMap<String, WorkflowNode>,
    pub responses: HashMap<String, serde_json::Value>,
    pub server_actions: HashMap<String, ServerActionDefinition>,
}

#[derive(Type, Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    pub id: String,
    pub owner_id: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub responses: HashMap<String, serde_json::Value>,
    pub initial_node_id: String,
    pub nodes: HashMap<String, WorkflowNode>,
    pub server_actions: HashMap<String, ServerActionDefinition>,
}

#[derive(Type, Debug, Clone, Serialize, Deserialize)]
pub struct ServerActionDefinition {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowState {
    pub workflow_id: String,
    pub instance_id: String,
    pub user_id: String,
    pub current_node_id: String,
    pub node_history: Vec<String>,
    pub responses: HashMap<String, serde_json::Value>,
    pub message_id: Option<String>,
    pub completed: bool,
    pub complete_message: Option<String>,
    #[serde(skip_serializing, skip_deserializing)]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[serde(skip_serializing, skip_deserializing)]
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserWorkflowPreferences {
    pub user_id: String,
    pub workflow_id: String,
    pub saved_responses: HashMap<String, serde_json::Value>,
    #[serde(skip_serializing, skip_deserializing)]
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
