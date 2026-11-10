//! Review a diff in a Herdr pane and send the comments to an agent.

pub mod actions;
pub mod agent;
pub mod agent_delivery;
pub mod apply;
pub mod cards;
pub mod cli;
pub mod comment;
pub mod diff;
pub mod edit_keys;
pub mod editor;
pub mod env;
pub mod hc;
pub mod herdr;
pub mod icons;
pub mod json;
pub mod keymap;
pub mod message;
pub mod message_action;
pub mod message_json;
pub mod message_tui;
pub mod meta;
pub mod motion;
pub mod open;
pub mod send;
pub mod state;
pub mod store;
pub mod syntax;
pub mod theme;
pub mod theme_json;
pub mod tui;
pub mod user;
pub mod view;
pub mod width;
pub mod words;

mod termination;
