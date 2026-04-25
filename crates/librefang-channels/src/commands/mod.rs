//! Central registry of slash commands surfaced by LibreFang.
//!
//! Single source of truth for every `/cmd` LibreFang understands. Every
//! consumer (channel bridge dispatch / `/help` text, Telegram BotCommands menu,
//! TUI chat runner, future Dashboard command palette) derives from
//! [`COMMAND_REGISTRY`] instead of maintaining its own copy.
//!
//! See `.plans/slash-command-registry.md` for the design rationale and
//! migration plan.
//!
//! Behavior (the actual handler for each command) still lives in
//! `bridge::handle_command` and per-surface dispatchers; this module only
//! describes commands.

use bitflags::bitflags;
use librefang_types::config::ChannelOverrides;

bitflags! {
    /// Surfaces a command may appear on. A command can be visible on multiple
    /// surfaces simultaneously (e.g. CHANNEL + CLI).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Scope: u8 {
        /// `librefang-cli` interactive REPL / TUI chat runner.
        const CLI       = 0b0000_0001;
        /// Every channel adapter (Telegram, Slack, Discord, …).
        const CHANNEL   = 0b0000_0010;
        /// Dashboard SPA command palette via `GET /api/commands`.
        const DASHBOARD = 0b0000_0100;
    }
}

/// Logical grouping for `/help` rendering.
///
/// `Misc` carries an unlabeled trailing block (matches the historical
/// hand-written `/help` layout where `/btw`, `/start`, `/help` had no header).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Session,
    Info,
    Automation,
    Monitoring,
    Misc,
}

impl Category {
    /// Section header to render in `/help`. `None` means "no header".
    fn label(self) -> Option<&'static str> {
        match self {
            Self::Session => Some("Session"),
            Self::Info => Some("Info"),
            Self::Automation => Some("Automation"),
            Self::Monitoring => Some("Monitoring"),
            Self::Misc => None,
        }
    }
}

/// One sub-form of a multi-action command (e.g. `/trigger add …` vs
/// `/trigger del …`). Renders as its own `/help` line.
#[derive(Debug, Clone, Copy)]
pub struct SubCommand {
    /// Args after the command name, including the verb.
    /// Example: `"add <agent> <pattern> <prompt>"`.
    pub args: &'static str,
    /// Sentence-case description for this sub-form.
    pub description: &'static str,
}

/// Static metadata for one slash command.
#[derive(Debug, Clone, Copy)]
pub struct CommandDef {
    /// Bare command name without leading `/` (e.g. `"agents"`).
    pub name: &'static str,
    /// Alternate names that resolve to the same command. Empty for now;
    /// reserved for future use (e.g. abbreviations).
    pub aliases: &'static [&'static str],
    /// Logical group for `/help` rendering.
    pub category: Category,
    /// Where this command is exposed.
    pub scope: Scope,
    /// Sentence-case description, no trailing period.
    pub description: &'static str,
    /// Args hint shown in `/help` for the top-level form. `""` when the
    /// command has no args, or when [`subcommands`] takes over.
    pub args_hint: &'static str,
    /// Optional sub-forms; when non-empty, `/help` renders one line per entry
    /// instead of a single line for the top-level form.
    pub subcommands: &'static [SubCommand],
    /// Whether to include this command in the Telegram BotCommands menu
    /// (the popup shown when the user types `/`). Telegram limits to 100.
    pub telegram_menu: bool,
}

// Sub-command tables for multi-form commands.
//
// Kept as `static` (not inline) so they have a stable address for the
// `&'static [SubCommand]` reference inside `CommandDef`.

static TRIGGER_SUBCOMMANDS: &[SubCommand] = &[
    SubCommand {
        args: "add <agent> <pattern> <prompt>",
        description: "create trigger",
    },
    SubCommand {
        args: "del <id>",
        description: "remove trigger",
    },
];

static SCHEDULE_SUBCOMMANDS: &[SubCommand] = &[
    SubCommand {
        args: "add <agent> <cron-5-fields> <message>",
        description: "create job",
    },
    SubCommand {
        args: "del <id>",
        description: "remove job",
    },
    SubCommand {
        args: "run <id>",
        description: "run job now",
    },
];

