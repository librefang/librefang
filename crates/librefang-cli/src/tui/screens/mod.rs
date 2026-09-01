pub mod agents;
pub mod audit;
// The `channels` screen is back (#8044), rebuilt against the sidecar
// endpoints that survived #5463 rather than the per-channel REST routes it
// originally drove. It manages `[[sidecar_channels]]` instances, so it can
// show several instances of one adapter type.
pub mod channels;
pub mod chat;
pub mod comms;
pub mod dashboard;
pub mod extensions;
pub mod free_provider_guide;
pub mod groups;
pub mod hands;
pub mod init_wizard;
pub mod logs;
pub mod memory;
pub mod models;
pub mod peers;
pub mod security;
pub mod sessions;
pub mod settings;
pub mod skills;
pub mod templates;
pub mod triggers;
pub mod usage;
pub mod welcome;
pub mod wizard;
pub mod workflows;
