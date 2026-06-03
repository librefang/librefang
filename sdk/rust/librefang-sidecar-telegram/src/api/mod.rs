//! Telegram Bot API client + types + error.

pub mod client;
pub mod error;
pub mod types;

pub use client::BotClient;
pub use error::{Error, Result};