/// The single source of truth for slash commands.
///
/// Order is significant: it controls the visual order inside each
/// [`Category`] in `/help`. Categories themselves are emitted in the order
/// the first command of each category appears.
pub const COMMAND_REGISTRY: &[CommandDef] = &[
    // ---- Session ----
    CommandDef {
        name: "agents",
        aliases: &[],
        category: Category::Session,
        scope: Scope::CHANNEL,
        description: "List running agents",
        args_hint: "",
        subcommands: &[],
        telegram_menu: true,
    },
    CommandDef {
        name: "agent",
        aliases: &[],
        category: Category::Session,
        scope: Scope::CHANNEL,
        description: "Select which agent to talk to",
        args_hint: "<name>",
        subcommands: &[],
        telegram_menu: true,
    },
    CommandDef {
        name: "new",
        aliases: &[],
        category: Category::Session,
        scope: Scope::CHANNEL,
        description: "Reset session (clear messages)",
        args_hint: "",
        subcommands: &[],
        telegram_menu: true,
    },
    CommandDef {
        name: "reboot",
        aliases: &[],
        category: Category::Session,
        scope: Scope::CHANNEL,
        description: "Hard reset session (full context clear, no summary)",
        args_hint: "",
        subcommands: &[],
        telegram_menu: true,
    },
    CommandDef {
        name: "compact",
        aliases: &[],
        category: Category::Session,
        scope: Scope::CHANNEL,
        description: "Trigger LLM session compaction",
        args_hint: "",
        subcommands: &[],
        telegram_menu: true,
    },
    CommandDef {
        name: "model",
        aliases: &[],
        category: Category::Session,
        scope: Scope::CHANNEL,
        description: "Show or switch agent model",
        args_hint: "[name]",
        subcommands: &[],
        telegram_menu: true,
    },
    CommandDef {
        name: "stop",
        aliases: &[],
        category: Category::Session,
        scope: Scope::CHANNEL,
        description: "Cancel current agent run",
        args_hint: "",
        subcommands: &[],
        telegram_menu: true,
    },
    CommandDef {
        name: "usage",
        aliases: &[],
        category: Category::Session,
        scope: Scope::CHANNEL,
        description: "Show session token usage and cost",
        args_hint: "",
        subcommands: &[],
        telegram_menu: true,
    },
    CommandDef {
        name: "think",
        aliases: &[],
        category: Category::Session,
        scope: Scope::CHANNEL,
        description: "Toggle extended thinking",
        args_hint: "[on|off]",
        subcommands: &[],
        telegram_menu: true,
    },
    // ---- Info ----
    CommandDef {
        name: "models",
        aliases: &[],
        category: Category::Info,
        scope: Scope::CHANNEL,
        description: "List available AI models",
        args_hint: "",
        subcommands: &[],
        telegram_menu: true,
    },
    CommandDef {
        name: "providers",
        aliases: &[],
        category: Category::Info,
        scope: Scope::CHANNEL,
        description: "Show configured providers",
        args_hint: "",
        subcommands: &[],
        telegram_menu: true,
    },
    CommandDef {
        name: "skills",
        aliases: &[],
        category: Category::Info,
        scope: Scope::CHANNEL,
        description: "List installed skills",
        args_hint: "",
        subcommands: &[],
        telegram_menu: true,
    },
    CommandDef {
        name: "hands",
        aliases: &[],
        category: Category::Info,
        scope: Scope::CHANNEL,
        description: "List available and active hands",
        args_hint: "",
        subcommands: &[],
        telegram_menu: true,
    },
    CommandDef {
        name: "status",
        aliases: &[],
        category: Category::Info,
        scope: Scope::CHANNEL,
        description: "Show system status",
        args_hint: "",
        subcommands: &[],
        telegram_menu: true,
    },
    // ---- Automation ----
    CommandDef {
        name: "workflows",
        aliases: &[],
        category: Category::Automation,
        scope: Scope::CHANNEL,
        description: "List workflows",
        args_hint: "",
        subcommands: &[],
        telegram_menu: true,
    },
    CommandDef {
        name: "workflow",
        aliases: &[],
        category: Category::Automation,
        scope: Scope::CHANNEL,
        description: "Run a workflow",
        args_hint: "run <name> [input]",
        subcommands: &[],
        telegram_menu: true,
    },
    CommandDef {
        name: "triggers",
        aliases: &[],
        category: Category::Automation,
        scope: Scope::CHANNEL,
        description: "List event triggers",
        args_hint: "",
        subcommands: &[],
        telegram_menu: false,
    },
    CommandDef {
        name: "trigger",
        aliases: &[],
        category: Category::Automation,
        scope: Scope::CHANNEL,
        description: "Manage event triggers",
        args_hint: "",
        subcommands: TRIGGER_SUBCOMMANDS,
        telegram_menu: false,
    },
    CommandDef {
        name: "schedules",
        aliases: &[],
        category: Category::Automation,
        scope: Scope::CHANNEL,
        description: "List cron jobs",
        args_hint: "",
        subcommands: &[],
        telegram_menu: false,
    },
    CommandDef {
        name: "schedule",
        aliases: &[],
        category: Category::Automation,
        scope: Scope::CHANNEL,
        description: "Manage cron jobs",
        args_hint: "",
        subcommands: SCHEDULE_SUBCOMMANDS,
        telegram_menu: false,
    },
    CommandDef {
        name: "approvals",
        aliases: &[],
        category: Category::Automation,
        scope: Scope::CHANNEL,
        description: "List pending approvals",
        args_hint: "",
        subcommands: &[],
        telegram_menu: false,
    },
    CommandDef {
        name: "approve",
        aliases: &[],
        category: Category::Automation,
        scope: Scope::CHANNEL,
        description: "Approve a request",
        args_hint: "<id>",
        subcommands: &[],
        telegram_menu: false,
    },
    CommandDef {
        name: "reject",
        aliases: &[],
        category: Category::Automation,
        scope: Scope::CHANNEL,
        description: "Reject a request",
        args_hint: "<id>",
        subcommands: &[],
        telegram_menu: false,
    },
    // ---- Monitoring ----
    CommandDef {
        name: "budget",
        aliases: &[],
        category: Category::Monitoring,
        scope: Scope::CHANNEL,
        description: "Show spending limits and current costs",
        args_hint: "",
        subcommands: &[],
        telegram_menu: true,
    },
    CommandDef {
        name: "peers",
        aliases: &[],
        category: Category::Monitoring,
        scope: Scope::CHANNEL,
        description: "Show OFP peer network status",
        args_hint: "",
        subcommands: &[],
        telegram_menu: false,
    },
    CommandDef {
        name: "a2a",
        aliases: &[],
        category: Category::Monitoring,
        scope: Scope::CHANNEL,
        description: "List discovered external A2A agents",
        args_hint: "",
        subcommands: &[],
        telegram_menu: false,
    },
    // ---- Misc (no header in /help) ----
    CommandDef {
        name: "btw",
        aliases: &[],
        category: Category::Misc,
        scope: Scope::CHANNEL,
        description: "Ask a side question (ephemeral, not saved to session)",
        args_hint: "<question>",
        subcommands: &[],
        telegram_menu: true,
    },
    CommandDef {
        name: "start",
        aliases: &[],
        category: Category::Misc,
        scope: Scope::CHANNEL,
        description: "Show welcome message",
        args_hint: "",
        subcommands: &[],
        telegram_menu: true,
    },
    CommandDef {
        name: "help",
        aliases: &[],
        category: Category::Misc,
        scope: Scope::CHANNEL,
        description: "Show this help",
        args_hint: "",
        subcommands: &[],
        telegram_menu: true,
    },
];

