//! Heartbeat monitor — detects unresponsive agents for 24/7 autonomous operation.
//!
//! The heartbeat monitor runs as a background tokio task, periodically checking
//! each running agent's `last_active` timestamp. If an agent hasn't been active
//! for longer than 2x its heartbeat interval, a `HealthCheckFailed` event is
//! published to the event bus.

use crate::registry::AgentRegistry;
use chrono::Utc;
use librefang_types::agent::{AgentId, AgentState};
use tracing::debug;

/// Default heartbeat check interval (seconds).
const DEFAULT_CHECK_INTERVAL_SECS: u64 = 30;

/// Multiplier: agent is considered unresponsive if inactive for this many
/// multiples of its heartbeat interval.
const UNRESPONSIVE_MULTIPLIER: u64 = 2;

/// Grace period (seconds): if an agent's `last_active` is within this window
/// of `created_at`, it has never genuinely processed a message and should not
/// be flagged as unresponsive.  This covers the small gap between registration
/// and the initial `set_state(Running)` call.
const IDLE_GRACE_SECS: i64 = 10;

/// Result of a heartbeat check.
#[derive(Debug, Clone)]
pub struct HeartbeatStatus {
    /// Agent ID.
    pub agent_id: AgentId,
    /// Agent name.
    pub name: String,
    /// Seconds since last activity.
    pub inactive_secs: i64,
    /// Whether the agent is considered unresponsive.
    pub unresponsive: bool,
}

/// Heartbeat monitor configuration.
#[derive(Debug, Clone)]
pub struct HeartbeatConfig {
    /// How often to run the heartbeat check (seconds).
    pub check_interval_secs: u64,
    /// Default threshold for unresponsiveness (seconds).
    /// Overridden per-agent by AutonomousConfig.heartbeat_timeout_secs,
    /// or computed from AutonomousConfig.heartbeat_interval_secs * 2.
    pub default_timeout_secs: u64,
    /// How many recent heartbeat turns to keep when pruning session context.
    /// Overridden per-agent by AutonomousConfig.heartbeat_keep_recent.
    pub keep_recent: usize,
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            check_interval_secs: DEFAULT_CHECK_INTERVAL_SECS,
            default_timeout_secs: DEFAULT_CHECK_INTERVAL_SECS * UNRESPONSIVE_MULTIPLIER,
            keep_recent: 10,
        }
    }
}

impl HeartbeatConfig {
    /// Create a `HeartbeatConfig` from the TOML-level `HeartbeatTomlConfig`.
    pub fn from_toml(toml: &librefang_types::config::HeartbeatTomlConfig) -> Self {
        Self {
            check_interval_secs: toml.check_interval_secs,
            default_timeout_secs: toml.default_timeout_secs,
            keep_recent: toml.keep_recent,
        }
    }
}

/// Check all running agents and return their heartbeat status.
///
/// This is a pure function — it doesn't start a background task.
/// The caller (kernel) can run this periodically or in a background task.
pub fn check_agents(registry: &AgentRegistry, config: &HeartbeatConfig) -> Vec<HeartbeatStatus> {
    let now = Utc::now();
    let mut statuses = Vec::new();

    for entry_ref in registry.list() {
        // Only check running agents
        if entry_ref.state != AgentState::Running {
            continue;
        }

        // Skip non-autonomous agents — they are passive (wait for messages)
        // and should not be flagged as unresponsive by the heartbeat monitor.
        if entry_ref.manifest.autonomous.is_none() {
            continue;
        }

        let inactive_secs = (now - entry_ref.last_active).num_seconds();

        // Determine timeout: per-agent heartbeat_timeout_secs > interval*2 > global default
        let timeout_secs = entry_ref
            .manifest
            .autonomous
            .as_ref()
            .and_then(|a| {
                a.heartbeat_timeout_secs
                    .map(|t| t as u64)
                    .or(Some(a.heartbeat_interval_secs * UNRESPONSIVE_MULTIPLIER))
            })
            .unwrap_or(config.default_timeout_secs) as i64;

        // --- Skip idle agents that have never genuinely processed a message ---
        //
        // When an agent is spawned, both `created_at` and `last_active` are set
        // to now. Administrative operations (set_state, etc.) bump `last_active`
        // by a tiny amount. If `last_active` is still within IDLE_GRACE_SECS of
        // `created_at`, the agent was never active beyond its initial startup and
        // should NOT be flagged as unresponsive. This prevents disabled/unused
        // agents from entering an infinite crash-recover loop (upstream openfang #844).
        //
        // Periodic / Hand agents with long schedule intervals (e.g. 3600s) are
        // also covered: they sit idle between ticks and their `last_active` stays
        // near `created_at` until the first tick fires.
        let never_active =
            (entry_ref.last_active - entry_ref.created_at).num_seconds() <= IDLE_GRACE_SECS;

        if never_active {
            debug!(
                agent = %entry_ref.name,
                inactive_secs,
                "Skipping idle agent — never received a message"
            );
            continue;
        }

        let unresponsive = inactive_secs > timeout_secs;

        // Logging is handled by the caller (heartbeat monitor loop) which
        // tracks state transitions to avoid spamming repeated warnings.
        debug!(
            agent = %entry_ref.name,
            inactive_secs,
            timeout_secs,
            unresponsive,
            "Heartbeat check"
        );

        statuses.push(HeartbeatStatus {
            agent_id: entry_ref.id,
            name: entry_ref.name.clone(),
            inactive_secs,
            unresponsive,
        });
    }

    statuses
}

