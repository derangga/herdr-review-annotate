//! `spike-send`: send a fixed two-line prompt to the focused agent, to try delivery by hand.

use std::process::ExitCode;

use crate::agent_delivery::{Delivery, deliver_to_agent};
use crate::herdr::{notify, run_herdr_output};
use crate::open::parse_context;

const PROMPT: &str = "Spike prompt from herdr-review.\nReply with the single word: received.";

pub fn send() -> ExitCode {
    let context = parse_context(std::env::var("HERDR_PLUGIN_CONTEXT_JSON").ok().as_deref());
    let result = deliver_to_agent(Delivery::Send, context.pane.as_deref(), PROMPT, |args| {
        run_herdr_output(args)
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            notify("review: spike-send refused", Some(&message));
            ExitCode::FAILURE
        }
    }
}