/// Look up a command by its bare name or any alias. The leading `/` is
/// optional and stripped if present.
pub fn lookup(name_or_alias: &str) -> Option<&'static CommandDef> {
    let bare = name_or_alias.strip_prefix('/').unwrap_or(name_or_alias);
    COMMAND_REGISTRY
        .iter()
        .find(|c| c.name == bare || c.aliases.contains(&bare))
}

/// Iterate every command visible on at least one of the given surfaces.
pub fn iter_for(scope: Scope) -> impl Iterator<Item = &'static CommandDef> {
    COMMAND_REGISTRY
        .iter()
        .filter(move |c| c.scope.intersects(scope))
}

/// Whether `name` is a known channel command (any registered command with
/// `Scope::CHANNEL`). This is the replacement for the historical hand-written
/// `matches!(...)` block in `bridge.rs`.
pub fn is_channel_command(name: &str) -> bool {
    lookup(name).is_some_and(|c| c.scope.contains(Scope::CHANNEL))
}

/// Check whether a built-in slash command is permitted given channel
/// overrides.
///
/// Precedence: `disable_commands` > `allowed_commands` (whitelist) >
/// `blocked_commands` (blacklist). When no overrides are configured,
/// everything is allowed.
///
/// Config entries may be written with or without a leading `/`
/// (`"agent"` and `"/agent"` both match the dispatcher's bare token).
pub fn is_command_allowed(cmd: &str, overrides: Option<&ChannelOverrides>) -> bool {
    let Some(ov) = overrides else { return true };
    if ov.disable_commands {
        return false;
    }
    let matches = |entry: &String| -> bool {
        let name = entry.strip_prefix('/').unwrap_or(entry);
        name == cmd
    };
    if !ov.allowed_commands.is_empty() {
        return ov.allowed_commands.iter().any(matches);
    }
    !ov.blocked_commands.iter().any(matches)
}

