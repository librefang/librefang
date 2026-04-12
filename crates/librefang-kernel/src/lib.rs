//! Core kernel for the LibreFang Agent Operating System.
//!
//! The kernel manages agent lifecycles, memory, permissions, scheduling,
//! and inter-agent communication.

pub mod approval;
pub mod auth;
pub mod auto_reply;
pub mod background;
pub mod capabilities;
pub mod config;
pub mod config_reload;
pub mod cron;
pub mod error;
pub mod event_bus;
pub mod heartbeat;
pub mod inbox;
pub mod kernel;
pub mod mcp_oauth_provider;
pub mod metering;
pub mod orchestration;
pub mod pairing;
pub mod registry;
pub mod router;
pub mod scheduler;
pub mod supervisor;
pub mod triggers;
pub mod whatsapp_gateway;
pub mod wizard;
pub mod workflow;

pub use kernel::DeliveryTracker;
pub use kernel::LibreFangKernel;
