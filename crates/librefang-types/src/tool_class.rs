//! Risk classification for tools.
//!
//! [`ToolApprovalClass`] tags every tool with a coarse risk class so the
//! approval ladder can apply tier-appropriate gating (auto-allow read-only
//! reads, prompt for mutations, require TOTP for control-plane changes,
//! etc.). This module only defines the taxonomy; nothing is wired to the
//! actual approval manager yet — that will be a follow-up PR.
//!
//! Modeled after openclaw's `AcpApprovalClass` (7 classes, severity-ordered).

use serde::{Deserialize, Serialize};
use std::fmt;

/// Risk class assigned to a tool, used to drive the approval ladder.
///
/// Variants are ordered from least to most surprising effect, with
/// [`Unknown`] reserved for tools that haven't been classified yet
/// (so the policy layer can be conservative by default).
///
/// [`Unknown`]: ToolApprovalClass::Unknown
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolApprovalClass {
    /// Pure read, scoped to workspace (e.g. `file_read`, `glob`, `grep`).
    ReadonlyScoped,
    /// Pure read, unscoped network search (`web_search`, `web_fetch` GET).
    ReadonlySearch,
    /// Mutates state in workspace (`file_write`, `apply_patch`).
    Mutating,
    /// Executes external code or commands (`shell_exec`, `python_exec`).
    ExecCapable,
    /// Modifies daemon configuration or agent registry.
    ControlPlane,
    /// Requires interactive user response (TOTP, prompt).
    Interactive,
    /// Default when no classification matches.
    #[default]
    Unknown,
}

impl ToolApprovalClass {
    /// Numeric severity for sorting / threshold comparisons.
    ///
    /// `0` is least risky (workspace reads), `5` covers interactive and
    /// `6` is reserved for `Unknown` so unclassified tools sort to the
    /// most-cautious end of any ranking.
    pub fn severity_rank(self) -> u8 {
        match self {
            Self::ReadonlyScoped => 0,
            Self::ReadonlySearch => 1,
            Self::Mutating => 2,
            Self::ExecCapable => 3,
            Self::ControlPlane => 4,
            Self::Interactive => 5,
            Self::Unknown => 6,
        }
    }

    /// Snake-case identifier matching the serde representation.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadonlyScoped => "readonly_scoped",
            Self::ReadonlySearch => "readonly_search",
            Self::Mutating => "mutating",
            Self::ExecCapable => "exec_capable",
            Self::ControlPlane => "control_plane",
            Self::Interactive => "interactive",
            Self::Unknown => "unknown",
        }
    }

    /// Parse a snake_case identifier back into a class.
    ///
    /// Used by the classifier to honor explicit annotations carried inside
    /// a tool's JSON schema (e.g. `"x-tool-class": "exec_capable"`).
    pub fn from_snake_case(s: &str) -> Option<Self> {
        match s {
            "readonly_scoped" => Some(Self::ReadonlyScoped),
            "readonly_search" => Some(Self::ReadonlySearch),
            "mutating" => Some(Self::Mutating),
            "exec_capable" => Some(Self::ExecCapable),
            "control_plane" => Some(Self::ControlPlane),
            "interactive" => Some(Self::Interactive),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

impl fmt::Display for ToolApprovalClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_returns_snake_case() {
        assert_eq!(
            ToolApprovalClass::ReadonlyScoped.to_string(),
            "readonly_scoped"
        );
        assert_eq!(
            ToolApprovalClass::ReadonlySearch.to_string(),
            "readonly_search"
        );
        assert_eq!(ToolApprovalClass::Mutating.to_string(), "mutating");
        assert_eq!(ToolApprovalClass::ExecCapable.to_string(), "exec_capable");
        assert_eq!(ToolApprovalClass::ControlPlane.to_string(), "control_plane");
        assert_eq!(ToolApprovalClass::Interactive.to_string(), "interactive");
        assert_eq!(ToolApprovalClass::Unknown.to_string(), "unknown");
    }

    #[test]
    fn default_is_unknown() {
        assert_eq!(ToolApprovalClass::default(), ToolApprovalClass::Unknown);
    }

    #[test]
    fn severity_rank_orders_least_to_most_risky() {
        assert_eq!(ToolApprovalClass::ReadonlyScoped.severity_rank(), 0);
        assert_eq!(ToolApprovalClass::ReadonlySearch.severity_rank(), 1);
        assert_eq!(ToolApprovalClass::Mutating.severity_rank(), 2);
        assert_eq!(ToolApprovalClass::ExecCapable.severity_rank(), 3);
        assert_eq!(ToolApprovalClass::ControlPlane.severity_rank(), 4);
        assert_eq!(ToolApprovalClass::Interactive.severity_rank(), 5);
        assert_eq!(ToolApprovalClass::Unknown.severity_rank(), 6);
    }

    #[test]
    fn severity_rank_relative_ordering() {
        // Spot checks called out in the task spec.
        assert!(
            ToolApprovalClass::ReadonlyScoped.severity_rank()
                < ToolApprovalClass::ExecCapable.severity_rank()
        );
        assert!(
            ToolApprovalClass::ExecCapable.severity_rank()
                < ToolApprovalClass::Interactive.severity_rank()
        );
        assert!(
            ToolApprovalClass::Interactive.severity_rank()
                < ToolApprovalClass::Unknown.severity_rank()
        );
    }

    #[test]
    fn serde_roundtrip_matches_snake_case() {
        assert_eq!(
            serde_json::to_string(&ToolApprovalClass::ReadonlyScoped).unwrap(),
            "\"readonly_scoped\""
        );
        let parsed: ToolApprovalClass = serde_json::from_str("\"exec_capable\"").unwrap();
        assert_eq!(parsed, ToolApprovalClass::ExecCapable);
    }

    #[test]
    fn from_snake_case_roundtrip() {
        for class in [
            ToolApprovalClass::ReadonlyScoped,
            ToolApprovalClass::ReadonlySearch,
            ToolApprovalClass::Mutating,
            ToolApprovalClass::ExecCapable,
            ToolApprovalClass::ControlPlane,
            ToolApprovalClass::Interactive,
            ToolApprovalClass::Unknown,
        ] {
            assert_eq!(
                ToolApprovalClass::from_snake_case(class.as_str()),
                Some(class)
            );
        }
        assert_eq!(ToolApprovalClass::from_snake_case("nope"), None);
    }
}
