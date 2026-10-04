//! Review a diff in a Herdr pane and send the comments to an agent.

pub mod agent_delivery;
pub mod edit_keys;
pub mod herdr;
pub mod open;
pub mod spike;
pub mod tui;
pub mod width;

// Waits for the TUI loop, which checks its flag on every tick.
#[allow(dead_code, reason = "used once the TUI loop exists")]
mod termination;