/// Render the channel-facing `/help` text.
///
/// Honors `overrides` so that disabled / filtered commands don't appear.
/// Categories with no surviving commands are silently skipped.
pub fn channel_help_text(overrides: Option<&ChannelOverrides>) -> String {
    let visible: Vec<&CommandDef> = COMMAND_REGISTRY
        .iter()
        .filter(|c| c.scope.contains(Scope::CHANNEL))
        .filter(|c| is_command_allowed(c.name, overrides))
        .collect();

    let mut out = String::from("LibreFang Bot Commands:");
    let mut current_cat: Option<Category> = None;
    let mut first_in_section = true;

    for c in &visible {
        if Some(c.category) != current_cat {
            // Section break: blank line between groups.
            out.push_str("\n\n");
            if let Some(label) = c.category.label() {
                out.push_str(label);
                out.push_str(":\n");
            }
            current_cat = Some(c.category);
            first_in_section = true;
        }

        if c.subcommands.is_empty() {
            if !first_in_section {
                out.push('\n');
            }
            if c.args_hint.is_empty() {
                out.push_str(&format!("/{} - {}", c.name, c.description));
            } else {
                out.push_str(&format!("/{} {} - {}", c.name, c.args_hint, c.description));
            }
        } else {
            for sub in c.subcommands {
                if !first_in_section {
                    out.push('\n');
                }
                out.push_str(&format!("/{} {} - {}", c.name, sub.args, sub.description));
                first_in_section = false;
            }
            // Defer setting first_in_section=false to common path below
            // — already handled inside loop.
            continue;
        }

        first_in_section = false;
    }

    out
}

