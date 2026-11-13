pub mod config;
pub mod controller;
pub mod core;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub phase: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connected_at: Option<u64>,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            phase: "disconnected".into(),
            message: "Xray is stopped.".into(),
            connected_at: None,
        }
    }
}