/// Check if an agent is currently within its quiet hours.
///
/// Quiet hours format: "HH:MM-HH:MM" (24-hour format, UTC).
/// Returns true if the current time falls within the quiet period.
pub fn is_quiet_hours(quiet_hours: &str) -> bool {
    let parts: Vec<&str> = quiet_hours.split('-').collect();
    if parts.len() != 2 {
        return false;
    }

    let now = Utc::now();
    let current_minutes = now.format("%H").to_string().parse::<u32>().unwrap_or(0) * 60
        + now.format("%M").to_string().parse::<u32>().unwrap_or(0);

    let parse_time = |s: &str| -> Option<u32> {
        let hm: Vec<&str> = s.trim().split(':').collect();
        if hm.len() != 2 {
            return None;
        }
        let h = hm[0].parse::<u32>().ok()?;
        let m = hm[1].parse::<u32>().ok()?;
        if h > 23 || m > 59 {
            return None;
        }
        Some(h * 60 + m)
    };

    let start = match parse_time(parts[0]) {
        Some(v) => v,
        None => return false,
    };
    let end = match parse_time(parts[1]) {
        Some(v) => v,
        None => return false,
    };

    if start <= end {
        // Same-day range: e.g., 22:00-06:00 would be cross-midnight
        // This is start <= current < end
        current_minutes >= start && current_minutes < end
    } else {
        // Cross-midnight: e.g., 22:00-06:00
        current_minutes >= start || current_minutes < end
    }
}

/// Aggregate heartbeat summary.
#[derive(Debug, Clone, Default)]
pub struct HeartbeatSummary {
    /// Total agents checked.
    pub total_checked: usize,
    /// Number of responsive agents.
    pub responsive: usize,
    /// Number of unresponsive agents.
    pub unresponsive: usize,
    /// Details of unresponsive agents.
    pub unresponsive_agents: Vec<HeartbeatStatus>,
}

