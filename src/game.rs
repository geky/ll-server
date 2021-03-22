//! game trait

use serde::{Deserialize, Serialize};
use actix::prelude::*;

#[derive(Debug, Message, Serialize, Deserialize, Clone)]
#[serde(rename_all="snake_case")]
#[rtype(result="()")]
pub struct GameState(pub serde_json::Value);

#[derive(Debug, Message, Serialize, Deserialize, Clone)]
#[serde(rename_all="snake_case")]
#[rtype(result="()")]
pub struct GameAction(pub serde_json::Value);

pub trait Game: Send + std::fmt::Debug {
    // extra info for users
    fn status(&self) -> String;

    // get game state, automatically broadcasted on
    // successful actions
    fn state(&self) -> GameState;

    fn ended(&self) -> bool {
        false
    }

    // take an action, may error
    fn action(
        &mut self,
        action: GameAction,
    ) -> Result<(), Box<dyn std::error::Error>>;
}
