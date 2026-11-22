pub mod client;
pub mod payload;
pub mod signature;
pub mod webhook;

pub use client::WhatsAppClient;
pub use payload::{IncomingMessage, MessageContent, WebhookPayload};

/// Button ids sent with the Yes/No confirmation message and received back when tapped.
pub const BTN_CONFIRM_YES: &str = "confirm_yes";
pub const BTN_CONFIRM_NO: &str = "confirm_no";