/// Produce a summary from heartbeat statuses.
pub fn summarize(statuses: &[HeartbeatStatus]) -> HeartbeatSummary {
    let unresponsive_agents: Vec<HeartbeatStatus> = statuses
        .iter()
        .filter(|s| s.unresponsive)
        .cloned()
        .collect();

    HeartbeatSummary {
        total_checked: statuses.len(),
        responsive: statuses.len() - unresponsive_agents.len(),
        unresponsive: unresponsive_agents.len(),
        unresponsive_agents,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quiet_hours_parsing() {
        // We can't easily test time-dependent logic, but we can test format parsing
        assert!(!is_quiet_hours("invalid"));
        assert!(!is_quiet_hours(""));
        assert!(!is_quiet_hours("25:00-06:00")); // Invalid hours handled gracefully
    }

    #[test]
    fn test_quiet_hours_format_valid() {
        // The function returns true/false based on current time
        // We just verify it doesn't panic on valid input
        let _ = is_quiet_hours("22:00-06:00");
        let _ = is_quiet_hours("00:00-23:59");
        let _ = is_quiet_hours("09:00-17:00");
    }

    #[test]
    fn test_heartbeat_config_default() {
        let config = HeartbeatConfig::default();
        assert_eq!(config.check_interval_secs, 30);
        assert_eq!(config.default_timeout_secs, 60);
    }

    #[test]
    fn test_summarize_empty() {
        let summary = summarize(&[]);
        assert_eq!(summary.total_checked, 0);
        assert_eq!(summary.responsive, 0);
        assert_eq!(summary.unresponsive, 0);
    }

    #[test]
    fn test_check_agents_skips_non_autonomous() {
        use chrono::Duration;
        use librefang_types::agent::{
            AgentEntry, AgentIdentity, AgentManifest, AgentMode, AutonomousConfig, SessionId,
        };

        let registry = AgentRegistry::new();
        let config = HeartbeatConfig::default();

        // Register a running, non-autonomous agent (autonomous = None).
        // It has been inactive long enough to be "unresponsive" if checked,
        // but it should be skipped entirely.
        let non_autonomous_entry = AgentEntry {
            id: AgentId::new(),
            name: "passive-agent".to_string(),
            manifest: AgentManifest::default(), // autonomous is None
            state: AgentState::Running,
            mode: AgentMode::default(),
            created_at: Utc::now(),
            last_active: Utc::now() - Duration::seconds(300),
            parent: None,
            children: Vec::new(),
            session_id: SessionId::new(),
            source_toml_path: None,
            tags: Vec::new(),
            identity: AgentIdentity::default(),
            onboarding_completed: false,
            onboarding_completed_at: None,
            is_hand: false,
            ..Default::default()
        };
        registry.register(non_autonomous_entry).unwrap();

        // Register a running, autonomous agent that IS inactive.
        // created_at is well in the past so the agent is past the IDLE_GRACE_SECS
        // window — it has genuinely been active before going silent.
        let autonomous_manifest = AgentManifest {
            autonomous: Some(AutonomousConfig::default()),
            ..Default::default()
        };
        let autonomous_entry = AgentEntry {
            id: AgentId::new(),
            name: "autonomous-agent".to_string(),
            manifest: autonomous_manifest,
            state: AgentState::Running,
            mode: AgentMode::default(),
            created_at: Utc::now() - Duration::seconds(3600),
            last_active: Utc::now() - Duration::seconds(300),
            parent: None,
            children: Vec::new(),
            session_id: SessionId::new(),
            source_toml_path: None,
            tags: Vec::new(),
            identity: AgentIdentity::default(),
            onboarding_completed: false,
            onboarding_completed_at: None,
            is_hand: false,
            ..Default::default()
        };
        registry.register(autonomous_entry).unwrap();

        let statuses = check_agents(&registry, &config);

        // Only the autonomous agent should appear in the results.
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].name, "autonomous-agent");
        assert!(statuses[0].unresponsive);
    }

    #[test]
    fn test_idle_agent_skipped_by_heartbeat() {
        // An autonomous agent spawned 5 minutes ago that has never processed a
        // message (last_active == created_at). It should NOT appear in heartbeat
        // statuses because it was never genuinely active. Prevents idle agents
        // from entering a crash-recover loop (openfang #844).
        use chrono::Duration;
        use librefang_types::agent::{
            AgentEntry, AgentIdentity, AgentManifest, AgentMode, AutonomousConfig, SessionId,
        };

        let registry = AgentRegistry::new();
        let config = HeartbeatConfig::default();

        let five_min_ago = Utc::now() - Duration::seconds(300);
        let autonomous_manifest = AgentManifest {
            autonomous: Some(AutonomousConfig::default()),
            ..Default::default()
        };
        let idle_entry = AgentEntry {
            id: AgentId::new(),
            name: "idle-autonomous".to_string(),
            manifest: autonomous_manifest,
            state: AgentState::Running,
            mode: AgentMode::default(),
            created_at: five_min_ago,
            last_active: five_min_ago, // never bumped beyond creation
            parent: None,
            children: Vec::new(),
            session_id: SessionId::new(),
            source_toml_path: None,
            tags: Vec::new(),
            identity: AgentIdentity::default(),
            onboarding_completed: false,
            onboarding_completed_at: None,
            is_hand: false,
            ..Default::default()
        };
        registry.register(idle_entry).unwrap();

        let statuses = check_agents(&registry, &config);

        assert!(
            statuses.is_empty(),
            "idle agent (last_active == created_at) must be skipped by heartbeat"
        );
    }

    #[test]
    fn test_active_agent_within_grace_then_inactive_is_skipped() {
        // Edge case: an agent whose last_active is exactly within IDLE_GRACE_SECS
        // of created_at must still be skipped, regardless of how long ago that was.
        use chrono::Duration;
        use librefang_types::agent::{
            AgentEntry, AgentIdentity, AgentManifest, AgentMode, AutonomousConfig, SessionId,
        };

        let registry = AgentRegistry::new();
        let config = HeartbeatConfig::default();

        let ten_min_ago = Utc::now() - Duration::seconds(600);
        // last_active only 5s after created_at — still within grace
        let last_active = ten_min_ago + Duration::seconds(5);
        let autonomous_manifest = AgentManifest {
            autonomous: Some(AutonomousConfig::default()),
            ..Default::default()
        };
        let entry = AgentEntry {
            id: AgentId::new(),
            name: "barely-touched".to_string(),
            manifest: autonomous_manifest,
            state: AgentState::Running,
            mode: AgentMode::default(),
            created_at: ten_min_ago,
            last_active,
            parent: None,
            children: Vec::new(),
            session_id: SessionId::new(),
            source_toml_path: None,
            tags: Vec::new(),
            identity: AgentIdentity::default(),
            onboarding_completed: false,
            onboarding_completed_at: None,
            is_hand: false,
            ..Default::default()
        };
        registry.register(entry).unwrap();

        let statuses = check_agents(&registry, &config);

        assert!(
            statuses.is_empty(),
            "agent with last_active inside IDLE_GRACE_SECS of created_at must be skipped"
        );
    }

    #[test]
    fn test_genuinely_active_agent_past_timeout_is_unresponsive() {
        // An autonomous agent that genuinely processed messages (last_active
        // well past the IDLE_GRACE_SECS window after created_at) but has gone
        // silent longer than the timeout — should be flagged unresponsive.
        use chrono::Duration;
        use librefang_types::agent::{
            AgentEntry, AgentIdentity, AgentManifest, AgentMode, AutonomousConfig, SessionId,
        };

        let registry = AgentRegistry::new();
        let config = HeartbeatConfig::default(); // default_timeout_secs = 60

        let one_hour_ago = Utc::now() - Duration::seconds(3600);
        // last_active 30 min after created_at — well past IDLE_GRACE_SECS
        let last_active = Utc::now() - Duration::seconds(300);
        let autonomous_manifest = AgentManifest {
            autonomous: Some(AutonomousConfig::default()),
            ..Default::default()
        };
        let entry = AgentEntry {
            id: AgentId::new(),
            name: "active-then-silent".to_string(),
            manifest: autonomous_manifest,
            state: AgentState::Running,
            mode: AgentMode::default(),
            created_at: one_hour_ago,
            last_active,
            parent: None,
            children: Vec::new(),
            session_id: SessionId::new(),
            source_toml_path: None,
            tags: Vec::new(),
            identity: AgentIdentity::default(),
            onboarding_completed: false,
            onboarding_completed_at: None,
            is_hand: false,
            ..Default::default()
        };
        registry.register(entry).unwrap();

        let statuses = check_agents(&registry, &config);

        assert_eq!(statuses.len(), 1);
        assert!(
            statuses[0].unresponsive,
            "genuinely active agent past timeout should be flagged unresponsive"
        );
    }

    #[test]
    fn test_summarize_mixed() {
        let statuses = vec![
            HeartbeatStatus {
                agent_id: AgentId::new(),
                name: "agent-1".to_string(),
                inactive_secs: 10,
                unresponsive: false,
            },
            HeartbeatStatus {
                agent_id: AgentId::new(),
                name: "agent-2".to_string(),
                inactive_secs: 120,
                unresponsive: true,
            },
            HeartbeatStatus {
                agent_id: AgentId::new(),
                name: "agent-3".to_string(),
                inactive_secs: 5,
                unresponsive: false,
            },
        ];

        let summary = summarize(&statuses);
        assert_eq!(summary.total_checked, 3);
        assert_eq!(summary.responsive, 2);
        assert_eq!(summary.unresponsive, 1);
        assert_eq!(summary.unresponsive_agents.len(), 1);
        assert_eq!(summary.unresponsive_agents[0].name, "agent-2");
    }
}
