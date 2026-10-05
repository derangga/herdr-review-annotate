//! Review a diff in a Herdr pane and send the comments to an agent.

pub mod agent;
pub mod agent_delivery;
pub mod apply;
pub mod cli;
pub mod comment;
pub mod diff;
pub mod edit_keys;
pub mod env;
pub mod herdr;
pub mod keymap;
pub mod meta;
pub mod open;
pub mod spike;
pub mod store;
pub mod tui;
pub mod width;

// Waits for the TUI loop, which checks its flag on every tick.
#[allow(dead_code, reason = "used once the TUI loop exists")]
mod termination;