/// Pairs of `(name, description)` for the Telegram BotCommands menu.
///
/// Adapters are responsible for converting to their wire type
/// (e.g. `telegram::BotCommand`).
pub fn telegram_bot_commands() -> Vec<(String, String)> {
    COMMAND_REGISTRY
        .iter()
        .filter(|c| c.telegram_menu && c.scope.contains(Scope::CHANNEL))
        .map(|c| (c.name.to_string(), c.description.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lookup must accept bare and slash-prefixed names; aliases must
    /// resolve to the canonical entry.
    #[test]
    fn lookup_resolves_bare_and_slashed() {
        assert_eq!(lookup("agents").map(|c| c.name), Some("agents"));
        assert_eq!(lookup("/agents").map(|c| c.name), Some("agents"));
        assert!(lookup("nonexistent").is_none());
        assert!(lookup("").is_none());
    }

    /// The channel-visible set must exactly match the historical
    /// hand-written `matches!` list in `bridge.rs:2777-2808`. This is the
    /// "anti-drift" golden assertion.
    #[test]
    fn channel_command_names_match_historical_set() {
        let expected: &[&str] = &[
            "start",
            "help",
            "agents",
            "agent",
            "status",
            "models",
            "providers",
            "new",
            "reboot",
            "compact",
            "model",
            "stop",
            "usage",
            "think",
            "skills",
            "hands",
            "btw",
            "workflows",
            "workflow",
            "triggers",
            "trigger",
            "schedules",
            "schedule",
            "approvals",
            "approve",
            "reject",
            "budget",
            "peers",
            "a2a",
        ];

        let actual: std::collections::BTreeSet<&str> =
            iter_for(Scope::CHANNEL).map(|c| c.name).collect();
        let want: std::collections::BTreeSet<&str> = expected.iter().copied().collect();

        assert_eq!(
            actual, want,
            "channel command set drifted from the historical bridge.rs matches! list"
        );
    }

    /// Names + aliases must form a global unique set across the registry,
    /// otherwise `lookup` becomes ambiguous.
    #[test]
    fn names_and_aliases_are_globally_unique() {
        let mut seen: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
        for c in COMMAND_REGISTRY {
            for token in std::iter::once(c.name).chain(c.aliases.iter().copied()) {
                if let Some(prev) = seen.insert(token, c.name) {
                    panic!(
                        "duplicate command token `{}` registered by both `{}` and `{}`",
                        token, prev, c.name
                    );
                }
            }
        }
    }

    /// Telegram BotCommands menu has hard limits (Bot API):
    /// - command name: 1..=32 chars, lowercase letters / digits / underscore.
    /// - description: 1..=256 chars.
    /// - at most 100 commands per scope.
    #[test]
    fn telegram_menu_respects_bot_api_limits() {
        let entries = telegram_bot_commands();
        assert!(
            entries.len() <= 100,
            "Telegram allows at most 100 BotCommands; got {}",
            entries.len()
        );
        for (name, desc) in &entries {
            assert!(
                (1..=32).contains(&name.len()),
                "command `{name}` length {} out of [1,32]",
                name.len()
            );
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "command `{name}` has invalid chars (Telegram allows [a-z0-9_])"
            );
            assert!(
                (1..=256).contains(&desc.len()),
                "description for `{name}` length {} out of [1,256]",
                desc.len()
            );
        }
    }

    #[test]
    fn is_command_allowed_no_overrides_allows_all() {
        assert!(is_command_allowed("help", None));
        assert!(is_command_allowed("agent", None));
    }

    #[test]
    fn is_command_allowed_disable_blocks_all() {
        let ov = ChannelOverrides {
            disable_commands: true,
            ..Default::default()
        };
        assert!(!is_command_allowed("help", Some(&ov)));
    }

    #[test]
    fn is_command_allowed_whitelist_then_blacklist() {
        let ov = ChannelOverrides {
            allowed_commands: vec!["help".into(), "/start".into()],
            blocked_commands: vec!["help".into()],
            ..Default::default()
        };
        // Whitelist wins over blacklist.
        assert!(is_command_allowed("help", Some(&ov)));
        assert!(is_command_allowed("start", Some(&ov)));
        assert!(!is_command_allowed("agent", Some(&ov)));
    }

    #[test]
    fn is_command_allowed_blacklist_only() {
        let ov = ChannelOverrides {
            blocked_commands: vec!["agent".into(), "/new".into()],
            ..Default::default()
        };
        assert!(!is_command_allowed("agent", Some(&ov)));
        assert!(!is_command_allowed("new", Some(&ov)));
        assert!(is_command_allowed("help", Some(&ov)));
    }

    #[test]
    fn channel_help_text_groups_categories() {
        let txt = channel_help_text(None);
        assert!(txt.starts_with("LibreFang Bot Commands:"));
        assert!(txt.contains("\n\nSession:\n"));
        assert!(txt.contains("\n\nInfo:\n"));
        assert!(txt.contains("\n\nAutomation:\n"));
        assert!(txt.contains("\n\nMonitoring:\n"));
        // Misc has no header.
        assert!(!txt.contains("Misc:"));
        // Subcommands render multi-line for /trigger and /schedule.
        assert!(txt.contains("/trigger add <agent> <pattern> <prompt> - create trigger"));
        assert!(txt.contains("/trigger del <id> - remove trigger"));
        assert!(txt.contains("/schedule run <id> - run job now"));
        // Single-arg form.
        assert!(txt.contains("/agent <name> - Select which agent to talk to"));
        // No-arg form.
        assert!(txt.contains("/agents - List running agents"));
        // Misc (no header) commands appear at the end.
        let btw_pos = txt.find("/btw").expect("btw missing");
        let mon_pos = txt.find("Monitoring:").expect("monitoring header missing");
        assert!(
            btw_pos > mon_pos,
            "btw should appear after Monitoring section"
        );
    }

    #[test]
    fn channel_help_text_filters_blocked() {
        let ov = ChannelOverrides {
            blocked_commands: vec!["help".into(), "agent".into()],
            ..Default::default()
        };
        let txt = channel_help_text(Some(&ov));
        assert!(!txt.contains("/help "));
        assert!(!txt.contains("/help -"));
        assert!(!txt.contains("/agent <name>"));
        // Other commands still show up.
        assert!(txt.contains("/agents - List running agents"));
    }

    #[test]
    fn telegram_menu_is_subset_of_channel_set() {
        let menu: std::collections::BTreeSet<String> = telegram_bot_commands()
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        let channel: std::collections::BTreeSet<String> = iter_for(Scope::CHANNEL)
            .map(|c| c.name.to_string())
            .collect();
        assert!(
            menu.is_subset(&channel),
            "telegram menu must be a subset of channel-scoped commands"
        );
        // Non-empty so this test catches accidental empty registry.
        assert!(!menu.is_empty());
    }
}
