//! LibreFang CLI — command-line interface for the LibreFang Agent OS.
//!
//! When a daemon is running (`librefang start`), the CLI talks to it over HTTP.
//! Otherwise, commands boot an in-process kernel (single-shot mode).

#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

mod desktop_install;
mod http_client;
pub mod i18n;
mod launcher;
mod mcp;
pub mod progress;
pub mod table;
mod templates;
mod tui;
mod ui;

use clap::{Parser, Subcommand};
use colored::Colorize;
use librefang_api::server::read_daemon_info;
use librefang_extensions::dotenv;
use librefang_kernel::{config::load_config, LibreFangKernel};
use librefang_types::agent::{AgentId, AgentManifest};
use std::ffi::OsString;
use std::io::{self, BufRead, Write};
#[cfg(unix)]
use std::os::unix::io::RawFd;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::AtomicBool;
#[cfg(windows)]
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Global flag set by the Ctrl+C handler.
static CTRLC_PRESSED: AtomicBool = AtomicBool::new(false);
const INIT_DEFAULT_CONFIG_TEMPLATE: &str = include_str!("../templates/init_default_config.toml");
const LOG_RETENTION_DAYS: u64 = 7;

/// Install a Ctrl+C handler that force-exits the process.
/// On Windows/MINGW, the default handler doesn't reliably interrupt blocking
/// `read_line` calls, so we explicitly call `process::exit`.
fn install_ctrlc_handler() {
    #[cfg(windows)]
    {
        extern "system" {
            fn SetConsoleCtrlHandler(
                handler: Option<unsafe extern "system" fn(u32) -> i32>,
                add: i32,
            ) -> i32;
        }
        unsafe extern "system" fn handler(_ctrl_type: u32) -> i32 {
            if CTRLC_PRESSED.swap(true, Ordering::SeqCst) {
                // Second press: hard exit
                std::process::exit(130);
            }
            // First press: print message and exit cleanly
            let _ = std::io::Write::write_all(&mut std::io::stderr(), b"\nInterrupted.\n");
            std::process::exit(0);
        }
        unsafe { SetConsoleCtrlHandler(Some(handler), 1) };
    }

    #[cfg(not(windows))]
    {
        // On Unix, the default SIGINT handler already interrupts read_line
        // and terminates the process.
        let _ = &CTRLC_PRESSED;
    }
}

const AFTER_HELP: &str = "\
\x1b[1mHint:\x1b[0m Commands suffixed with [*] have subcommands. Run `<command> --help` for details.

\x1b[1;36mExamples:\x1b[0m
  librefang init                 Initialize config and data directories
  librefang start                Start the kernel daemon
  librefang update               Update the CLI to the latest release
  librefang tui                  Launch the interactive terminal dashboard
  librefang chat                 Quick chat with the default agent
  librefang agent new coder      Spawn a new agent from a template
  librefang models list          Browse available LLM models
  librefang mcp add github       Install the GitHub MCP server
  librefang doctor               Run diagnostic health checks
  librefang channel setup        Interactive channel setup wizard
  librefang cron list            List scheduled jobs
  librefang uninstall            Completely remove LibreFang from your system

\x1b[1;36mQuick Start:\x1b[0m
  1. librefang init              Set up config + API key
  2. librefang start             Launch the daemon
  3. librefang chat              Start chatting!

\x1b[1;36mMore:\x1b[0m
  Docs:       https://github.com/librefang/librefang
  Dashboard:  http://127.0.0.1:4545/ (when daemon is running)";

/// LibreFang — the open-source Agent Operating System.
#[derive(Parser)]
#[command(
    name = "librefang",
    version,
    about = "\u{1F40D} LibreFang \u{2014} Open-source Agent Operating System",
    long_about = "\u{1F40D} LibreFang \u{2014} Open-source Agent Operating System\n\n\
                  Deploy, manage, and orchestrate AI agents from your terminal.\n\
                  40 channels \u{00b7} 60 skills \u{00b7} 50+ models \u{00b7} infinite possibilities.",
    after_help = AFTER_HELP,
)]
pub(crate) struct Cli {
    /// Path to config file.
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize LibreFang (create ~/.librefang/ and default config).
    #[command(
        long_about = "Initialize LibreFang by creating the ~/.librefang/ directory and a default config.toml.\n\nThis is the first command you should run after installing LibreFang. It sets up\nthe data directory, writes a default configuration, and optionally prompts for\nan API key.\n\nExamples:\n  librefang init              # Interactive setup with prompts\n  librefang init --quick      # Non-interactive, just write defaults (CI/scripts)"
    )]
    Init {
        /// Quick mode: no prompts, just write config + .env (for CI/scripts).
        #[arg(long, conflicts_with = "upgrade")]
        quick: bool,
        /// Upgrade an existing installation: backup config, sync registry, merge new defaults.
        #[arg(long, conflicts_with = "quick")]
        upgrade: bool,
    },
    /// Start the LibreFang kernel daemon (API server + kernel).
    #[command(
        long_about = "Start the LibreFang kernel daemon, which runs the API server and agent runtime.\n\nBy default the daemon detaches into the background. Use --foreground to keep it\nattached to the current terminal, or --tail to detach but stream logs.\n\nThe bind address is resolved in priority order:\n  --bind flag > LIBREFANG_LISTEN env var > api_listen in config.toml > compiled default (0.0.0.0:4545)\n\nExamples:\n  librefang start                              # Start daemon in the background\n  librefang start --tail                       # Start and follow log output\n  librefang start --foreground                 # Run in the foreground (Ctrl+C to stop)\n  librefang start --bind 127.0.0.1:4545        # Loopback only\n  librefang start --bind 0.0.0.0:9000          # All interfaces, custom port"
    )]
    Start {
        /// Follow the daemon log after launching it in the background.
        #[arg(long, conflicts_with_all = ["foreground", "spawned"])]
        tail: bool,
        /// Keep the daemon attached to the current terminal.
        #[arg(long, conflicts_with = "spawned")]
        foreground: bool,
        /// Internal flag used by the detached daemon child process.
        #[arg(long, hide = true)]
        spawned: bool,
        /// Override the bind address for this run (e.g. "0.0.0.0:4545" or "127.0.0.1:9000").
        /// Overrides LIBREFANG_LISTEN env var and api_listen in config.toml.
        #[arg(long, value_name = "HOST:PORT", env = "LIBREFANG_LISTEN")]
        bind: Option<String>,
    },
    /// Restart the running daemon (or start it if not running).
    #[command(
        long_about = "Restart the running daemon, or start it if it is not already running.\n\nThis stops the current daemon process and launches a fresh one. Useful after\nchanging configuration or updating the binary.\n\nExamples:\n  librefang restart              # Restart in the background\n  librefang restart --tail       # Restart and follow log output\n  librefang restart --foreground # Restart in the foreground"
    )]
    Restart {
        /// Follow the daemon log after launching it in the background.
        #[arg(long, conflicts_with = "foreground")]
        tail: bool,
        /// Keep the relaunched daemon attached to the current terminal.
        #[arg(long)]
        foreground: bool,
    },
    /// Spawn an agent by template name or manifest path.
    #[command(
        long_about = "Spawn a new agent from a built-in template or a manifest file.\n\nIf no target is given, an interactive picker is shown. You can also pass\na template name (e.g. \"coder\") or a path to a TOML manifest.\n\nExamples:\n  librefang spawn               # Interactive template picker\n  librefang spawn coder         # Spawn from the \"coder\" template\n  librefang spawn ./agent.toml  # Spawn from a manifest file\n  librefang spawn coder --name my-agent  # Override agent name\n  librefang spawn coder --dry-run        # Preview without spawning"
    )]
    Spawn(SpawnAliasArgs),
    /// List running agents (alias for `agent list`).
    #[command(
        long_about = "List all currently running agents.\n\nThis is a convenience alias for `librefang agent list`.\n\nExamples:\n  librefang agents          # Pretty-printed table\n  librefang agents --json   # JSON output for scripting"
    )]
    Agents {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Kill a running agent by ID (alias for `agent kill`).
    #[command(
        long_about = "Kill a running agent by its UUID.\n\nThis is a convenience alias for `librefang agent kill`.\n\nExamples:\n  librefang kill 550e8400-e29b-41d4-a716-446655440000"
    )]
    Kill {
        /// Agent ID (UUID).
        agent_id: String,
    },
    /// Update the CLI to the latest published release.
    #[command(
        long_about = "Update the LibreFang CLI binary to the latest published GitHub release.\n\nBy default, downloads and installs the latest release for your configured\nupdate channel. Use --check to see if an update is available without\ninstalling, --version to pin a specific tag, or --channel to override.\n\nChannels (like Apple software updates):\n  stable  — only stable releases (default)\n  beta    — stable + beta releases\n  rc      — all releases including release candidates\n\nSet a persistent default in config.toml:\n  update_channel = \"rc\"\n\nExamples:\n  librefang update                   # Install latest for your channel\n  librefang update --check           # Check for updates only\n  librefang update --channel rc      # Use rc channel for this update\n  librefang update --version v0.4.0  # Install a specific version"
    )]
    Update {
        /// Check whether a newer release exists without installing it.
        #[arg(long)]
        check: bool,
        /// Install a specific GitHub release tag instead of the latest release.
        #[arg(long)]
        version: Option<String>,
        /// Update channel: stable, beta, or rc.
        /// Overrides the `update_channel` setting in config.toml.
        #[arg(long)]
        channel: Option<String>,
    },
    /// Stop the running daemon.
    #[command(
        long_about = "Stop the running LibreFang daemon.\n\nSends a shutdown signal to the background daemon process. If no daemon is\nrunning, this is a no-op.\n\nExamples:\n  librefang stop"
    )]
    Stop,
    /// Manage agents (new, list, chat, kill, spawn) [*].
    #[command(
        subcommand,
        long_about = "Manage agents: create, list, chat, kill, and configure.\n\nExamples:\n  librefang agent new              # Interactive template picker\n  librefang agent new coder        # Spawn from template\n  librefang agent list             # List all agents\n  librefang agent chat <ID>        # Chat with an agent\n  librefang agent kill <ID>        # Kill an agent\n  librefang agent set <ID> model gpt-4o  # Change agent model"
    )]
    Agent(AgentCommands),
    /// Manage workflows (list, create, run) [*].
    #[command(
        subcommand,
        long_about = "Manage multi-step workflows that chain agents together.\n\nExamples:\n  librefang workflow list                      # List workflows\n  librefang workflow create workflow.json      # Create from file\n  librefang workflow run <ID> \"summarize this\" # Run a workflow"
    )]
    Workflow(WorkflowCommands),
    /// Manage event triggers (list, create, delete) [*].
    #[command(
        subcommand,
        long_about = "Manage event triggers that fire agents on system events.\n\nTriggers let agents react to lifecycle events, other agents spawning, or\ncustom patterns.\n\nExamples:\n  librefang trigger list                   # List all triggers\n  librefang trigger list --agent-id <ID>   # Filter by agent\n  librefang trigger create <AGENT_ID> '\"lifecycle\"' --prompt \"Event: {{event}}\"\n  librefang trigger delete <TRIGGER_ID>"
    )]
    Trigger(TriggerCommands),
    /// Migrate from another agent framework to LibreFang.
    #[command(
        long_about = "Migrate agents and configuration from another framework to LibreFang.\n\nSupported sources: openclaw, langchain, autogpt.\n\nExamples:\n  librefang migrate --from langchain\n  librefang migrate --from autogpt --source-dir ./my-agents\n  librefang migrate --from openclaw --dry-run  # Preview changes"
    )]
    Migrate(MigrateArgs),
    /// Manage skills (install, list, search, create, remove) [*].
    #[command(
        subcommand,
        long_about = "Manage agent skills: install from FangHub, list, search, test, and publish.\n\nSkills extend agent capabilities with tools, integrations, and custom logic.\n\nExamples:\n  librefang skill install web-search   # Install from FangHub\n  librefang skill list                 # List installed skills\n  librefang skill search \"code review\" # Search FangHub\n  librefang skill test ./my-skill      # Validate a local skill\n  librefang skill create               # Scaffold a new skill\n  librefang skill publish              # Publish to FangHub"
    )]
    Skill(SkillCommands),
    /// Manage channel integrations (setup, test, enable, disable) [*].
    #[command(
        subcommand,
        long_about = "Manage messaging channel integrations (Telegram, Discord, Slack, etc.).\n\nChannels connect your agents to external messaging platforms.\n\nExamples:\n  librefang channel list              # Show configured channels\n  librefang channel setup telegram    # Interactive Telegram setup\n  librefang channel setup             # Interactive channel picker\n  librefang channel test telegram     # Send a test message\n  librefang channel enable telegram   # Enable a channel\n  librefang channel disable telegram  # Disable without removing config"
    )]
    Channel(ChannelCommands),
    /// Manage hands (list, activate, status, pause, info) [*].
    #[command(
        subcommand,
        long_about = "Manage hands (autonomous execution modules for agents).\n\nHands give agents the ability to take actions in the real world, such as\nbrowsing the web, managing files, or interacting with APIs.\n\nExamples:\n  librefang hand list                # List available hands\n  librefang hand active              # Show active hand instances\n  librefang hand activate clip       # Activate a hand by ID\n  librefang hand deactivate clip     # Deactivate a hand\n  librefang hand info clip           # Show hand details\n  librefang hand check-deps clip     # Check dependencies\n  librefang hand install-deps clip   # Install missing deps\n  librefang hand pause clip          # Pause a running hand\n  librefang hand resume clip         # Resume a paused hand"
    )]
    Hand(HandCommands),
    /// Show or edit configuration (show, edit, get, set, keys) [*].
    #[command(
        subcommand,
        long_about = "Show, edit, and manage the LibreFang configuration.\n\nExamples:\n  librefang config show                           # Print current config\n  librefang config edit                           # Open in $EDITOR\n  librefang config get default_model.provider     # Get a value\n  librefang config set api_listen 0.0.0.0:8080    # Set a value\n  librefang config unset api.cors_origin          # Remove a key\n  librefang config set-key groq                   # Save API key interactively\n  librefang config delete-key groq                # Remove an API key\n  librefang config test-key groq                  # Test connectivity"
    )]
    Config(ConfigCommands),
    /// Quick chat with the default agent.
    #[command(
        long_about = "Start an interactive chat session with the default agent.\n\nOptionally specify an agent name or ID to chat with a specific agent.\nType your messages and press Enter; Ctrl+C or Ctrl+D to exit.\n\nExamples:\n  librefang chat              # Chat with the default agent\n  librefang chat coder        # Chat with the \"coder\" agent\n  librefang chat 550e8400...  # Chat with an agent by ID"
    )]
    Chat {
        /// Optional agent name or ID to chat with.
        agent: Option<String>,
    },
    /// Show kernel status.
    #[command(
        long_about = "Show the current status of the LibreFang kernel daemon.\n\nDisplays uptime, version, default provider/model, health checks, and (when an\n`api_key` is configured) agent list, sessions, and memory usage. Without a key\nthe command still works — only the protected detail section is hidden.\n\nExit codes:\n  0  daemon running and healthy\n  1  daemon not running (in-process fallback)\n  2  daemon running but reporting a degraded status\n  3  daemon port claims to be open but /api/health is unreachable\n\nExamples:\n  librefang status             # Pretty-printed status\n  librefang status --json      # JSON output for scripting\n  librefang status -v          # Verbose: config warnings, auth, MCP, peers\n  librefang status -q          # Quiet: one-line summary\n  librefang status --watch 2   # Refresh every 2 seconds (Ctrl+C to stop)"
    )]
    Status {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
        /// Verbose mode: include config warnings, auth mode, MCP server list, peers.
        #[arg(long, short = 'v', conflicts_with_all = ["quiet", "json"])]
        verbose: bool,
        /// Quiet mode: single-line summary, no section layout.
        #[arg(long, short = 'q', conflicts_with_all = ["verbose", "json"])]
        quiet: bool,
        /// Refresh every N seconds until Ctrl+C. Conflicts with --json / --quiet.
        #[arg(long, value_name = "SECS", conflicts_with_all = ["json", "quiet"])]
        watch: Option<u64>,
    },
    /// Run diagnostic health checks.
    #[command(
        long_about = "Run diagnostic health checks on your LibreFang installation.\n\nChecks config files, data directories, API keys, daemon connectivity,\nand installed dependencies. Use --repair to auto-fix common issues.\n\nExamples:\n  librefang doctor            # Run all checks\n  librefang doctor --repair   # Auto-fix missing dirs/config\n  librefang doctor --json     # JSON output for CI pipelines"
    )]
    Doctor {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
        /// Attempt to auto-fix issues (create missing dirs/config).
        #[arg(long)]
        repair: bool,
    },
    /// Open the web dashboard in the default browser.
    #[command(
        long_about = "Open the LibreFang web dashboard in your default browser.\n\nRequires the daemon to be running (serves at http://127.0.0.1:4545/ by default).\n\nExamples:\n  librefang dashboard"
    )]
    Dashboard,
    /// Generate shell completion scripts.
    #[command(
        long_about = "Generate shell completion scripts for your shell.\n\nOutput the completion script to stdout. Redirect to a file and source it\nin your shell profile.\n\nExamples:\n  librefang completion bash > ~/.bashrc.d/librefang.bash\n  librefang completion zsh  > ~/.zfunc/_librefang\n  librefang completion fish > ~/.config/fish/completions/librefang.fish"
    )]
    Completion {
        /// Shell to generate completions for.
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// MCP (Model Context Protocol) server management.
    #[command(
        long_about = "Manage MCP (Model Context Protocol) servers.\n\nCalled without a subcommand, starts the stdio MCP server that exposes\nLibreFang to MCP-compatible clients (Claude Code, Cursor, ...).\n\nExamples:\n  librefang mcp                    # Start the stdio MCP server\n  librefang mcp list               # List configured MCP servers\n  librefang mcp catalog            # List installable catalog entries\n  librefang mcp add github         # Install the 'github' catalog entry\n  librefang mcp add slack --key xoxb-...  # Provide key inline\n  librefang mcp remove github      # Remove an MCP server by id"
    )]
    Mcp {
        #[command(subcommand)]
        command: Option<McpCommands>,
    },
    /// Authenticate with a provider (chatgpt) [*].
    #[command(
        subcommand,
        long_about = "Authenticate with external providers.\n\nExamples:\n  librefang auth chatgpt\n  librefang auth chatgpt --device-auth"
    )]
    Auth(AuthCommands),
    /// Manage the credential vault (init, set, list, remove) [*].
    #[command(
        subcommand,
        long_about = "Manage the encrypted credential vault for storing API keys and tokens.\n\nExamples:\n  librefang vault init            # Initialize the vault\n  librefang vault set GROQ_API_KEY  # Store a credential (prompts for value)\n  librefang vault list            # List stored keys (values hidden)\n  librefang vault remove GROQ_API_KEY  # Remove a credential"
    )]
    Vault(VaultCommands),
    /// Scaffold a new skill or MCP server template.
    #[command(
        long_about = "Scaffold a new skill or MCP server template.\n\nCreates boilerplate files for developing a custom skill or MCP server.\n\nExamples:\n  librefang new skill   # Scaffold a new skill\n  librefang new mcp     # Scaffold a new MCP server"
    )]
    New {
        /// What to scaffold.
        #[arg(value_enum)]
        kind: ScaffoldKind,
    },
    /// Launch the interactive terminal dashboard.
    #[command(
        long_about = "Launch the interactive terminal dashboard (TUI).\n\nProvides a full-screen terminal interface for managing agents, viewing logs,\nand monitoring system status.\n\nExamples:\n  librefang tui"
    )]
    Tui,
    /// Browse models, aliases, and providers [*].
    #[command(
        subcommand,
        long_about = "Browse and manage LLM models, aliases, and providers.\n\nExamples:\n  librefang models list                  # List all models\n  librefang models list --provider groq  # Filter by provider\n  librefang models aliases               # Show model aliases\n  librefang models providers             # List providers and auth status\n  librefang models set gpt-4o            # Set default model"
    )]
    Models(ModelsCommands),
    /// Daemon control (start, stop, status) [*].
    #[command(
        subcommand,
        long_about = "Low-level daemon control commands.\n\nExamples:\n  librefang gateway start          # Start the daemon\n  librefang gateway stop           # Stop the daemon\n  librefang gateway restart        # Restart the daemon\n  librefang gateway status         # Show daemon status"
    )]
    Gateway(GatewayCommands),
    /// Manage execution approvals (list, approve, reject) [*].
    #[command(
        subcommand,
        long_about = "Manage execution approvals for agent actions that require human review.\n\nWhen agents request to perform sensitive operations, approval requests are\nqueued here for human review.\n\nExamples:\n  librefang approvals list          # List pending approvals\n  librefang approvals approve <ID>  # Approve a request\n  librefang approvals reject <ID>   # Reject a request"
    )]
    Approvals(ApprovalsCommands),
    /// Manage scheduled jobs (list, create, delete, enable, disable) [*].
    #[command(
        subcommand,
        long_about = "Manage cron-style scheduled jobs that run agents on a recurring basis.\n\nExamples:\n  librefang cron list\n  librefang cron create my-agent \"0 */6 * * *\" \"Check for updates\"\n  librefang cron create my-agent \"0 9 * * 1\" \"Weekly report\" --name weekly-report\n  librefang cron enable <ID>\n  librefang cron disable <ID>\n  librefang cron delete <ID>"
    )]
    Cron(CronCommands),
    /// List conversation sessions.
    #[command(
        long_about = "List conversation sessions stored by agents.\n\nOptionally filter by agent name or ID.\n\nExamples:\n  librefang sessions              # List all sessions\n  librefang sessions coder        # Filter by agent name\n  librefang sessions --json       # JSON output for scripting"
    )]
    Sessions {
        /// Optional agent name or ID to filter by.
        agent: Option<String>,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Tail the LibreFang log file.
    #[command(
        long_about = "Tail the LibreFang daemon log file.\n\nShows recent log lines and optionally follows new output in real time.\n\nExamples:\n  librefang logs                  # Show last 50 lines\n  librefang logs --lines 100      # Show last 100 lines\n  librefang logs -f                # Follow log output\n  librefang logs --lines 20 -f    # Show 20 lines then follow"
    )]
    Logs {
        /// Number of lines to show.
        #[arg(long, default_value = "50")]
        lines: usize,
        /// Follow log output in real time.
        #[arg(long, short)]
        follow: bool,
    },
    /// Quick daemon health check.
    #[command(
        long_about = "Perform a quick health check on the running daemon.\n\nReturns basic connectivity and status info. For comprehensive diagnostics,\nuse `librefang doctor` instead.\n\nExamples:\n  librefang health          # Pretty-printed output\n  librefang health --json   # JSON output for monitoring"
    )]
    Health {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Security tools and audit trail [*].
    #[command(
        subcommand,
        long_about = "Security tools: view status, audit trail, and verify integrity.\n\nExamples:\n  librefang security status          # Security summary\n  librefang security audit           # Show recent audit entries\n  librefang security audit --limit 50  # Show more entries\n  librefang security verify          # Verify Merkle chain integrity"
    )]
    Security(SecurityCommands),
    /// Inspect storage and migrate between backends [*].
    ///
    /// Phase 7 of `surrealdb-storage-swap`. Surfaces the SurrealDB
    /// db-explorer and the SQLite → SurrealDB migrator. Available only
    /// when the binary is built with `surreal-backend` (default).
    #[cfg(feature = "surreal-backend")]
    #[command(
        subcommand,
        long_about = "Inspect the embedded SurrealDB store and migrate legacy SQLite\ndata into it.\n\nExamples:\n  librefang storage explore --limit 50          # peek at audit_entries\n  librefang storage migrate --from sqlite --to surreal --dry-run\n  librefang storage migrate --from sqlite --to surreal"
    )]
    Storage(StorageCommands),
    /// Search and manage agent memory (KV store) [*].
    #[command(
        subcommand,
        long_about = "Search and manage agent memory (key-value store).\n\nEach agent has its own KV namespace for persisting data across sessions.\n\nExamples:\n  librefang memory list coder          # List all keys for \"coder\" agent\n  librefang memory get coder my-key    # Get a specific value\n  librefang memory set coder my-key \"hello\"  # Set a value\n  librefang memory delete coder my-key       # Delete a key"
    )]
    Memory(MemoryCommands),
    /// Device pairing and token management [*].
    #[command(
        subcommand,
        long_about = "Manage paired devices and remote access tokens.\n\nExamples:\n  librefang devices list          # List paired devices\n  librefang devices pair          # Start pairing flow\n  librefang devices remove <ID>   # Remove a device"
    )]
    Devices(DevicesCommands),
    /// Generate device pairing QR code.
    #[command(
        long_about = "Generate a QR code for pairing a mobile device.\n\nDisplays a QR code in the terminal that can be scanned to pair a device.\n\nExamples:\n  librefang qr"
    )]
    Qr,
    /// Webhook helpers and trigger management [*].
    #[command(
        subcommand,
        long_about = "Manage webhook triggers that invoke agents via HTTP callbacks.\n\nExamples:\n  librefang webhooks list                          # List webhooks\n  librefang webhooks create coder https://...      # Create a webhook\n  librefang webhooks test <ID>                     # Send test payload\n  librefang webhooks delete <ID>                   # Delete a webhook"
    )]
    Webhooks(WebhooksCommands),
    /// Interactive onboarding wizard.
    #[command(
        long_about = "Run the interactive onboarding wizard.\n\nWalks you through initial configuration: API keys, default model, channels,\nand your first agent.\n\nExamples:\n  librefang onboard          # Full interactive wizard\n  librefang onboard --quick  # Non-interactive quick setup"
    )]
    Onboard {
        /// Quick non-interactive mode.
        #[arg(long, conflicts_with = "upgrade")]
        quick: bool,
        /// Upgrade an existing installation.
        #[arg(long, conflicts_with = "quick")]
        upgrade: bool,
    },
    /// Quick non-interactive initialization.
    #[command(
        long_about = "Quick non-interactive initialization (alias for `init --quick`).\n\nWrites default config and data directories without prompts.\n\nExamples:\n  librefang setup          # Quick init\n  librefang setup --quick  # Same behavior"
    )]
    Setup {
        /// Quick mode (same as `init --quick`).
        #[arg(long, conflicts_with = "upgrade")]
        quick: bool,
        /// Upgrade an existing installation.
        #[arg(long, conflicts_with = "quick")]
        upgrade: bool,
    },
    /// Interactive setup wizard for credentials and channels.
    #[command(
        long_about = "Launch the interactive setup wizard for credentials and channels.\n\nGuides you through configuring API keys, messaging channels, and other\nintegration settings.\n\nExamples:\n  librefang configure"
    )]
    Configure,
    /// Send a one-shot message to an agent.
    #[command(
        long_about = "Send a single message to an agent and print the response.\n\nUnlike `chat`, this does not start an interactive session. Useful for\nscripting and automation.\n\nExamples:\n  librefang message coder \"Fix the bug in main.rs\"\n  librefang message coder \"Summarize this file\" --json"
    )]
    Message {
        /// Agent name or ID.
        agent: String,
        /// Message text.
        text: String,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// System info and version [*].
    #[command(
        subcommand,
        long_about = "Display system information and version details.\n\nExamples:\n  librefang system info          # Detailed system info\n  librefang system version       # Version information"
    )]
    System(SystemCommands),
    /// Manage boot service (systemd/launchd/Windows autostart) [*].
    #[command(
        subcommand,
        long_about = "Install, remove, or check the status of a system boot service so LibreFang\nstarts automatically on login/boot.\n\nExamples:\n  librefang service install      # Register auto-start service\n  librefang service uninstall    # Remove auto-start service\n  librefang service status       # Check if the service is registered"
    )]
    Service(ServiceCommands),
    /// Reset local config and state.
    #[command(
        long_about = "Reset local configuration and state to defaults.\n\nRemoves the ~/.librefang/ directory and all its contents. You will be\nprompted for confirmation unless --confirm is passed.\n\nExamples:\n  librefang reset            # Interactive confirmation\n  librefang reset --confirm  # Skip confirmation (for scripts)"
    )]
    Reset {
        /// Skip confirmation prompt.
        #[arg(long)]
        confirm: bool,
    },
    /// Completely uninstall LibreFang from your system.
    #[command(
        long_about = "Completely uninstall LibreFang from your system.\n\nRemoves the binary, data directory, config files, and all related state.\nUse --keep-config to preserve config.toml, .env, and secrets.env.\n\nExamples:\n  librefang uninstall                     # Interactive confirmation\n  librefang uninstall --confirm           # Skip confirmation\n  librefang uninstall --confirm --keep-config  # Keep config files"
    )]
    Uninstall {
        /// Skip confirmation prompt (also --yes).
        #[arg(long, alias = "yes")]
        confirm: bool,
        /// Keep config files (config.toml, .env, secrets.env).
        #[arg(long)]
        keep_config: bool,
    },
    /// Generate an Argon2id password hash for dashboard authentication.
    #[command(
        name = "hash-password",
        long_about = "Generate an Argon2id password hash for use with dashboard_pass_hash in config.toml.\n\nIf --password is not provided, prompts for interactive input.\n\nExamples:\n  librefang hash-password                       # Interactive prompt\n  librefang hash-password --password 'secret'   # Inline (less secure, visible in shell history)"
    )]
    HashPassword {
        /// Password to hash (omit for interactive prompt).
        #[arg(long)]
        password: Option<String>,
    },
}

#[derive(Subcommand)]
enum VaultCommands {
    /// Initialize the credential vault.
    #[command(
        long_about = "Initialize the encrypted credential vault.\n\nCreates the vault storage file if it does not exist.\n\nExamples:\n  librefang vault init"
    )]
    Init,
    /// Store a credential in the vault.
    #[command(
        long_about = "Store a credential in the vault (prompts for the value securely).\n\nExamples:\n  librefang vault set GROQ_API_KEY\n  librefang vault set OPENAI_API_KEY"
    )]
    Set {
        /// Credential key (env var name).
        key: String,
    },
    /// List all keys in the vault (values are hidden).
    #[command(
        long_about = "List all credential keys stored in the vault.\n\nValues are hidden for security; only key names are displayed.\n\nExamples:\n  librefang vault list"
    )]
    List,
    /// Remove a credential from the vault.
    #[command(
        long_about = "Remove a credential from the vault by key name.\n\nExamples:\n  librefang vault remove GROQ_API_KEY"
    )]
    Remove {
        /// Credential key.
        key: String,
    },
}

#[derive(Subcommand)]
enum AuthCommands {
    /// Authenticate with ChatGPT using browser or device auth.
    #[command(
        long_about = "Authenticate with ChatGPT using the OpenAI Codex login flow.\n\nBy default this opens a browser and waits for the localhost callback.\nUse --device-auth for headless or remote environments. If device auth is\nnot enabled for the current OpenAI account or workspace, LibreFang falls\nback to the standard browser login flow.\n\nExamples:\n  librefang auth chatgpt\n  librefang auth chatgpt --device-auth"
    )]
    Chatgpt {
        /// Use the OpenAI device auth flow before falling back to browser auth.
        #[arg(long)]
        device_auth: bool,
    },
}

#[derive(Clone, clap::ValueEnum)]
enum ScaffoldKind {
    Skill,
    Mcp,
}

#[derive(Subcommand)]
enum McpCommands {
    /// List configured MCP servers (reads config.toml).
    #[command(long_about = "List every MCP server currently in config.toml with its status.")]
    List,
    /// List or search the catalog of installable MCP templates.
    #[command(
        long_about = "List or search the read-only MCP catalog.\n\nExamples:\n  librefang mcp catalog           # List all catalog entries\n  librefang mcp catalog \"code\"   # Search"
    )]
    Catalog {
        /// Search query.
        query: Option<String>,
    },
    /// Install a catalog entry as a new MCP server.
    #[command(
        long_about = "Install a catalog entry as a new MCP server. Writes a new \
[[mcp_servers]] entry to config.toml. If the daemon is running, it hot-reloads.\n\nExamples:\n  librefang mcp add github\n  librefang mcp add slack --key xoxb-..."
    )]
    Add {
        /// Catalog id.
        name: String,
        /// API key or token to store in the vault.
        #[arg(long)]
        key: Option<String>,
    },
    /// Remove a configured MCP server by id.
    #[command(
        long_about = "Remove a configured MCP server by id.\n\nExamples:\n  librefang mcp remove github"
    )]
    Remove {
        /// MCP server id.
        name: String,
    },
}

#[derive(clap::Args)]
struct MigrateArgs {
    /// Source framework to migrate from.
    #[arg(long, value_enum)]
    from: MigrateSourceArg,
    /// Path to the source workspace (auto-detected if not set).
    #[arg(long)]
    source_dir: Option<PathBuf>,
    /// Dry run — show what would be imported without making changes.
    #[arg(long)]
    dry_run: bool,
}

#[derive(clap::Args)]
struct SpawnAliasArgs {
    /// Template name (e.g. "coder") or manifest path. Interactive picker if omitted.
    target: Option<String>,
    /// Explicit manifest path (legacy alias for a template file path).
    #[arg(long)]
    template: Option<PathBuf>,
    /// Override the agent name before spawning.
    #[arg(long)]
    name: Option<String>,
    /// Parse and preview the manifest without spawning an agent.
    #[arg(long)]
    dry_run: bool,
}

#[derive(clap::Args)]
struct AgentSpawnArgs {
    /// Path to the agent manifest TOML file.
    manifest: PathBuf,
    /// Override the agent name before spawning.
    #[arg(long)]
    name: Option<String>,
    /// Parse and preview the manifest without spawning an agent.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Clone, clap::ValueEnum)]
enum MigrateSourceArg {
    Openclaw,
    Langchain,
    Autogpt,
    Openfang,
}

#[derive(Subcommand)]
enum SkillCommands {
    /// Install a skill from FangHub or a local directory.
    #[command(
        long_about = "Install a skill from FangHub, a local directory, or a git URL.\n\nExamples:\n  librefang skill install web-search\n  librefang skill install ./my-skill\n  librefang skill install https://github.com/user/skill.git"
    )]
    Install {
        /// Skill name, local path, or git URL.
        source: String,
        /// Install into a specific hand's workspace instead of globally.
        #[arg(long)]
        hand: Option<String>,
    },
    /// List installed skills.
    #[command(
        long_about = "List all skills currently installed in this LibreFang instance.\n\nExamples:\n  librefang skill list\n  librefang skill list --hand clip"
    )]
    List {
        /// List skills installed in a specific hand's workspace.
        #[arg(long)]
        hand: Option<String>,
    },
    /// Remove an installed skill.
    #[command(
        long_about = "Remove an installed skill by name.\n\nExamples:\n  librefang skill remove web-search\n  librefang skill remove web-search --hand clip"
    )]
    Remove {
        /// Skill name.
        name: String,
        /// Remove from a specific hand's workspace instead of globally.
        #[arg(long)]
        hand: Option<String>,
    },
    /// Search FangHub for skills.
    #[command(
        long_about = "Search the FangHub registry for available skills.\n\nExamples:\n  librefang skill search \"web scraping\"\n  librefang skill search github"
    )]
    Search {
        /// Search query.
        query: String,
    },
    /// Validate a local skill and optionally execute one tool.
    #[command(
        long_about = "Validate a local skill manifest and optionally execute one of its tools.\n\nDefaults to the current directory if no path is given. Runs the first\ndeclared tool unless --tool is specified.\n\nExamples:\n  librefang skill test                              # Test skill in cwd\n  librefang skill test ./my-skill                   # Test specific skill\n  librefang skill test --tool search --input '{}'   # Run a specific tool"
    )]
    Test {
        /// Skill directory, skill.toml, SKILL.md, or package.json. Defaults to the current directory.
        path: Option<PathBuf>,
        /// Tool name to execute after validation. Defaults to the first declared tool.
        #[arg(long)]
        tool: Option<String>,
        /// JSON input payload passed to the selected tool.
        #[arg(long)]
        input: Option<String>,
    },
    /// Package a local skill and publish it to a FangHub GitHub release.
    #[command(
        long_about = "Package a local skill and publish it to a FangHub GitHub release.\n\nBundles the skill into a zip file and uploads it as a GitHub release asset.\nUse --dry-run to validate and package without uploading.\n\nExamples:\n  librefang skill publish\n  librefang skill publish ./my-skill\n  librefang skill publish --repo myorg/my-skill --tag v1.0.0\n  librefang skill publish --dry-run"
    )]
    Publish {
        /// Skill directory, skill.toml, SKILL.md, or package.json. Defaults to the current directory.
        path: Option<PathBuf>,
        /// Target GitHub repo in owner/name form. Defaults to librefang-skills/<skill-name>.
        #[arg(long)]
        repo: Option<String>,
        /// Release tag to create or update. Defaults to v<skill-version>.
        #[arg(long)]
        tag: Option<String>,
        /// Output directory for the generated bundle zip. Defaults to <skill-dir>/dist.
        #[arg(long)]
        output: Option<PathBuf>,
        /// Validate and package locally without uploading to GitHub.
        #[arg(long)]
        dry_run: bool,
    },
    /// Create a new skill scaffold.
    #[command(
        long_about = "Scaffold a new skill project with boilerplate files.\n\nCreates a skill.toml, SKILL.md, and starter tool implementation.\n\nExamples:\n  librefang skill create"
    )]
    Create,
    /// Agent-driven skill evolution — create/update/patch/rollback installed skills.
    #[command(
        subcommand,
        long_about = "Manually invoke the skill evolution pipeline that agents use internally.\n\nOperates on the globally-installed skill directory (~/.librefang/skills).\nAll mutations go through the same validation, security scan, file locking,\nand version-history bookkeeping as the agent tools.\n\nExamples:\n  librefang skill evolve create --name my-skill --description ... --context-file prompt.md\n  librefang skill evolve update my-skill prompt.md --changelog \"tightened wording\"\n  librefang skill evolve patch my-skill --old-file a.txt --new-file b.txt --changelog \"fix typo\"\n  librefang skill evolve rollback my-skill\n  librefang skill evolve history my-skill"
    )]
    Evolve(EvolveCommands),
}

#[derive(Subcommand)]
enum EvolveCommands {
    /// Create a new prompt-only skill from a Markdown file.
    Create {
        /// Skill name (lowercase alphanumeric + hyphens).
        #[arg(long)]
        name: String,
        /// One-line description (≤1024 chars).
        #[arg(long)]
        description: String,
        /// File containing the Markdown prompt_context. Use "-" for stdin.
        #[arg(long = "context-file")]
        context_file: PathBuf,
        /// Comma-separated tags (e.g., "data,csv,analysis").
        #[arg(long, default_value = "")]
        tags: String,
        /// Target a specific hand's workspace instead of the global skills dir.
        #[arg(long)]
        hand: Option<String>,
    },
    /// Fully rewrite a skill's prompt_context from a file.
    Update {
        /// Skill name.
        name: String,
        /// File containing the new prompt_context. Use "-" for stdin.
        context_file: PathBuf,
        /// Brief description of what changed and why.
        #[arg(long)]
        changelog: String,
        /// Target a specific hand's workspace instead of the global skills dir.
        #[arg(long)]
        hand: Option<String>,
    },
    /// Find-and-replace patch a skill's prompt_context (fuzzy-matched).
    Patch {
        /// Skill name.
        name: String,
        /// File containing the text to find.
        #[arg(long = "old-file")]
        old_file: PathBuf,
        /// File containing the replacement text.
        #[arg(long = "new-file")]
        new_file: PathBuf,
        /// Brief description of what changed and why.
        #[arg(long)]
        changelog: String,
        /// Replace every occurrence (default: require unique match).
        #[arg(long)]
        replace_all: bool,
        /// Target a specific hand's workspace instead of the global skills dir.
        #[arg(long)]
        hand: Option<String>,
    },
    /// Delete a locally-evolved skill.
    Delete {
        /// Skill name.
        name: String,
        /// Target a specific hand's workspace instead of the global skills dir.
        #[arg(long)]
        hand: Option<String>,
    },
    /// Roll back the most recent evolution of a skill.
    Rollback {
        /// Skill name.
        name: String,
        /// Target a specific hand's workspace instead of the global skills dir.
        #[arg(long)]
        hand: Option<String>,
    },
    /// Add a supporting file to a skill (under references/, templates/, scripts/, or assets/).
    WriteFile {
        /// Skill name.
        name: String,
        /// Relative path under the skill directory (e.g., references/api.md).
        path: String,
        /// Source file whose contents will be copied. Use "-" for stdin.
        source: PathBuf,
        /// Target a specific hand's workspace instead of the global skills dir.
        #[arg(long)]
        hand: Option<String>,
    },
    /// Remove a supporting file from a skill.
    RemoveFile {
        /// Skill name.
        name: String,
        /// Relative path of the file to remove.
        path: String,
        /// Target a specific hand's workspace instead of the global skills dir.
        #[arg(long)]
        hand: Option<String>,
    },
    /// Print the version history and usage counters for a skill.
    History {
        /// Skill name.
        name: String,
        /// Emit JSON instead of a human-readable table.
        #[arg(long)]
        json: bool,
        /// Target a specific hand's workspace instead of the global skills dir.
        #[arg(long)]
        hand: Option<String>,
    },
}

#[derive(Subcommand)]
enum ChannelCommands {
    /// List configured channels and their status.
    #[command(
        long_about = "List all configured channels and show their current status (enabled/disabled).\n\nExamples:\n  librefang channel list"
    )]
    List,
    /// Interactive setup wizard for a channel.
    #[command(
        long_about = "Run the interactive setup wizard for a messaging channel.\n\nIf no channel name is given, shows an interactive picker.\n\nExamples:\n  librefang channel setup            # Interactive picker\n  librefang channel setup telegram   # Set up Telegram\n  librefang channel setup discord    # Set up Discord"
    )]
    Setup {
        /// Channel name (telegram, discord, slack, whatsapp, etc.). Shows picker if omitted.
        channel: Option<String>,
    },
    /// Test a channel by sending a test message.
    #[command(
        long_about = "Send a test message through a configured channel to verify connectivity.\n\nExamples:\n  librefang channel test telegram\n  librefang channel test telegram --chat-id 123456789\n  librefang channel test discord --channel 123456789\n  librefang channel test slack --channel C1234567890"
    )]
    Test {
        /// Channel name.
        #[arg(value_name = "NAME")]
        name: String,
        /// Target channel ID for Discord or Slack live message tests.
        #[arg(long = "channel", conflicts_with = "chat_id")]
        channel_id: Option<String>,
        /// Target chat ID for Telegram live message tests.
        #[arg(long, conflicts_with = "channel_id")]
        chat_id: Option<String>,
    },
    /// Enable a channel.
    #[command(
        long_about = "Enable a previously configured channel.\n\nExamples:\n  librefang channel enable telegram"
    )]
    Enable {
        /// Channel name.
        channel: String,
    },
    /// Disable a channel without removing its configuration.
    #[command(
        long_about = "Disable a channel without removing its configuration.\n\nThe channel can be re-enabled later without reconfiguring.\n\nExamples:\n  librefang channel disable telegram"
    )]
    Disable {
        /// Channel name.
        channel: String,
    },
}

#[derive(Subcommand)]
enum HandCommands {
    /// List all available hands.
    #[command(
        long_about = "List all available hands (autonomous execution modules).\n\nExamples:\n  librefang hand list"
    )]
    List,
    /// Show currently active hand instances.
    #[command(
        long_about = "Show currently active hand instances and their runtime state.\n\nExamples:\n  librefang hand active"
    )]
    Active,
    /// Show active status for a hand or hand instance.
    #[command(
        long_about = "Show active status for a specific hand or all active hands.\n\nExamples:\n  librefang hand status          # Show all active hands\n  librefang hand status clip     # Show status for \"clip\" hand"
    )]
    Status {
        /// Optional hand ID or instance ID. Shows all active hands if omitted.
        id: Option<String>,
    },
    /// Install a hand from a local directory containing HAND.toml.
    #[command(
        long_about = "Install a hand from a local directory.\n\nThe directory must contain a HAND.toml manifest file.\n\nExamples:\n  librefang hand install ./my-hand"
    )]
    Install {
        /// Path to the hand directory (must contain HAND.toml).
        path: String,
    },
    /// Activate a hand by ID.
    #[command(
        long_about = "Activate a hand, making it available for agent use.\n\nExamples:\n  librefang hand activate clip\n  librefang hand activate researcher"
    )]
    Activate {
        /// Hand ID (e.g. "clip", "lead", "researcher").
        id: String,
    },
    /// Deactivate an active hand by hand ID.
    #[command(
        long_about = "Deactivate a running hand, stopping its execution.\n\nExamples:\n  librefang hand deactivate clip"
    )]
    Deactivate {
        /// Hand ID.
        id: String,
    },
    /// Show detailed info about a hand.
    #[command(
        long_about = "Show detailed information about a hand including its capabilities,\ndependencies, and configuration.\n\nExamples:\n  librefang hand info clip"
    )]
    Info {
        /// Hand ID.
        id: String,
    },
    /// Check dependency status for a hand.
    #[command(
        long_about = "Check whether all required dependencies for a hand are installed.\n\nExamples:\n  librefang hand check-deps clip"
    )]
    CheckDeps {
        /// Hand ID.
        id: String,
    },
    /// Install missing dependencies for a hand.
    #[command(
        long_about = "Install any missing dependencies required by a hand.\n\nExamples:\n  librefang hand install-deps clip"
    )]
    InstallDeps {
        /// Hand ID.
        id: String,
    },
    /// Pause a running hand by hand ID or instance ID.
    #[command(
        long_about = "Pause a running hand without fully deactivating it.\n\nThe hand can be resumed later with `hand resume`.\n\nExamples:\n  librefang hand pause clip"
    )]
    Pause {
        /// Hand ID or instance ID.
        id: String,
    },
    /// Resume a paused hand by hand ID or instance ID.
    #[command(
        long_about = "Resume a previously paused hand.\n\nExamples:\n  librefang hand resume clip"
    )]
    Resume {
        /// Hand ID or instance ID.
        id: String,
    },
    /// Show current settings for a hand.
    #[command(
        long_about = "Show the current settings/configuration for a hand.\n\nExamples:\n  librefang hand settings clip"
    )]
    Settings {
        /// Hand ID.
        id: String,
    },
    /// Set a configuration value for a hand.
    #[command(
        long_about = "Set a configuration key-value pair for a hand.\n\nExamples:\n  librefang hand set clip interval 30m\n  librefang hand set researcher max_results 20"
    )]
    Set {
        /// Hand ID.
        id: String,
        /// Configuration key.
        key: String,
        /// Configuration value.
        value: String,
    },
    /// Reload hand definitions from disk.
    #[command(
        long_about = "Reload all hand definitions from ~/.librefang/hands/ without restarting.\n\nPicks up newly added or modified HAND.toml files.\n\nExamples:\n  librefang hand reload"
    )]
    Reload,
    /// Chat with an active hand interactively.
    #[command(
        long_about = "Start an interactive chat session with an active hand.\n\nThe hand must be activated first. Type your messages and press Enter.\nType /quit or Ctrl+C to exit.\n\nExamples:\n  librefang hand chat clip\n  librefang hand chat researcher"
    )]
    Chat {
        /// Hand ID (e.g. "clip", "researcher").
        id: String,
    },
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// Show the current configuration.
    #[command(
        long_about = "Print the current LibreFang configuration to stdout.\n\nExamples:\n  librefang config show"
    )]
    Show,
    /// Open the configuration file in your editor.
    #[command(
        long_about = "Open ~/.librefang/config.toml in your default $EDITOR.\n\nExamples:\n  librefang config edit"
    )]
    Edit,
    /// Get a config value by dotted key path (e.g. "default_model.provider").
    #[command(
        long_about = "Get a single configuration value by its dotted key path.\n\nExamples:\n  librefang config get default_model.provider\n  librefang config get api_listen"
    )]
    Get {
        /// Dotted key path (e.g. "default_model.provider", "api_listen").
        key: String,
    },
    /// Set a config value (warning: strips TOML comments).
    #[command(
        long_about = "Set a configuration value by dotted key path.\n\nNote: This rewrites the TOML file and will strip comments.\n\nExamples:\n  librefang config set api_listen 0.0.0.0:8080\n  librefang config set default_model.provider groq"
    )]
    Set {
        /// Dotted key path.
        key: String,
        /// New value.
        value: String,
    },
    /// Remove a config key (warning: strips TOML comments).
    #[command(
        long_about = "Remove a configuration key from config.toml.\n\nNote: This rewrites the TOML file and will strip comments.\n\nExamples:\n  librefang config unset api.cors_origin"
    )]
    Unset {
        /// Dotted key path to remove (e.g. "api.cors_origin").
        key: String,
    },
    /// Save an API key to ~/.librefang/.env (prompts interactively).
    #[command(
        long_about = "Save an API key for a provider to ~/.librefang/.env.\n\nPrompts securely for the key value.\n\nExamples:\n  librefang config set-key groq\n  librefang config set-key openai\n  librefang config set-key anthropic"
    )]
    SetKey {
        /// Provider name (groq, anthropic, openai, gemini, deepseek, etc.).
        provider: String,
    },
    /// Remove an API key from ~/.librefang/.env.
    #[command(
        long_about = "Remove a stored API key from ~/.librefang/.env.\n\nExamples:\n  librefang config delete-key groq"
    )]
    DeleteKey {
        /// Provider name.
        provider: String,
    },
    /// Test provider connectivity with the stored API key.
    #[command(
        long_about = "Test connectivity to a provider using the stored API key.\n\nMakes a lightweight API call to verify the key is valid.\n\nExamples:\n  librefang config test-key groq\n  librefang config test-key openai"
    )]
    TestKey {
        /// Provider name.
        provider: String,
    },
}

#[derive(Subcommand)]
enum AgentCommands {
    /// Spawn a new agent from a template (interactive or by name).
    #[command(
        long_about = "Spawn a new agent from a built-in template.\n\nIf no template name is given, shows an interactive picker with all\navailable templates.\n\nExamples:\n  librefang agent new            # Interactive picker\n  librefang agent new coder      # Spawn a \"coder\" agent\n  librefang agent new assistant   # Spawn an \"assistant\" agent"
    )]
    New {
        /// Template name (e.g., "coder", "assistant"). Interactive picker if omitted.
        template: Option<String>,
    },
    /// Spawn a new agent from a manifest file.
    #[command(
        long_about = "Spawn a new agent from a TOML manifest file.\n\nExamples:\n  librefang agent spawn ./agent.toml\n  librefang agent spawn ./agent.toml --name my-agent\n  librefang agent spawn ./agent.toml --dry-run"
    )]
    Spawn(AgentSpawnArgs),
    /// List all running agents.
    #[command(
        long_about = "List all currently running agents with their IDs, names, and status.\n\nExamples:\n  librefang agent list          # Pretty-printed table\n  librefang agent list --json   # JSON output for scripting"
    )]
    List {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Interactive chat with an agent.
    #[command(
        long_about = "Start an interactive chat session with an agent by its UUID.\n\nType messages and press Enter. Use Ctrl+C or Ctrl+D to exit.\n\nExamples:\n  librefang agent chat 550e8400-e29b-41d4-a716-446655440000"
    )]
    Chat {
        /// Agent ID (UUID).
        agent_id: String,
    },
    /// Kill an agent.
    #[command(
        long_about = "Terminate a running agent by its UUID.\n\nExamples:\n  librefang agent kill 550e8400-e29b-41d4-a716-446655440000"
    )]
    Kill {
        /// Agent ID (UUID).
        agent_id: String,
    },
    /// Set an agent property (e.g., model).
    #[command(
        long_about = "Set a property on a running agent.\n\nCurrently supports changing the model. Provider can be set if provided as a prefix.\n\nExamples:\n  librefang agent set <ID> model gpt-4o\n  librefang agent set <ID> model claude-code/claude-sonnet"
    )]
    Set {
        /// Agent ID (UUID).
        agent_id: String,
        /// Field to set (model).
        field: String,
        /// New value.
        value: String,
    },
}

#[derive(Subcommand)]
enum WorkflowCommands {
    /// List all registered workflows.
    #[command(
        long_about = "List all registered workflows.\n\nExamples:\n  librefang workflow list"
    )]
    List,
    /// Create a workflow from a JSON file.
    #[command(
        long_about = "Create a new workflow from a JSON definition file.\n\nThe file should describe the workflow steps, agents, and routing logic.\n\nExamples:\n  librefang workflow create my-workflow.json"
    )]
    Create {
        /// Path to a JSON file describing the workflow.
        file: PathBuf,
    },
    /// Run a workflow by ID.
    #[command(
        long_about = "Run a registered workflow by its UUID with the given input.\n\nExamples:\n  librefang workflow run <ID> \"Summarize the quarterly report\""
    )]
    Run {
        /// Workflow ID (UUID).
        workflow_id: String,
        /// Input text for the workflow.
        input: String,
    },
}

#[derive(Subcommand)]
enum TriggerCommands {
    /// List all triggers (optionally filtered by agent).
    #[command(
        long_about = "List all event triggers, optionally filtered by agent ID.\n\nExamples:\n  librefang trigger list\n  librefang trigger list --agent-id <UUID>"
    )]
    List {
        /// Optional agent ID to filter by.
        #[arg(long)]
        agent_id: Option<String>,
    },
    /// Show details of a single trigger.
    #[command(
        long_about = "Show full details of a trigger by its UUID.\n\nExamples:\n  librefang trigger get <TRIGGER_ID>"
    )]
    Get {
        /// Trigger ID (UUID).
        trigger_id: String,
    },
    /// Create a trigger for an agent.
    #[command(
        long_about = "Create an event trigger that fires an agent when a matching event occurs.\n\nThe pattern is a JSON object describing what events to match. Use the\n{{event}} placeholder in the prompt template.\n\nExamples:\n  librefang trigger create <AGENT_ID> '\"lifecycle\"'\n  librefang trigger create <AGENT_ID> '{\"agent_spawned\":{\"name_pattern\":\"*\"}}' \\\n    --prompt \"New agent: {{event}}\" --max-fires 10\n  librefang trigger create <OWNER_ID> '\"task_posted\"' --target-agent <WORKER_ID>"
    )]
    Create {
        /// Agent ID (UUID) that owns the trigger.
        agent_id: String,
        /// Trigger pattern as JSON (e.g. '{"lifecycle":{}}' or '{"agent_spawned":{"name_pattern":"*"}}').
        pattern_json: String,
        /// Prompt template (use {{event}} placeholder).
        #[arg(long, default_value = "Event: {{event}}")]
        prompt: String,
        /// Maximum number of times to fire (0 = unlimited).
        #[arg(long, default_value = "0")]
        max_fires: u64,
        /// Route triggered messages to this agent instead of the owner (cross-session wake).
        #[arg(long)]
        target_agent: Option<String>,
        /// Cooldown in seconds before this trigger can fire again (0 = no cooldown).
        #[arg(long)]
        cooldown: Option<u64>,
        /// Session mode override: "persistent" or "new".
        #[arg(long)]
        session_mode: Option<String>,
    },
    /// Update fields of an existing trigger.
    #[command(
        long_about = "Update one or more fields of a trigger. Only supplied flags are changed.\n\nExamples:\n  librefang trigger update <ID> --prompt \"New prompt: {{event}}\"\n  librefang trigger update <ID> --max-fires 5 --cooldown 30\n  librefang trigger update <ID> --enabled false"
    )]
    Update {
        /// Trigger ID (UUID).
        trigger_id: String,
        /// New pattern JSON.
        #[arg(long)]
        pattern: Option<String>,
        /// New prompt template.
        #[arg(long)]
        prompt: Option<String>,
        /// Enable or disable the trigger.
        #[arg(long)]
        enabled: Option<bool>,
        /// New maximum fires limit (0 = unlimited).
        #[arg(long)]
        max_fires: Option<u64>,
        /// New cooldown in seconds between fires.
        #[arg(long)]
        cooldown: Option<u64>,
        /// Remove the cooldown limit entirely.
        #[arg(long)]
        clear_cooldown: bool,
        /// Override session mode for this trigger (persistent|new).
        #[arg(long)]
        session_mode: Option<String>,
        /// Remove the session mode override (revert to agent default).
        #[arg(long)]
        clear_session_mode: bool,
        /// Set the cross-session wake target agent ID (UUID).
        #[arg(long)]
        target_agent: Option<String>,
        /// Clear the target agent (revert to owner routing).
        #[arg(long)]
        clear_target_agent: bool,
    },
    /// Enable a trigger.
    #[command(
        long_about = "Enable a previously disabled trigger.\n\nExamples:\n  librefang trigger enable <TRIGGER_ID>"
    )]
    Enable {
        /// Trigger ID (UUID).
        trigger_id: String,
    },
    /// Disable a trigger without deleting it.
    #[command(
        long_about = "Disable a trigger without removing it.\n\nExamples:\n  librefang trigger disable <TRIGGER_ID>"
    )]
    Disable {
        /// Trigger ID (UUID).
        trigger_id: String,
    },
    /// Delete a trigger by ID.
    #[command(
        long_about = "Delete a trigger by its UUID.\n\nExamples:\n  librefang trigger delete <TRIGGER_ID>"
    )]
    Delete {
        /// Trigger ID (UUID).
        trigger_id: String,
    },
}

#[derive(Subcommand)]
enum ModelsCommands {
    /// List available models (optionally filter by provider).
    #[command(
        long_about = "List all available LLM models, optionally filtered by provider.\n\nExamples:\n  librefang models list\n  librefang models list --provider groq\n  librefang models list --json"
    )]
    List {
        /// Filter by provider name.
        #[arg(long)]
        provider: Option<String>,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Show model aliases (shorthand names).
    #[command(
        long_about = "Show model alias mappings (shorthand names to full model IDs).\n\nExamples:\n  librefang models aliases\n  librefang models aliases --json"
    )]
    Aliases {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// List known LLM providers and their auth status.
    #[command(
        long_about = "List known LLM providers and whether their API keys are configured.\n\nExamples:\n  librefang models providers\n  librefang models providers --json"
    )]
    Providers {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Set the default model for the daemon.
    #[command(
        long_about = "Set the default LLM model for the daemon.\n\nIf no model is specified, shows an interactive picker.\n\nExamples:\n  librefang models set              # Interactive picker\n  librefang models set gpt-4o       # Set by alias\n  librefang models set claude-sonnet"
    )]
    Set {
        /// Model ID or alias (e.g. "gpt-4o", "claude-sonnet"). Interactive picker if omitted.
        model: Option<String>,
    },
}

#[derive(Subcommand)]
enum GatewayCommands {
    /// Start the kernel daemon.
    #[command(
        long_about = "Start the kernel daemon.\n\nExamples:\n  librefang gateway start\n  librefang gateway start --tail\n  librefang gateway start --foreground"
    )]
    Start {
        /// Follow the daemon log after launching it in the background.
        #[arg(long, conflicts_with = "foreground")]
        tail: bool,
        /// Keep the daemon attached to the current terminal.
        #[arg(long)]
        foreground: bool,
    },
    /// Restart the kernel daemon.
    #[command(
        long_about = "Restart the kernel daemon (stop then start).\n\nExamples:\n  librefang gateway restart\n  librefang gateway restart --tail"
    )]
    Restart {
        /// Follow the daemon log after launching it in the background.
        #[arg(long, conflicts_with = "foreground")]
        tail: bool,
        /// Keep the relaunched daemon attached to the current terminal.
        #[arg(long)]
        foreground: bool,
    },
    /// Stop the running daemon.
    #[command(
        long_about = "Stop the running kernel daemon.\n\nExamples:\n  librefang gateway stop"
    )]
    Stop,
    /// Show daemon status.
    #[command(
        long_about = "Show the current daemon status.\n\nExamples:\n  librefang gateway status\n  librefang gateway status --json"
    )]
    Status {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum ApprovalsCommands {
    /// List pending approvals.
    #[command(
        long_about = "List pending execution approvals that require human review.\n\nExamples:\n  librefang approvals list\n  librefang approvals list --json"
    )]
    List {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Approve a pending request.
    #[command(
        long_about = "Approve a pending agent execution request.\n\nExamples:\n  librefang approvals approve <ID>"
    )]
    Approve {
        /// Approval ID.
        id: String,
    },
    /// Reject a pending request.
    #[command(
        long_about = "Reject a pending agent execution request.\n\nExamples:\n  librefang approvals reject <ID>"
    )]
    Reject {
        /// Approval ID.
        id: String,
    },
}

#[derive(Subcommand)]
enum CronCommands {
    /// List scheduled jobs.
    #[command(
        long_about = "List all scheduled cron jobs.\n\nExamples:\n  librefang cron list\n  librefang cron list --json"
    )]
    List {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Create a new scheduled job.
    #[command(
        long_about = "Create a new cron-style scheduled job.\n\nThe agent will receive the given prompt each time the cron expression fires.\n\nExamples:\n  librefang cron create my-agent \"0 */6 * * *\" \"Check for updates\"\n  librefang cron create my-agent \"0 9 * * 1\" \"Weekly summary\" --name weekly-report"
    )]
    Create {
        /// Agent name or ID to run.
        agent: String,
        /// Cron expression (e.g. "0 */6 * * *").
        spec: String,
        /// Prompt to send when the job fires.
        prompt: String,
        /// Optional job name (auto-generated if omitted).
        #[arg(long)]
        name: Option<String>,
    },
    /// Delete a scheduled job.
    #[command(
        long_about = "Delete a scheduled job by ID.\n\nExamples:\n  librefang cron delete <ID>"
    )]
    Delete {
        /// Job ID.
        id: String,
    },
    /// Enable a disabled job.
    #[command(
        long_about = "Re-enable a previously disabled cron job.\n\nExamples:\n  librefang cron enable <ID>"
    )]
    Enable {
        /// Job ID.
        id: String,
    },
    /// Disable a job without deleting it.
    #[command(
        long_about = "Disable a cron job without deleting it.\n\nThe job can be re-enabled later with `cron enable`.\n\nExamples:\n  librefang cron disable <ID>"
    )]
    Disable {
        /// Job ID.
        id: String,
    },
}

#[derive(Subcommand)]
enum SecurityCommands {
    /// Show security status summary.
    #[command(
        long_about = "Show a summary of the current security posture.\n\nExamples:\n  librefang security status\n  librefang security status --json"
    )]
    Status {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Show recent audit trail entries.
    #[command(
        long_about = "Show recent entries from the security audit trail.\n\nExamples:\n  librefang security audit\n  librefang security audit --limit 50\n  librefang security audit --json"
    )]
    Audit {
        /// Maximum number of entries to show.
        #[arg(long, default_value = "20")]
        limit: usize,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Verify audit trail integrity (Merkle chain).
    #[command(
        long_about = "Verify the integrity of the audit trail using its Merkle chain.\n\nReports whether the chain is intact or has been tampered with.\n\nExamples:\n  librefang security verify"
    )]
    Verify,
    /// Reset the audit trail: truncate `audit_entries` and remove the anchor file.
    ///
    /// Destructive. Use only when the chain is already broken (tampering, manual
    /// DB edits, partial restore) and you want to start a fresh chain. Requires
    /// `--confirm` and refuses to run while a daemon holds the database.
    #[command(
        long_about = "DESTRUCTIVE: wipe the audit trail and restart the chain from empty.\n\nOnly needed when `librefang security verify` reports a chain break that you can't recover — e.g. after a manual SQL edit, partial DB restore, or a crash that left the anchor file ahead of `audit_entries`.\n\nRefuses to run if the daemon is still holding the database. Requires `--confirm`.\n\nExamples:\n  librefang security audit-reset --confirm"
    )]
    #[cfg(feature = "sqlite-backend")]
    AuditReset {
        /// Required. Without this flag the command prints what it would do and exits non-zero.
        #[arg(long)]
        confirm: bool,
    },
}

/// Phase 7 of `surrealdb-storage-swap`: storage administration commands.
///
/// Surfaces the SurrealDB-backed db-explorer and the one-shot
/// SQLite → SurrealDB migrator. Gated on `surreal-backend`; the
/// `migrate` subcommand additionally requires `sqlite-backend`.
#[cfg(feature = "surreal-backend")]
#[derive(Subcommand)]
enum StorageCommands {
    /// Read-only inspection of the embedded SurrealDB store.
    #[command(
        long_about = "Read-only inspection of the embedded SurrealDB store.\n\nRefuses to run while the daemon is up (RocksDB holds an exclusive\nfile lock). Currently surfaces the `audit_entries` table; future\nrevisions will add hook traces, circuit breaker states, and the\nagent registry.\n\nExamples:\n  librefang storage explore\n  librefang storage explore --limit 100 --json"
    )]
    Explore {
        /// Maximum number of rows to print.
        #[arg(long, default_value = "20")]
        limit: u32,
        /// Emit JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// One-shot migration between storage backends.
    #[cfg(feature = "sqlite-backend")]
    #[command(
        long_about = "Stream the legacy SQLite tables (audit_entries, hook_traces,\ncircuit_breaker_states, totp_lockout, agents) into the configured\nSurrealDB instance.\n\nIdempotent: re-running converges via SurrealDB upserts. Refuses to\nrun while the daemon holds the SQLite writer lock. Writes a receipt\nJSON to `<data_dir>/migrations/` on success.\n\nExamples:\n  librefang storage migrate --from sqlite --to surreal --dry-run\n  librefang storage migrate --from sqlite --to surreal"
    )]
    Migrate {
        /// Source backend.
        #[arg(long, value_enum)]
        from: StorageMigrateSource,
        /// Target backend.
        #[arg(long, value_enum)]
        to: StorageMigrateTarget,
        /// Print row counts only; do not write to the target.
        #[arg(long)]
        dry_run: bool,
    },

    /// Provision a UAR namespace and application user on a shared remote SurrealDB.
    #[cfg(feature = "surreal-backend")]
    #[command(
        long_about = "Connect to a remote SurrealDB instance using root credentials and:\n  1. Create the UAR namespace (default: `uar`) and database (default: `main`).\n  2. Create a least-privilege EDITOR user scoped to that namespace.\n  3. Write the [uar.remote] block to config.toml so the next daemon restart\n     uses the shared instance.\n\nRoot credentials are only used once for provisioning and are never\npersisted.  The application password must be supplied via environment\nvariable (see --app-pass-env).\n\nExamples:\n  SURREAL_ROOT_PASS=s3cr3t SURREAL_APP_PASS=apppass \\\n    librefang storage link-uar \\\n      --remote-url ws://surreal:8000 \\\n      --root-user root \\\n      --root-pass-env SURREAL_ROOT_PASS \\\n      --app-user uar_app \\\n      --app-pass-env SURREAL_APP_PASS\n\n  # Also link memory (sets share_librefang_storage = true)\n  librefang storage link-uar ... --also-link-memory"
    )]
    LinkUar {
        /// Remote SurrealDB URL (ws://, wss://, http://, https://).
        #[arg(long)]
        remote_url: String,
        /// SurrealDB root (or admin-level) username.
        #[arg(long, default_value = "root")]
        root_user: String,
        /// Name of the env var that holds the root password.
        #[arg(long)]
        root_pass_env: String,
        /// Namespace to create for UAR (default: uar).
        #[arg(long, default_value = "uar")]
        namespace: String,
        /// Database to create inside the UAR namespace (default: main).
        #[arg(long, default_value = "main")]
        database: String,
        /// Application username for the UAR daemon (default: uar_app).
        #[arg(long, default_value = "uar_app")]
        app_user: String,
        /// Name of the env var that holds the application user's password.
        #[arg(long)]
        app_pass_env: String,
        /// Also set share_librefang_storage = true in config.toml so the
        /// UAR memory layer shares the librefang storage namespace.
        #[arg(long)]
        also_link_memory: bool,
    },

    /// Remove the UAR remote SurrealDB link written by `link-uar`.
    #[cfg(feature = "surreal-backend")]
    #[command(
        long_about = "Remove the [uar.remote] block and share_librefang_storage flag from\nconfig.toml.  Optionally drops the application user from the remote\nSurrealDB instance (requires root credentials).\n\nExamples:\n  librefang storage unlink-uar\n  SURREAL_ROOT_PASS=s3cr3t librefang storage unlink-uar \\\n    --purge-user --root-user root --root-pass-env SURREAL_ROOT_PASS"
    )]
    UnlinkUar {
        /// Also drop the application user from the remote SurrealDB.
        #[arg(long)]
        purge_user: bool,
        /// Root username (required only with --purge-user).
        #[arg(long, default_value = "root")]
        root_user: String,
        /// Name of the env var holding the root password (required only with --purge-user).
        #[arg(long)]
        root_pass_env: Option<String>,
    },
}

#[cfg(all(feature = "surreal-backend", feature = "sqlite-backend"))]
#[derive(Copy, Clone, Debug, PartialEq, Eq, clap::ValueEnum)]
enum StorageMigrateSource {
    Sqlite,
}

#[cfg(all(feature = "surreal-backend", feature = "sqlite-backend"))]
#[derive(Copy, Clone, Debug, PartialEq, Eq, clap::ValueEnum)]
enum StorageMigrateTarget {
    Surreal,
}

#[derive(Subcommand)]
enum MemoryCommands {
    /// List KV pairs for an agent.
    #[command(
        long_about = "List all key-value pairs stored in an agent's memory.\n\nExamples:\n  librefang memory list coder\n  librefang memory list coder --json"
    )]
    List {
        /// Agent name or ID.
        agent: String,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Get a specific KV value.
    #[command(
        long_about = "Get the value of a specific key from an agent's memory.\n\nExamples:\n  librefang memory get coder my-key\n  librefang memory get coder my-key --json"
    )]
    Get {
        /// Agent name or ID.
        agent: String,
        /// Key name.
        key: String,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Set a KV value.
    #[command(
        long_about = "Store a key-value pair in an agent's memory.\n\nExamples:\n  librefang memory set coder my-key \"hello world\""
    )]
    Set {
        /// Agent name or ID.
        agent: String,
        /// Key name.
        key: String,
        /// Value to store.
        value: String,
    },
    /// Delete a KV pair.
    #[command(
        long_about = "Delete a key-value pair from an agent's memory.\n\nExamples:\n  librefang memory delete coder my-key"
    )]
    Delete {
        /// Agent name or ID.
        agent: String,
        /// Key name.
        key: String,
    },
}

#[derive(Subcommand)]
enum DevicesCommands {
    /// List paired devices.
    #[command(
        long_about = "List all devices currently paired with this LibreFang instance.\n\nExamples:\n  librefang devices list\n  librefang devices list --json"
    )]
    List {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Start a new device pairing flow.
    #[command(
        long_about = "Start the device pairing flow to connect a new device.\n\nExamples:\n  librefang devices pair"
    )]
    Pair,
    /// Remove a paired device.
    #[command(
        long_about = "Remove a previously paired device by its ID.\n\nExamples:\n  librefang devices remove <DEVICE_ID>"
    )]
    Remove {
        /// Device ID.
        id: String,
    },
}

#[derive(Subcommand)]
enum WebhooksCommands {
    /// List configured webhooks.
    #[command(
        long_about = "List all configured webhook triggers.\n\nExamples:\n  librefang webhooks list\n  librefang webhooks list --json"
    )]
    List {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Create a new webhook trigger.
    #[command(
        long_about = "Create a new webhook trigger for an agent.\n\nThe agent will be invoked when the webhook URL receives a POST request.\n\nExamples:\n  librefang webhooks create coder https://example.com/hook"
    )]
    Create {
        /// Agent name or ID.
        agent: String,
        /// Webhook callback URL.
        url: String,
    },
    /// Delete a webhook.
    #[command(
        long_about = "Delete a webhook trigger by its ID.\n\nExamples:\n  librefang webhooks delete <ID>"
    )]
    Delete {
        /// Webhook ID.
        id: String,
    },
    /// Send a test payload to a webhook.
    #[command(
        long_about = "Send a test payload to a webhook to verify connectivity.\n\nExamples:\n  librefang webhooks test <ID>"
    )]
    Test {
        /// Webhook ID.
        id: String,
    },
}

#[derive(Subcommand)]
enum SystemCommands {
    /// Show detailed system info.
    #[command(
        long_about = "Show detailed system information including OS, architecture,\nhome directory, config path, and resource usage.\n\nExamples:\n  librefang system info\n  librefang system info --json"
    )]
    Info {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Show version information.
    #[command(
        long_about = "Show the LibreFang version, build info, and commit hash.\n\nExamples:\n  librefang system version\n  librefang system version --json"
    )]
    Version {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum ServiceCommands {
    /// Register auto-start service so LibreFang starts on boot/login.
    #[command(
        long_about = "Register a system service so LibreFang starts automatically.\n\nOn Linux:   creates a systemd user service (~/.config/systemd/user/librefang.service)\nOn macOS:   creates a LaunchAgent (~/Library/LaunchAgents/ai.librefang.daemon.plist)\nOn Windows: adds a registry entry (HKCU\\...\\Run)\n\nExamples:\n  librefang service install"
    )]
    Install,
    /// Remove the auto-start service.
    #[command(
        long_about = "Remove the previously installed auto-start service.\n\nExamples:\n  librefang service uninstall"
    )]
    Uninstall,
    /// Show whether the auto-start service is registered.
    #[command(
        long_about = "Check whether the auto-start service is currently registered.\n\nExamples:\n  librefang service status"
    )]
    Status,
}

fn init_tracing_stderr(log_level: &str) {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    // One-shot CLI commands (status, stop, doctor, …) load config.toml as a
    // side effect. librefang_kernel::config emits INFO on every load and WARN
    // for every unknown field; in a CLI context those lines leak into the
    // user's stdout flow and make basic commands look broken. Keep them out
    // of the default stderr budget — users who set RUST_LOG explicitly still
    // see everything, and daemon/foreground boots route through a different
    // initialiser where the full log is expected.
    let user_set_rust_log = std::env::var("RUST_LOG").is_ok();
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(log_level));
    let env_filter = if user_set_rust_log {
        env_filter
    } else {
        // For one-shot CLI commands, downgrade library-level chatter so the
        // user sees only their command's own output. WARN and above still
        // surface everywhere — the filter is per-target verbosity, not a
        // global mute. Setting RUST_LOG restores full detail.
        env_filter
            .add_directive("librefang_kernel=warn".parse().expect("static directive"))
            .add_directive("librefang_runtime=warn".parse().expect("static directive"))
            .add_directive(
                "librefang_extensions=warn"
                    .parse()
                    .expect("static directive"),
            )
            .add_directive(
                "librefang_kernel::config=error"
                    .parse()
                    .expect("static directive"),
            )
            .add_directive(
                "librefang_runtime::registry_sync=error"
                    .parse()
                    .expect("static directive"),
            )
    };

    // Compact stderr format: in a one-shot CLI context the user cares about
    // the WARN/ERROR text, not the timestamp or the fully-qualified target.
    // One-shot CLI runs are transient — stderr is the only sink; the daemon
    // has its own file appender under `logs/daemon.log`.
    let fmt_layer = tracing_subscriber::fmt::layer()
        .without_time()
        .with_target(false)
        .compact();

    // Register a no-op reload slot so `init_otel_tracing` can swap a real
    // OTel layer in later without needing to claim the global dispatcher.
    // The slot is stacked **first** (directly on Registry) so its boxed
    // `Layer<Registry>` trait object matches the innermost subscriber type.
    #[cfg(feature = "telemetry")]
    let registry =
        tracing_subscriber::registry().with(librefang_api::telemetry::install_otel_reload_layer());
    #[cfg(not(feature = "telemetry"))]
    let registry = tracing_subscriber::registry();

    registry.with(env_filter).with(fmt_layer).init();
}

/// Get the LibreFang home directory, respecting LIBREFANG_HOME env var.
fn cli_librefang_home() -> std::path::PathBuf {
    if let Ok(home) = std::env::var("LIBREFANG_HOME") {
        return std::path::PathBuf::from(home);
    }
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".librefang")
}

#[derive(Debug, Clone)]
struct DaemonConfigContext {
    home_dir: PathBuf,
    api_key: Option<String>,
    log_dir: Option<PathBuf>,
}

fn daemon_config_context(config: Option<&std::path::Path>) -> DaemonConfigContext {
    let config = load_config(config);
    let api_key = {
        let trimmed = config.api_key.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    };
    DaemonConfigContext {
        home_dir: config.home_dir,
        api_key,
        log_dir: config.log_dir,
    }
}

/// Redirect tracing to a log file so it doesn't corrupt the ratatui TUI.
fn init_tracing_file(log_level: &str, custom_log_dir: Option<&std::path::Path>) {
    // `custom_log_dir` is already a log directory (typically `daemon.log_dir`
    // from config); use it as-is. Otherwise default to `<home>/logs/`.
    let log_dir = custom_log_dir
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| cli_librefang_home().join("logs"));
    let _ = std::fs::create_dir_all(&log_dir);
    let log_path = log_dir.join("tui.log");

    match std::fs::File::create(&log_path) {
        Ok(file) => {
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(log_level)),
                )
                .with_writer(std::sync::Mutex::new(file))
                .with_ansi(false)
                .init();
        }
        Err(_) => {
            // Fallback: suppress all output rather than corrupt the TUI
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::ERROR)
                .with_writer(std::io::sink)
                .init();
        }
    }
}

fn load_language_from_config() -> Option<String> {
    let config_path = dirs::home_dir()?.join(".librefang").join("config.toml");
    let content = std::fs::read_to_string(&config_path).ok()?;
    let config: toml::Value = toml::from_str(&content).ok()?;
    config.get("language")?.as_str().map(|s| s.to_string())
}

/// Load just the `log_level` field from config.toml without fully deserializing.
/// Returns the configured level (e.g. "debug", "warn") or falls back to "info".
fn load_log_level_from_config() -> String {
    let level = (|| -> Option<String> {
        let config_path = dirs::home_dir()?.join(".librefang").join("config.toml");
        let content = std::fs::read_to_string(&config_path).ok()?;
        let config: toml::Value = toml::from_str(&content).ok()?;
        config.get("log_level")?.as_str().map(|s| s.to_string())
    })();
    level.unwrap_or_else(|| "info".to_string())
}

/// Load just the `update_channel` field from config.toml without fully deserializing.
fn load_update_channel_from_config() -> Option<librefang_types::config::UpdateChannel> {
    let config_path = dirs::home_dir()?.join(".librefang").join("config.toml");
    let content = std::fs::read_to_string(&config_path).ok()?;
    let config: toml::Value = toml::from_str(&content).ok()?;
    config
        .get("update_channel")?
        .as_str()?
        .parse::<librefang_types::config::UpdateChannel>()
        .ok()
}

/// Load just the `log_dir` field from config.toml without fully deserializing.
/// Returns the configured custom log directory, or `None` to use the default.
fn load_log_dir_from_config() -> Option<PathBuf> {
    let config_path = dirs::home_dir()?.join(".librefang").join("config.toml");
    let content = std::fs::read_to_string(&config_path).ok()?;
    let config: toml::Value = toml::from_str(&content).ok()?;
    config.get("log_dir")?.as_str().map(PathBuf::from)
}

fn main() {
    // Initialize rustls crypto provider FIRST, before any async/TLS operations
    // This is required because rustls 0.23 needs explicit crypto provider initialization
    {
        use rustls::crypto::aws_lc_rs;
        let _ = aws_lc_rs::default_provider().install_default();
    }

    // Load ~/.librefang/.env into process environment (system env takes priority).
    dotenv::load_dotenv();

    let language = load_language_from_config().unwrap_or_else(|| "en".to_string());
    i18n::init(&language);

    let cli = Cli::parse();

    // Determine if this invocation launches a ratatui TUI.
    // TUI modes must NOT install the Ctrl+C handler (it calls process::exit
    // which bypasses ratatui::restore and leaves the terminal in raw mode).
    // TUI modes also need file-based tracing (stderr output corrupts the TUI).
    let is_launcher = cli.command.is_none() && std::io::IsTerminal::is_terminal(&std::io::stdout());
    let is_tui_mode = is_launcher
        || matches!(cli.command, Some(Commands::Tui))
        || matches!(cli.command, Some(Commands::Chat { .. }))
        || matches!(
            cli.command,
            Some(Commands::Agent(AgentCommands::Chat { .. }))
        );

    let log_level = load_log_level_from_config();
    let custom_log_dir = load_log_dir_from_config();

    if is_tui_mode {
        init_tracing_file(&log_level, custom_log_dir.as_deref());
    } else {
        // CLI subcommands: install Ctrl+C handler for clean interrupt of
        // blocking read_line calls, and trace to stderr.
        install_ctrlc_handler();
        init_tracing_stderr(&log_level);
    }

    match cli.command {
        None => {
            if !std::io::IsTerminal::is_terminal(&std::io::stdout()) {
                // Piped: fall back to text help
                use clap::CommandFactory;
                Cli::command().print_help().unwrap();
                println!();
                return;
            }
            match launcher::run(cli.config.clone()) {
                launcher::LauncherChoice::GetStarted => cmd_init(false),
                launcher::LauncherChoice::Chat => cmd_quick_chat(cli.config, None),
                launcher::LauncherChoice::Dashboard => cmd_dashboard(),
                launcher::LauncherChoice::DesktopApp => launcher::launch_desktop_app(),
                launcher::LauncherChoice::TerminalUI => tui::run(cli.config),
                launcher::LauncherChoice::ShowHelp => {
                    use clap::CommandFactory;
                    Cli::command().print_help().unwrap();
                    println!();
                }
                launcher::LauncherChoice::Quit => {}
            }
        }
        Some(Commands::Tui) => tui::run(cli.config),
        Some(Commands::Init { quick, upgrade }) => {
            if upgrade {
                cmd_init_upgrade();
            } else {
                cmd_init(quick);
            }
        }
        Some(Commands::Start {
            tail,
            foreground,
            spawned,
            bind,
        }) => cmd_start(cli.config, tail, spawned, foreground, bind),
        Some(Commands::Restart { tail, foreground }) => cmd_restart(cli.config, tail, foreground),
        Some(Commands::Spawn(args)) => cmd_spawn_alias(
            cli.config,
            args.target,
            args.template,
            args.name,
            args.dry_run,
        ),
        Some(Commands::Agents { json }) => cmd_agent_list(cli.config, json),
        Some(Commands::Kill { agent_id }) => cmd_agent_kill(cli.config, &agent_id),
        Some(Commands::Update {
            check,
            version,
            channel,
        }) => cmd_update(check, version, channel),
        Some(Commands::Stop) => cmd_stop(cli.config),
        Some(Commands::Agent(sub)) => match sub {
            AgentCommands::New { template } => cmd_agent_new(cli.config, template),
            AgentCommands::Spawn(args) => {
                cmd_agent_spawn(cli.config, args.manifest, args.name, args.dry_run)
            }
            AgentCommands::List { json } => cmd_agent_list(cli.config, json),
            AgentCommands::Chat { agent_id } => cmd_agent_chat(cli.config, &agent_id),
            AgentCommands::Kill { agent_id } => cmd_agent_kill(cli.config, &agent_id),
            AgentCommands::Set {
                agent_id,
                field,
                value,
            } => cmd_agent_set(&agent_id, &field, &value),
        },
        Some(Commands::Workflow(sub)) => match sub {
            WorkflowCommands::List => cmd_workflow_list(),
            WorkflowCommands::Create { file } => cmd_workflow_create(file),
            WorkflowCommands::Run { workflow_id, input } => cmd_workflow_run(&workflow_id, &input),
        },
        Some(Commands::Trigger(sub)) => match sub {
            TriggerCommands::List { agent_id } => cmd_trigger_list(agent_id.as_deref()),
            TriggerCommands::Get { trigger_id } => cmd_trigger_get(&trigger_id),
            TriggerCommands::Create {
                agent_id,
                pattern_json,
                prompt,
                max_fires,
                target_agent,
                cooldown,
                session_mode,
            } => cmd_trigger_create(
                &agent_id,
                &pattern_json,
                &prompt,
                max_fires,
                target_agent.as_deref(),
                cooldown,
                session_mode.as_deref(),
            ),
            TriggerCommands::Update {
                trigger_id,
                pattern,
                prompt,
                enabled,
                max_fires,
                cooldown,
                clear_cooldown,
                session_mode,
                clear_session_mode,
                target_agent,
                clear_target_agent,
            } => cmd_trigger_update(
                &trigger_id,
                pattern.as_deref(),
                prompt.as_deref(),
                enabled,
                max_fires,
                cooldown,
                clear_cooldown,
                session_mode.as_deref(),
                clear_session_mode,
                target_agent.as_deref(),
                clear_target_agent,
            ),
            TriggerCommands::Enable { trigger_id } => cmd_trigger_set_enabled(&trigger_id, true),
            TriggerCommands::Disable { trigger_id } => cmd_trigger_set_enabled(&trigger_id, false),
            TriggerCommands::Delete { trigger_id } => cmd_trigger_delete(&trigger_id),
        },
        Some(Commands::Migrate(args)) => cmd_migrate(args),
        Some(Commands::Skill(sub)) => match sub {
            SkillCommands::Install { source, hand } => cmd_skill_install(&source, hand.as_deref()),
            SkillCommands::List { hand } => cmd_skill_list(hand.as_deref()),
            SkillCommands::Remove { name, hand } => cmd_skill_remove(&name, hand.as_deref()),
            SkillCommands::Search { query } => cmd_skill_search(&query),
            SkillCommands::Test { path, tool, input } => cmd_skill_test(path, tool, input),
            SkillCommands::Publish {
                path,
                repo,
                tag,
                output,
                dry_run,
            } => cmd_skill_publish(path, repo, tag, output, dry_run),
            SkillCommands::Create => cmd_skill_create(),
            SkillCommands::Evolve(sub) => cmd_skill_evolve(sub),
        },
        Some(Commands::Channel(sub)) => match sub {
            ChannelCommands::List => cmd_channel_list(),
            ChannelCommands::Setup { channel } => cmd_channel_setup(channel.as_deref()),
            ChannelCommands::Test {
                name,
                channel_id,
                chat_id,
            } => cmd_channel_test(&name, channel_id.as_deref(), chat_id.as_deref()),
            ChannelCommands::Enable { channel } => cmd_channel_toggle(&channel, true),
            ChannelCommands::Disable { channel } => cmd_channel_toggle(&channel, false),
        },
        Some(Commands::Hand(sub)) => match sub {
            HandCommands::List => cmd_hand_list(),
            HandCommands::Active => cmd_hand_active(),
            HandCommands::Status { id } => cmd_hand_status(id.as_deref()),
            HandCommands::Install { path } => cmd_hand_install(&path),
            HandCommands::Activate { id } => cmd_hand_activate(&id),
            HandCommands::Deactivate { id } => cmd_hand_deactivate(&id),
            HandCommands::Info { id } => cmd_hand_info(&id),
            HandCommands::CheckDeps { id } => cmd_hand_check_deps(&id),
            HandCommands::InstallDeps { id } => cmd_hand_install_deps(&id),
            HandCommands::Pause { id } => cmd_hand_pause(&id),
            HandCommands::Resume { id } => cmd_hand_resume(&id),
            HandCommands::Settings { id } => cmd_hand_settings(&id),
            HandCommands::Set { id, key, value } => cmd_hand_set(&id, &key, &value),
            HandCommands::Reload => cmd_hand_reload(),
            HandCommands::Chat { id } => cmd_hand_chat(&id),
        },
        Some(Commands::Config(sub)) => match sub {
            ConfigCommands::Show => cmd_config_show(),
            ConfigCommands::Edit => cmd_config_edit(),
            ConfigCommands::Get { key } => cmd_config_get(&key),
            ConfigCommands::Set { key, value } => cmd_config_set(&key, &value),
            ConfigCommands::Unset { key } => cmd_config_unset(&key),
            ConfigCommands::SetKey { provider } => cmd_config_set_key(&provider),
            ConfigCommands::DeleteKey { provider } => cmd_config_delete_key(&provider),
            ConfigCommands::TestKey { provider } => cmd_config_test_key(&provider),
        },
        Some(Commands::Chat { agent }) => cmd_quick_chat(cli.config, agent),
        Some(Commands::Status {
            json,
            verbose,
            quiet,
            watch,
        }) => cmd_status(cli.config, json, verbose, quiet, watch),
        Some(Commands::Doctor { json, repair }) => cmd_doctor(json, repair),
        Some(Commands::Dashboard) => cmd_dashboard(),
        Some(Commands::Completion { shell }) => cmd_completion(shell),
        Some(Commands::Mcp { command }) => match command {
            None => mcp::run_mcp_server(cli.config),
            Some(McpCommands::List) => cmd_mcp_list(),
            Some(McpCommands::Catalog { query }) => cmd_mcp_catalog(query.as_deref()),
            Some(McpCommands::Add { name, key }) => cmd_mcp_add(&name, key.as_deref()),
            Some(McpCommands::Remove { name }) => cmd_mcp_remove(&name),
        },
        Some(Commands::Auth(sub)) => match sub {
            AuthCommands::Chatgpt { device_auth } => cmd_auth_chatgpt(device_auth),
        },
        Some(Commands::Vault(sub)) => match sub {
            VaultCommands::Init => cmd_vault_init(),
            VaultCommands::Set { key } => cmd_vault_set(&key),
            VaultCommands::List => cmd_vault_list(),
            VaultCommands::Remove { key } => cmd_vault_remove(&key),
        },
        Some(Commands::New { kind }) => cmd_scaffold(kind),
        // ── New commands ────────────────────────────────────────────────
        Some(Commands::Models(sub)) => match sub {
            ModelsCommands::List { provider, json } => cmd_models_list(provider.as_deref(), json),
            ModelsCommands::Aliases { json } => cmd_models_aliases(json),
            ModelsCommands::Providers { json } => cmd_models_providers(json),
            ModelsCommands::Set { model } => cmd_models_set(model),
        },
        Some(Commands::Gateway(sub)) => match sub {
            GatewayCommands::Start { tail, foreground } => {
                cmd_start(cli.config, tail, false, foreground, None)
            }
            GatewayCommands::Restart { tail, foreground } => {
                cmd_restart(cli.config, tail, foreground)
            }
            GatewayCommands::Stop => cmd_stop(cli.config),
            GatewayCommands::Status { json } => cmd_status(cli.config, json, false, false, None),
        },
        Some(Commands::Approvals(sub)) => match sub {
            ApprovalsCommands::List { json } => cmd_approvals_list(json),
            ApprovalsCommands::Approve { id } => cmd_approvals_respond(&id, true),
            ApprovalsCommands::Reject { id } => cmd_approvals_respond(&id, false),
        },
        Some(Commands::Cron(sub)) => match sub {
            CronCommands::List { json } => cmd_cron_list(json),
            CronCommands::Create {
                agent,
                spec,
                prompt,
                name,
            } => cmd_cron_create(&agent, &spec, &prompt, name.as_deref()),
            CronCommands::Delete { id } => cmd_cron_delete(&id),
            CronCommands::Enable { id } => cmd_cron_toggle(&id, true),
            CronCommands::Disable { id } => cmd_cron_toggle(&id, false),
        },
        Some(Commands::Sessions { agent, json }) => cmd_sessions(agent.as_deref(), json),
        Some(Commands::Logs { lines, follow }) => cmd_logs(cli.config, lines, follow),
        Some(Commands::Health { json }) => cmd_health(json),
        Some(Commands::Security(sub)) => match sub {
            SecurityCommands::Status { json } => cmd_security_status(json),
            SecurityCommands::Audit { limit, json } => cmd_security_audit(limit, json),
            SecurityCommands::Verify => cmd_security_verify(),
            #[cfg(feature = "sqlite-backend")]
            SecurityCommands::AuditReset { confirm } => cmd_audit_reset(cli.config, confirm),
        },
        #[cfg(feature = "surreal-backend")]
        Some(Commands::Storage(sub)) => match sub {
            StorageCommands::Explore { limit, json } => {
                cmd_audit_explore_surreal(cli.config, limit, json)
            }
            #[cfg(feature = "sqlite-backend")]
            StorageCommands::Migrate { from, to, dry_run } => {
                if from != StorageMigrateSource::Sqlite || to != StorageMigrateTarget::Surreal {
                    ui::error("only `--from sqlite --to surreal` is currently supported");
                    std::process::exit(2);
                }
                cmd_storage_migrate(cli.config, dry_run);
            }
            #[cfg(feature = "surreal-backend")]
            StorageCommands::LinkUar {
                remote_url,
                root_user,
                root_pass_env,
                namespace,
                database,
                app_user,
                app_pass_env,
                also_link_memory,
            } => cmd_storage_link_uar(
                remote_url,
                root_user,
                root_pass_env,
                namespace,
                database,
                app_user,
                app_pass_env,
                also_link_memory,
            ),
            #[cfg(feature = "surreal-backend")]
            StorageCommands::UnlinkUar {
                purge_user,
                root_user,
                root_pass_env,
            } => cmd_storage_unlink_uar(purge_user, root_user, root_pass_env),
        },
        Some(Commands::Memory(sub)) => match sub {
            MemoryCommands::List { agent, json } => cmd_memory_list(&agent, json),
            MemoryCommands::Get { agent, key, json } => cmd_memory_get(&agent, &key, json),
            MemoryCommands::Set { agent, key, value } => cmd_memory_set(&agent, &key, &value),
            MemoryCommands::Delete { agent, key } => cmd_memory_delete(&agent, &key),
        },
        Some(Commands::Devices(sub)) => match sub {
            DevicesCommands::List { json } => cmd_devices_list(json),
            DevicesCommands::Pair => cmd_devices_pair(),
            DevicesCommands::Remove { id } => cmd_devices_remove(&id),
        },
        Some(Commands::Qr) => cmd_devices_pair(),
        Some(Commands::Webhooks(sub)) => match sub {
            WebhooksCommands::List { json } => cmd_webhooks_list(json),
            WebhooksCommands::Create { agent, url } => cmd_webhooks_create(&agent, &url),
            WebhooksCommands::Delete { id } => cmd_webhooks_delete(&id),
            WebhooksCommands::Test { id } => cmd_webhooks_test(&id),
        },
        Some(Commands::Onboard { quick, upgrade }) | Some(Commands::Setup { quick, upgrade }) => {
            if upgrade {
                cmd_init_upgrade();
            } else {
                cmd_init(quick);
            }
        }
        Some(Commands::Configure) => cmd_init(false),
        Some(Commands::Message { agent, text, json }) => cmd_message(&agent, &text, json),
        Some(Commands::System(sub)) => match sub {
            SystemCommands::Info { json } => cmd_system_info(json),
            SystemCommands::Version { json } => cmd_system_version(json),
        },
        Some(Commands::Service(sub)) => match sub {
            ServiceCommands::Install => cmd_service_install(),
            ServiceCommands::Uninstall => cmd_service_uninstall(),
            ServiceCommands::Status => cmd_service_status(),
        },
        Some(Commands::Reset { confirm }) => cmd_reset(confirm),
        Some(Commands::Uninstall {
            confirm,
            keep_config,
        }) => cmd_uninstall(confirm, keep_config),
        Some(Commands::HashPassword { password }) => cmd_hash_password(password),
    }
}

// ---------------------------------------------------------------------------
// Daemon detection helpers
// ---------------------------------------------------------------------------

/// Try to find a running daemon. Returns its base URL if found.
/// SECURITY: Restrict file permissions to owner-only (0600) on Unix.
#[cfg(unix)]
pub(crate) fn restrict_file_permissions(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
pub(crate) fn restrict_file_permissions(_path: &std::path::Path) {}

/// SECURITY: Restrict directory permissions to owner-only (0700) on Unix.
#[cfg(unix)]
pub(crate) fn restrict_dir_permissions(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
pub(crate) fn restrict_dir_permissions(_path: &std::path::Path) {}

fn find_daemon_in_home(home_dir: &std::path::Path) -> Option<String> {
    let info = read_daemon_info(home_dir)?;

    // Normalize listen address: replace 0.0.0.0 with 127.0.0.1 to avoid
    // DNS/connectivity issues on macOS where 0.0.0.0 can hang.
    let addr = info.listen_addr.replace("0.0.0.0", "127.0.0.1");
    let url = format!("http://{addr}/api/health");

    let client = crate::http_client::client_builder()
        .connect_timeout(std::time::Duration::from_secs(1))
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .ok()?;
    let resp = client.get(&url).send().ok()?;
    if resp.status().is_success() {
        Some(format!("http://{addr}"))
    } else {
        None
    }
}

pub(crate) fn find_daemon() -> Option<String> {
    find_daemon_in_home(&cli_librefang_home())
}

/// Build an HTTP client for daemon calls.
///
/// When api_key is configured in config.toml, the client automatically
/// includes a `Authorization: Bearer <key>` header on every request.
/// When api_key is empty or missing, no auth header is sent.
pub(crate) fn daemon_client() -> reqwest::blocking::Client {
    daemon_client_with_api_key(read_api_key().as_deref())
}

fn daemon_client_with_api_key(api_key: Option<&str>) -> reqwest::blocking::Client {
    let mut builder =
        crate::http_client::client_builder().timeout(std::time::Duration::from_secs(120));

    if let Some(key) = api_key {
        let mut headers = reqwest::header::HeaderMap::new();
        if let Ok(val) = reqwest::header::HeaderValue::from_str(&format!("Bearer {key}")) {
            headers.insert(reqwest::header::AUTHORIZATION, val);
        }
        builder = builder.default_headers(headers);
    }

    builder.build().expect("Failed to build HTTP client")
}

/// Helper: send a request to the daemon and parse the JSON body.
/// Exits with error on connection failure.
pub(crate) fn daemon_json(
    resp: Result<reqwest::blocking::Response, reqwest::Error>,
) -> serde_json::Value {
    match resp {
        Ok(r) => {
            let status = r.status();
            let body = r.json::<serde_json::Value>().unwrap_or_default();
            if status.is_server_error() {
                ui::error_with_fix(
                    &i18n::t_args("error-daemon-returned", &[("status", &status.to_string())]),
                    &i18n::t("error-daemon-returned-fix"),
                );
            }
            body
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("timed out") || msg.contains("Timeout") {
                ui::error_with_fix(
                    &i18n::t("error-request-timeout"),
                    &i18n::t("error-request-timeout-fix"),
                );
            } else if msg.contains("Connection refused") || msg.contains("connect") {
                ui::error_with_fix(
                    &i18n::t("error-connect-refused"),
                    &i18n::t("error-connect-refused-fix"),
                );
            } else {
                ui::error_with_fix(
                    &i18n::t_args("error-daemon-comm", &[("error", &msg)]),
                    &i18n::t("error-daemon-comm-fix"),
                );
            }
            std::process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn cmd_init(quick: bool) {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => {
            ui::error(&i18n::t("error-home-dir"));
            std::process::exit(1);
        }
    };

    let librefang_dir = cli_librefang_home();

    // When an existing config is detected in interactive mode, redirect to the
    // upgrade path so user settings (channels, keys, etc.) are preserved.
    // The interactive wizard unconditionally overwrites config.toml, which
    // would silently delete channels and custom configuration (#1862).
    if !quick && librefang_dir.join("config.toml").exists() {
        ui::hint("Existing installation detected — running upgrade to preserve your settings.");
        ui::hint("To start fresh, remove ~/.librefang/config.toml and run `librefang init` again.");
        cmd_init_upgrade();
        return;
    }

    // --- Ensure directories exist ---
    if !librefang_dir.exists() {
        std::fs::create_dir_all(&librefang_dir).unwrap_or_else(|e| {
            ui::error_with_fix(
                &i18n::t_args(
                    "error-create-dir",
                    &[("path", &librefang_dir.display().to_string())],
                ),
                &i18n::t_args(
                    "error-create-dir-fix",
                    &[("path", &home.display().to_string())],
                ),
            );
            eprintln!("  {e}");
            std::process::exit(1);
        });
        restrict_dir_permissions(&librefang_dir);
    }

    let data_dir = librefang_dir.join("data");
    if !data_dir.exists() {
        std::fs::create_dir_all(&data_dir).unwrap_or_else(|e| {
            eprintln!("Error creating data dir: {e}");
            std::process::exit(1);
        });
    }

    // Sync registry content (downloads to registry/, pre-installs providers/integrations/assistant)
    librefang_runtime::registry_sync::sync_registry(
        &librefang_dir,
        librefang_runtime::registry_sync::DEFAULT_CACHE_TTL_SECS,
        "",
    );

    // Initialize vault if not already initialized
    init_vault_if_missing(&librefang_dir);

    // Initialize git repo for config version control
    init_git_if_missing(&librefang_dir);

    if quick {
        cmd_init_quick(&librefang_dir);
    } else if !std::io::IsTerminal::is_terminal(&std::io::stdin())
        || !std::io::IsTerminal::is_terminal(&std::io::stdout())
    {
        ui::hint(&i18n::t("hint-non-interactive"));
        ui::hint(&i18n::t("hint-non-interactive-wizard"));
        cmd_init_quick(&librefang_dir);
    } else {
        cmd_init_interactive(&librefang_dir);
    }

    // Fallback: ensure config.toml exists even if wizard was cancelled/failed
    let config_path = librefang_dir.join("config.toml");
    if !config_path.exists() {
        let (provider, api_key_env, model) = detect_best_provider();
        write_config_if_missing(&librefang_dir, &provider, &model, &api_key_env);
    }
}

/// Upgrade an existing LibreFang installation: backup config, sync registry, merge new defaults.
fn cmd_init_upgrade() {
    let librefang_dir = cli_librefang_home();
    let config_path = librefang_dir.join("config.toml");

    // 1. Must have an existing installation
    if !config_path.exists() {
        ui::error("Nothing to upgrade — no config.toml found. Run `librefang init` first.");
        std::process::exit(1);
    }

    ui::banner();
    ui::blank();
    ui::section("Upgrading LibreFang installation");

    // 2. Backup existing config under backups/ (keep last 3)
    let backups_dir = librefang_dir.join("backups");
    if let Err(e) = std::fs::create_dir_all(&backups_dir) {
        ui::error(&format!("Failed to create backups dir: {e}"));
        std::process::exit(1);
    }
    let backup_name = format!("config-{}.toml", format_local_timestamp());
    let backup_path = backups_dir.join(&backup_name);
    if let Err(e) = std::fs::copy(&config_path, &backup_path) {
        ui::error(&format!("Failed to backup config: {e}"));
        std::process::exit(1);
    }
    restrict_file_permissions(&backup_path);
    prune_old_config_backups(&backups_dir, 3);
    ui::success(&format!("Backed up config to backups/{backup_name}"));

    // 3. Sync registry (TTL=0 forces refresh regardless of last sync time)
    ui::hint("Syncing registry...");
    if librefang_runtime::registry_sync::sync_registry(&librefang_dir, 0, "") {
        ui::success("Registry synced");
    } else {
        ui::hint("Registry sync failed (network issue?) — continuing with cached content");
    }

    // 4. Ensure data dir, vault, and git exist
    let data_dir = librefang_dir.join("data");
    if !data_dir.exists() {
        let _ = std::fs::create_dir_all(&data_dir);
    }
    init_vault_if_missing(&librefang_dir);
    init_git_if_missing(&librefang_dir);

    // Ensure .gitignore excludes the backups/ directory (may be missing in older installations)
    let gitignore = librefang_dir.join(".gitignore");
    if gitignore.exists() {
        if let Ok(content) = std::fs::read_to_string(&gitignore) {
            if !content.lines().any(|l| l.trim() == "backups/") {
                let _ = std::fs::write(&gitignore, format!("{content}backups/\n"));
            }
        }
    }

    // 5. Merge new default config fields
    let existing_raw = match std::fs::read_to_string(&config_path) {
        Ok(s) => s,
        Err(e) => {
            ui::error(&format!("Failed to read config.toml: {e}"));
            std::process::exit(1);
        }
    };

    let existing: toml::Value = match toml::from_str(&existing_raw) {
        Ok(v) => v,
        Err(e) => {
            ui::error(&format!("Failed to parse config.toml: {e}"));
            ui::hint(&format!(
                "Your original config was saved to backups/{backup_name}"
            ));
            std::process::exit(1);
        }
    };

    let (provider, api_key_env, model) = detect_best_provider();
    let default_config_str = render_init_default_config(&provider, &model, &api_key_env);
    let defaults: toml::Value = match toml::from_str(&default_config_str) {
        Ok(v) => v,
        Err(e) => {
            ui::error(&format!("Failed to parse default config template: {e}"));
            std::process::exit(1);
        }
    };

    // Find top-level keys/sections missing from user config and append them
    // as TOML fragments. This preserves the original file's comments and formatting.
    let added = find_missing_toplevel_keys(&existing, &defaults);

    if added.is_empty() {
        ui::success("Config is already up to date — no new fields added");
    } else {
        // Partition into scalars (must stay in TOML root scope) and tables.
        // Scalars appended after a [table] header would be absorbed into that
        // table's scope, potentially colliding with same-named sub-keys (#2021).
        let (scalar_keys, table_keys): (Vec<_>, Vec<_>) = added
            .iter()
            .partition(|k| defaults.get(*k).is_none_or(|v| !v.is_table()));

        let mut content = existing_raw.clone();

        // Insert scalar keys before the first [table] header so they remain
        // top-level in the TOML document.
        if !scalar_keys.is_empty() {
            let mut scalar_snippet = String::new();
            for key in &scalar_keys {
                if let Some(val) = defaults.get(*key) {
                    let mut fragment = toml::map::Map::new();
                    fragment.insert((*key).clone(), val.clone());
                    if let Ok(s) = toml::to_string_pretty(&toml::Value::Table(fragment)) {
                        scalar_snippet.push_str(&s);
                    }
                }
            }
            // Find the first line that starts with '[' (a table header).
            // We search for "\n[" then insert just before the '['.
            if let Some(pos) = content.find("\n[").map(|p| p + 1) {
                content.insert_str(pos, &format!("{scalar_snippet}\n"));
            } else {
                // No table headers in file — appending is safe.
                content.push('\n');
                content.push_str(&scalar_snippet);
            }
        }

        // Append table sections at the end of the file.
        if !table_keys.is_empty() {
            content.push_str("\n# ── Added by upgrade ────────────────────────────────────\n");
            for key in &table_keys {
                if let Some(val) = defaults.get(*key) {
                    let mut fragment = toml::map::Map::new();
                    fragment.insert((*key).clone(), val.clone());
                    if let Ok(snippet) = toml::to_string_pretty(&toml::Value::Table(fragment)) {
                        content.push('\n');
                        content.push_str(&snippet);
                    }
                }
            }
        }

        if let Err(e) = std::fs::write(&config_path, &content) {
            ui::error(&format!("Failed to write config: {e}"));
            ui::hint(&format!(
                "Your original config was saved to backups/{backup_name}"
            ));
            std::process::exit(1);
        }
        restrict_file_permissions(&config_path);
        ui::success(&format!("Added {} new config section(s):", added.len()));
        for key in &added {
            ui::kv("  +", key);
        }
    }

    // 6. Check for legacy ~/.openclaw installation
    if let Some(home) = dirs::home_dir() {
        let openclaw_dir = home.join(".openclaw");
        if openclaw_dir.exists() {
            ui::blank();
            ui::hint("Legacy ~/.openclaw installation detected.");
            ui::hint("Run `librefang migrate --from openclaw` to migrate your data.");
        }
    }

    // 7. Warn users whose require_approval list predates the file_write default (#1861).
    // The default was expanded to include file_write and file_delete, but users who
    // had an explicit `require_approval = [...]` entry in their config won't pick up
    // the new default automatically.
    let approval_needs_update = existing
        .get("approval")
        .and_then(|a| a.get("require_approval"))
        .and_then(|r| r.as_array())
        .is_some_and(|list| {
            let has_shell = list.iter().any(|v| v.as_str() == Some("shell_exec"));
            let missing_new = ["file_write", "file_delete", "apply_patch"]
                .iter()
                .any(|tool| !list.iter().any(|v| v.as_str() == Some(*tool)));
            has_shell && missing_new
        });
    if approval_needs_update {
        ui::blank();
        ui::hint(
            "Your require_approval list only contains \"shell_exec\". \
             File operations (file_write, file_delete) now require approval by default.",
        );
        ui::hint(
            "To enable: add \"file_write\" and \"file_delete\" to require_approval in config.toml",
        );
    }

    // 8. Summary
    ui::blank();
    ui::success("Upgrade complete!");
    ui::kv("Backup", &format!("backups/{backup_name}"));
    if !added.is_empty() {
        ui::kv("New fields", &added.len().to_string());
    }
    ui::blank();
}

/// Keep only the `keep` most recent `config-*.toml` backups under `backups_dir`.
/// The embedded `YYYYMMDD-HHMMSS` timestamp sorts lexicographically, so a
/// filename sort gives the same order as a chronological sort.
fn prune_old_config_backups(backups_dir: &std::path::Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(backups_dir) else {
        return;
    };
    let mut files: Vec<std::path::PathBuf> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let name = path.file_name()?.to_str()?;
            if name.starts_with("config-") && name.ends_with(".toml") {
                Some(path)
            } else {
                None
            }
        })
        .collect();
    files.sort();
    if files.len() > keep {
        for old in &files[..files.len() - keep] {
            let _ = std::fs::remove_file(old);
        }
    }
}

/// Generate a local timestamp string in YYYYMMDD-HHMMSS format without external deps.
fn format_local_timestamp() -> String {
    // Use libc to get local time on unix; fallback to UTC seconds on other platforms.
    #[cfg(unix)]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as libc::time_t;
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        // SAFETY: localtime_r is thread-safe and writes into our stack buffer.
        unsafe { libc::localtime_r(&secs, &mut tm) };
        format!(
            "{:04}{:02}{:02}-{:02}{:02}{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        )
    }
    #[cfg(not(unix))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        // Fallback: use UTC (acceptable on Windows where libc tm isn't available)
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        // Simple UTC breakdown
        let days = secs / 86400;
        let day_secs = secs % 86400;
        let hour = day_secs / 3600;
        let min = (day_secs % 3600) / 60;
        let sec = day_secs % 60;
        // Days since 1970-01-01
        let (year, month, day) = days_to_ymd(days);
        format!("{year:04}{month:02}{day:02}-{hour:02}{min:02}{sec:02}")
    }
}

/// Convert days since Unix epoch to (year, month, day). Used only on non-Unix platforms.
#[cfg(not(unix))]
fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    let mut year = 1970u64;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }
    let leap = is_leap(year);
    let month_days: [u64; 12] = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1u64;
    for &md in &month_days {
        if days < md {
            break;
        }
        days -= md;
        month += 1;
    }
    (year, month, days + 1)
}

#[cfg(not(unix))]
fn is_leap(y: u64) -> bool {
    y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
}

/// Find top-level keys in `defaults` that are missing from `existing`.
/// Only checks top-level — does not recurse into sub-tables to avoid
/// injecting partial sections the user intentionally omitted.
fn find_missing_toplevel_keys(existing: &toml::Value, defaults: &toml::Value) -> Vec<String> {
    let (Some(existing_table), Some(defaults_table)) = (existing.as_table(), defaults.as_table())
    else {
        return Vec::new();
    };
    defaults_table
        .keys()
        .filter(|k| !existing_table.contains_key(*k))
        .cloned()
        .collect()
}

/// Initialize vault if it doesn't exist yet (silent no-op if already initialized).
fn init_vault_if_missing(librefang_dir: &std::path::Path) {
    let vault_path = librefang_dir.join("vault.enc");
    if vault_path.exists() {
        return; // Already initialized
    }

    let mut vault = librefang_extensions::vault::CredentialVault::new(vault_path);
    if let Err(e) = vault.init() {
        // Silently skip vault init on failure - it's optional
        tracing::debug!("vault init skipped: {e}");
    }
}

/// Initialize a git repo in ~/.librefang/ for config version control.
fn init_git_if_missing(librefang_dir: &std::path::Path) {
    if librefang_dir.join(".git").exists() {
        return;
    }

    let Ok(status) = std::process::Command::new("git")
        .args(["init", "-q", "-b", "main"])
        .current_dir(librefang_dir)
        .status()
    else {
        tracing::debug!("git not available, skipping repo init");
        return;
    };
    if !status.success() {
        tracing::debug!("git init failed");
        return;
    }

    // Write .gitignore for sensitive/temporary files
    let gitignore = librefang_dir.join(".gitignore");
    if !gitignore.exists() {
        let _ = std::fs::write(
            &gitignore,
            "secrets.env\nvault.enc\ndaemon.json\nlogs/\ncache/\nregistry/\ndata/\nbackups/\n*.db\n*.db-shm\n*.db-wal\n",
        );
    }

    // Initial commit
    let _ = std::process::Command::new("git")
        .args(["add", "-A"])
        .current_dir(librefang_dir)
        .status();
    let _ = std::process::Command::new("git")
        .args(["commit", "-q", "-m", "chore: initial librefang config"])
        .current_dir(librefang_dir)
        .status();
}

/// Quick init: no prompts, auto-detect, write config + .env, print next steps.
fn cmd_init_quick(librefang_dir: &std::path::Path) {
    ui::banner();
    ui::blank();

    let (provider, api_key_env, model) = detect_best_provider();

    write_config_if_missing(librefang_dir, &provider, &model, &api_key_env);

    ui::blank();
    ui::success(&i18n::t("init-quick-success"));
    ui::kv(&i18n::t("label-provider"), &provider);
    ui::kv(&i18n::t("label-model"), &model);
    ui::blank();
    ui::next_steps(&[&i18n::t("init-next-start"), &i18n::t("init-next-chat")]);
}

/// Interactive 5-step onboarding wizard (ratatui TUI).
fn cmd_init_interactive(librefang_dir: &std::path::Path) {
    use tui::screens::init_wizard::{self, InitResult, LaunchChoice};

    match init_wizard::run() {
        InitResult::Completed {
            provider,
            model,
            daemon_started,
            launch,
        } => {
            // Print summary after TUI restores terminal
            ui::blank();
            ui::success(&i18n::t("init-interactive-success"));
            ui::kv(&i18n::t("label-provider"), &provider);
            ui::kv(&i18n::t("label-model"), &model);

            if daemon_started {
                ui::kv_ok(&i18n::t("label-daemon"), "running");
            }
            ui::blank();

            // Execute the user's chosen launch action.
            match launch {
                LaunchChoice::Desktop => {
                    launch_desktop_app(librefang_dir);
                }
                LaunchChoice::Dashboard => {
                    if let Some(base) = find_daemon() {
                        let url = format!("{base}/");
                        ui::success(&i18n::t_args("dashboard-opening", &[("url", &url)]));
                        if !open_in_browser(&url) {
                            ui::hint(&i18n::t_args(
                                "hint-could-not-open-browser-visit",
                                &[("url", &url)],
                            ));
                        }
                    } else {
                        ui::error(&i18n::t("daemon-not-running-start"));
                    }
                }
                LaunchChoice::Chat => {
                    ui::hint(&i18n::t("hint-starting-chat"));
                    ui::blank();
                    // Note: tracing was initialized for stderr (init is a CLI
                    // subcommand).  The chat TUI takes over the terminal with
                    // raw mode so stderr output is suppressed.  We can't
                    // reinitialize tracing (global subscriber is set once).
                    cmd_quick_chat(None, None);
                }
            }
        }
        InitResult::Cancelled => {
            println!("  {}", i18n::t("init-cancelled"));
        }
    }
}

/// Launch the librefang-desktop Tauri app, connecting to the running daemon.
fn launch_desktop_app(_librefang_dir: &std::path::Path) {
    if let Some(path) = desktop_install::find_desktop_binary() {
        desktop_install::launch(&path);
        return;
    }

    // Not installed — offer to download
    if let Some(installed) = desktop_install::prompt_and_install() {
        desktop_install::launch(&installed);
    }
}

/// Auto-detect the best available provider.
///
/// Delegates to the runtime's `detect_available_provider()` which probes 13+
/// providers (OpenAI, Anthropic, Gemini, Groq, DeepSeek, OpenRouter, Mistral,
/// Together, Fireworks, xAI, Perplexity, Cohere, Azure OpenAI) plus the
/// GOOGLE_API_KEY alias.  Falls back to local Ollama, then the interactive
/// free-provider TUI guide.
fn detect_best_provider() -> (String, String, String) {
    // 1. Check all cloud provider API keys via the runtime registry
    if let Some((provider, _model, env_var)) =
        librefang_runtime::drivers::detect_available_provider()
    {
        // Capitalize provider name for display (e.g. "groq" → "Groq")
        let display_name = {
            let mut c = provider.chars();
            match c.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().to_string() + c.as_str(),
            }
        };
        ui::success(&i18n::t_args(
            "detected-provider",
            &[("display", &display_name), ("env_var", env_var)],
        ));
        return (
            provider.to_string(),
            env_var.to_string(),
            default_model_for_provider(provider),
        );
    }

    // 2. Check if Ollama is running locally (no API key needed)
    if check_ollama_available() {
        ui::success(&i18n::t("detected-ollama"));
        return (
            "ollama".to_string(),
            "OLLAMA_API_KEY".to_string(),
            default_model_for_provider("ollama"),
        );
    }

    // 3. No API key found — launch TUI guide to pick a free provider
    {
        if let Some(result) = guide_free_provider_setup() {
            return result;
        }
    }

    // 4. Non-interactive fallback: just print hints
    ui::hint(&i18n::t("hint-no-api-keys"));
    ui::hint(&i18n::t("hint-groq-free"));
    ui::hint(&i18n::t("hint-gemini-free"));
    ui::hint(&i18n::t("hint-deepseek-free"));
    ui::hint(&i18n::t("hint-ollama-local"));
    (
        "groq".to_string(),
        "GROQ_API_KEY".to_string(),
        default_model_for_provider("groq"),
    )
}

/// Interactive TUI guide: help user pick a free LLM provider and set up an API key.
/// Returns `Some((provider, env_var, model))` on success, `None` if user cancels.
fn guide_free_provider_setup() -> Option<(String, String, String)> {
    use tui::screens::free_provider_guide::{self, GuideResult};

    match free_provider_guide::run() {
        GuideResult::Completed { provider, env_var } => {
            ui::success(&i18n::t_args("config-saved-key", &[("env_var", &env_var)]));
            let model = default_model_for_provider(&provider);
            Some((provider, env_var, model))
        }
        GuideResult::Skipped => None,
    }
}

/// Quick probe to check if Ollama is running on localhost.
fn check_ollama_available() -> bool {
    std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], 11434)),
        std::time::Duration::from_millis(500),
    )
    .is_ok()
}

fn render_init_default_config(provider: &str, model: &str, api_key_env: &str) -> String {
    INIT_DEFAULT_CONFIG_TEMPLATE
        .replace("{{provider}}", provider)
        .replace("{{model}}", model)
        .replace("{{api_key_env}}", api_key_env)
}

fn default_model_for_provider(provider: &str) -> String {
    let catalog = librefang_runtime::model_catalog::ModelCatalog::default();
    catalog
        .default_model_for_provider(provider)
        .unwrap_or_else(|| "local-model".to_string())
}

/// Write config.toml if it doesn't already exist.
fn write_config_if_missing(
    librefang_dir: &std::path::Path,
    provider: &str,
    model: &str,
    api_key_env: &str,
) {
    let config_path = librefang_dir.join("config.toml");
    if config_path.exists() {
        ui::check_ok(&i18n::t_args(
            "error-config-exists",
            &[("path", &config_path.display().to_string())],
        ));
    } else {
        let default_config = render_init_default_config(provider, model, api_key_env);
        std::fs::write(&config_path, &default_config).unwrap_or_else(|e| {
            ui::error_with_fix(&i18n::t("error-write-config"), &e.to_string());
            std::process::exit(1);
        });
        restrict_file_permissions(&config_path);
        ui::success(&i18n::t_args(
            "error-config-created",
            &[("path", &config_path.display().to_string())],
        ));
    }

    // Write config.example.toml with the full annotated template for reference
    let example_path = librefang_dir.join("config.example.toml");
    if !example_path.exists() {
        let example_content = include_str!("../templates/init_default_config.toml");
        if let Err(e) = std::fs::write(&example_path, example_content) {
            ui::hint(&format!("Could not write config.example.toml: {e}"));
        }
    }
}

fn daemon_log_path_for_home(home_dir: &std::path::Path) -> PathBuf {
    home_dir.join("logs").join("daemon.log")
}

fn daemon_log_path_for_config(config: Option<&std::path::Path>) -> PathBuf {
    let daemon = daemon_config_context(config);
    if let Some(ref log_dir) = daemon.log_dir {
        log_dir.join("daemon.log")
    } else {
        daemon_log_path_for_home(&daemon.home_dir)
    }
}

fn detached_daemon_args(config: Option<&std::path::Path>, bind: Option<&str>) -> Vec<OsString> {
    let mut args = Vec::new();
    if let Some(path) = config {
        args.push(OsString::from("--config"));
        args.push(path.as_os_str().to_owned());
    }
    args.push(OsString::from("start"));
    args.push(OsString::from("--spawned"));
    if let Some(addr) = bind {
        args.push(OsString::from("--bind"));
        args.push(OsString::from(addr));
    }
    args
}

fn spawn_detached_daemon(
    config: Option<&std::path::Path>,
    log_path: &std::path::Path,
    bind: Option<&str>,
) -> Result<std::process::Child, String> {
    let exe = std::env::current_exe().map_err(|e| format!("resolve current executable: {e}"))?;
    if let Some(log_dir) = log_path.parent() {
        std::fs::create_dir_all(log_dir)
            .map_err(|e| format!("create log directory {}: {e}", log_dir.display()))?;
        restrict_dir_permissions(log_dir);
    }

    let stdout = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .map_err(|e| format!("open daemon log {}: {e}", log_path.display()))?;
    let stderr = stdout
        .try_clone()
        .map_err(|e| format!("clone daemon log handle {}: {e}", log_path.display()))?;

    let mut command = std::process::Command::new(exe);
    command
        .args(detached_daemon_args(config, bind))
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .envs(std::env::vars());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;

        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }

    command
        .spawn()
        .map_err(|e| format!("spawn detached daemon: {e}"))
}

/// Generate a daily log path for the current daemon start.
/// Returns e.g. ~/.librefang/logs/daemon-2026-04-23.log
/// Same day restarts reuse the same file.
fn timestamped_log_path(config: Option<&std::path::Path>) -> std::path::PathBuf {
    let daemon = daemon_config_context(config);
    let log_dir = daemon
        .log_dir
        .unwrap_or_else(|| daemon.home_dir.join("logs"));
    let date = chrono_lite_date();
    log_dir.join(format!("daemon-{date}.log"))
}

/// Lightweight date string (YYYY-MM-DD) without external dependencies.
fn chrono_lite_date() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let days = secs / 86400;
    let mut year = 1970;
    let mut remaining_days = days as i64;
    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if remaining_days < days_in_year {
            break;
        }
        remaining_days -= days_in_year;
        year += 1;
    }
    let month_days = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month: u64 = 1;
    let mut day: i64 = remaining_days + 1;
    let mut md: i64 = if is_leap_year(year) { 29 } else { 28 };
    while day > md {
        day -= md;
        month += 1;
        md = month_days
            .get((month.saturating_sub(1)) as usize)
            .copied()
            .unwrap_or(28) as i64;
    }
    format!("{:04}-{:02}-{:02}", year, month, day)
}

fn is_leap_year(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}

/// Prune rotated daemon logs older than `max_age_days`, keeping the log dir tidy.
fn prune_rotated_logs(config: Option<&std::path::Path>, max_age_days: u64) {
    let daemon = daemon_config_context(config);
    let log_dir = daemon
        .log_dir
        .unwrap_or_else(|| daemon.home_dir.join("logs"));
    let cutoff = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .saturating_sub(max_age_days.saturating_mul(86400));

    let entries = match std::fs::read_dir(&log_dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if !name.starts_with("daemon-") || !name.ends_with(".log") {
            continue;
        }
        // Parse date from filename: daemon-YYYY-MM-DD.log
        let date_str = name
            .strip_prefix("daemon-")
            .and_then(|s| s.strip_suffix(".log"));
        let is_old = date_str
            .and_then(parse_daily_date_timestamp)
            .map(|ts| ts < cutoff)
            .unwrap_or(false);
        if is_old {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Parse YYYY-MM-DD to Unix seconds at 00:00:00 UTC.
fn parse_daily_date_timestamp(s: &str) -> Option<u64> {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 3 {
        return None;
    }
    let year: u64 = parts[0].parse().ok()?;
    let month: u64 = parts[1].parse().ok()?;
    let day: u64 = parts[2].parse().ok()?;
    Some(days_since_epoch(year, month, day) * 86400)
}

fn days_since_epoch(year: u64, month: u64, day: u64) -> u64 {
    let mut days = 0;
    for y in 1970..year {
        days += if is_leap_year(y as i64) { 366 } else { 365 };
    }
    let month_days = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    for m in 1..month {
        days += month_days.get((m - 1) as usize).copied().unwrap_or(28) as u64;
    }
    if is_leap_year(year as i64) && month > 2 {
        days += 1;
    }
    days + day - 1
}

/// Guard that tees all stdout/stderr to a log file in foreground mode.
/// On drop, restores original stdout/stderr and joins the tee thread.
#[cfg(unix)]
struct ForegroundTeeGuard {
    _pipe_fd: RawFd, // kept alive to keep pipe open until guard drops
}

#[cfg(unix)]
impl Drop for ForegroundTeeGuard {
    fn drop(&mut self) {
        // Restore original stdout/stderr
        unsafe {
            libc::dup2(self._pipe_fd, libc::STDOUT_FILENO);
            libc::dup2(self._pipe_fd, libc::STDERR_FILENO);
            libc::close(self._pipe_fd);
        }
    }
}

/// Set up tee for --foreground mode: redirect stdout/stderr to a pipe,
/// spawn a background thread that copies to both terminal and log file.
#[cfg(unix)]
fn setup_foreground_tee(log_path: &std::path::Path) -> ForegroundTeeGuard {
    // Create pipe for stdout+stderr (we'll write to it, background thread reads)
    let mut fds = [0i32, 0i32];
    unsafe {
        libc::pipe(fds.as_mut_ptr());
    }
    let pipe_write = fds[1];
    let pipe_read = fds[0];

    // Save copy of original stdout/stderr (to restore on drop)
    let stdout_copy = unsafe { libc::dup(libc::STDOUT_FILENO) };
    let stderr_copy = unsafe { libc::dup(libc::STDERR_FILENO) };

    // Redirect stdout and stderr to the pipe
    unsafe {
        libc::dup2(pipe_write, libc::STDOUT_FILENO);
        libc::dup2(pipe_write, libc::STDERR_FILENO);
        libc::close(pipe_write);
    }

    // Create log file (append mode)
    let log_file = std::sync::Mutex::new(
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)
            .expect("daemon log file"),
    );

    // Spawn background thread that reads from pipe and writes to both terminal and log
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            let n =
                unsafe { libc::read(pipe_read, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if n <= 0 {
                // EOF or error — exit; guard Drop will restore stdout/stderr
                unsafe { libc::close(pipe_read) };
                break;
            }
            // Write to terminal (original stdout/stderr)
            unsafe {
                libc::write(stdout_copy, buf.as_ptr() as *const libc::c_void, n as usize);
                libc::write(stderr_copy, buf.as_ptr() as *const libc::c_void, n as usize);
            }
            // Write to log file
            if let Ok(mut f) = log_file.lock() {
                let _ = f.write_all(&buf[..n as usize]);
                let _ = f.flush();
            }
        }
        // guard Drop closes stdout_copy/stderr_copy; pipe_read is closed above on break
    });

    ForegroundTeeGuard {
        _pipe_fd: stdout_copy,
    }
}

/// Ensure LibreFang is initialized (config.toml exists). Auto-runs quick init on first run.
fn ensure_initialized(config: &Option<PathBuf>) {
    match config {
        None => {
            let home = cli_librefang_home();
            if !home.join("config.toml").exists() {
                ui::hint("First run detected — running quick setup...");
                cmd_init(true);
            }
        }
        Some(path) => {
            if !path.exists() {
                ui::error_with_fix(
                    &format!("Config file not found: {}", path.display()),
                    "Run `librefang init` to create a default config at ~/.librefang/config.toml, or check the --config path.",
                );
                std::process::exit(1);
            }
        }
    }
}

fn cmd_start(
    config: Option<PathBuf>,
    tail: bool,
    spawned: bool,
    foreground: bool,
    bind: Option<String>,
) {
    ensure_initialized(&config);

    let daemon = daemon_config_context(config.as_deref());
    if let Some(base) = find_daemon_in_home(&daemon.home_dir) {
        ui::error_with_fix(
            &i18n::t_args("daemon-already-running", &[("url", &base)]),
            &i18n::t("daemon-already-running-fix"),
        );
        std::process::exit(1);
    }

    if !spawned && !foreground {
        let log_path = daemon_log_path_for_config(config.as_deref());
        let mut child = spawn_detached_daemon(config.as_deref(), &log_path, bind.as_deref())
            .unwrap_or_else(|e| {
                ui::error_with_fix(&i18n::t("daemon-launch-fail"), &e);
                std::process::exit(1);
            });

        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(base) = find_daemon_in_home(&daemon.home_dir) {
                let pid = child.id();
                std::mem::forget(child);
                ui::success(&i18n::t("daemon-started-bg"));
                ui::kv(&i18n::t("label-pid"), &pid.to_string());
                ui::kv(&i18n::t("label-api"), &base);
                ui::kv(&i18n::t("label-dashboard"), &format!("{base}/"));
                ui::kv(&i18n::t("label-log"), &log_path.display().to_string());
                if tail {
                    ui::hint(&i18n::t("hint-tail-stop"));
                    ui::blank();
                    show_log_file(&log_path, 50, true);
                } else {
                    ui::hint(&i18n::t("hint-stop-daemon"));
                }
                return;
            }

            match child.try_wait() {
                Ok(Some(status)) => {
                    ui::error_with_fix(
                        &i18n::t_args("daemon-bg-exited", &[("status", &status.to_string())]),
                        &i18n::t_args(
                            "daemon-bg-exited-fix",
                            &[("path", &log_path.display().to_string())],
                        ),
                    );
                    std::process::exit(1);
                }
                Ok(None) => {}
                Err(e) => {
                    ui::error_with_fix(
                        &i18n::t("daemon-bg-wait-fail"),
                        &i18n::t_args(
                            "daemon-bg-wait-fail-fix",
                            &[
                                ("error", &e.to_string()),
                                ("path", &log_path.display().to_string()),
                            ],
                        ),
                    );
                    std::process::exit(1);
                }
            }

            if Instant::now() >= deadline {
                let pid = child.id();
                std::mem::forget(child);
                ui::success(&i18n::t("daemon-still-starting"));
                ui::kv(&i18n::t("label-pid"), &pid.to_string());
                ui::kv(&i18n::t("label-log"), &log_path.display().to_string());
                if tail {
                    ui::hint(&i18n::t("hint-tail-stop"));
                    ui::blank();
                    show_log_file(&log_path, 50, true);
                } else {
                    ui::hint(&i18n::t("hint-check-status"));
                }
                return;
            }

            std::thread::sleep(Duration::from_millis(250));
        }
    }

    ui::banner();
    ui::blank();
    println!("  {}", i18n::t("daemon-starting"));
    ui::blank();

    // For --foreground mode, tee stdout/stderr to both the terminal and a time-stamped
    // log file. Detached mode keeps appending to the stable daemon.log.
    let log_path = if foreground {
        // Prune rotated logs older than LOG_RETENTION_DAYS, then start a fresh daily file.
        prune_rotated_logs(config.as_deref(), LOG_RETENTION_DAYS);
        timestamped_log_path(config.as_deref())
    } else {
        daemon_log_path_for_config(config.as_deref())
    };
    #[cfg(unix)]
    let _foreground_guard = if foreground {
        Some(setup_foreground_tee(&log_path))
    } else {
        None
    };
    ui::kv(&i18n::t("label-log"), &log_path.display().to_string());

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let kernel = match LibreFangKernel::boot(config.as_deref()) {
            Ok(k) => k,
            Err(e) => {
                boot_kernel_error(&e);
                std::process::exit(1);
            }
        };

        let cfg = kernel.config_ref();
        // CLI --bind wins over LIBREFANG_LISTEN (applied by kernel boot) and config.toml.
        let listen_addr = bind.clone().unwrap_or_else(|| cfg.api_listen.clone());
        let daemon_info_path = kernel.home_dir().join("daemon.json");
        let provider = cfg.default_model.provider.clone();
        let model = cfg.default_model.model.clone();
        let agent_count = kernel.agent_registry().count();
        let model_count = kernel
            .model_catalog_ref()
            .read()
            .map(|c| c.list_models().len())
            .unwrap_or(0);

        ui::success(&i18n::t_args(
            "kernel-booted",
            &[("provider", &provider), ("model", &model)],
        ));
        if model_count > 0 {
            ui::success(&i18n::t_args(
                "models-available",
                &[("count", &model_count.to_string())],
            ));
        }
        if agent_count > 0 {
            ui::success(&i18n::t_args(
                "agents-loaded",
                &[("count", &agent_count.to_string())],
            ));
        }
        ui::blank();
        ui::kv(&i18n::t("label-api"), &format!("http://{listen_addr}"));
        ui::kv(
            &i18n::t("label-dashboard"),
            &format!("http://{listen_addr}/"),
        );
        ui::kv(&i18n::t("label-provider"), &provider);
        ui::kv(&i18n::t("label-model"), &model);
        ui::blank();
        ui::hint(&i18n::t("hint-open-dashboard"));
        ui::hint(&i18n::t("hint-stop-daemon"));
        ui::blank();

        if let Err(e) =
            librefang_api::server::run_daemon(kernel, &listen_addr, Some(&daemon_info_path)).await
        {
            ui::error(&i18n::t_args("daemon-error", &[("error", &e.to_string())]));
            std::process::exit(1);
        }

        ui::blank();
        println!("  {}", i18n::t("daemon-stopped"));
    });
}

/// Read the daemon api_key from the effective CLI config (if any).
///
/// Returns `None` when the key is missing, empty, or whitespace-only —
/// meaning the daemon is running in public (unauthenticated) mode.
fn read_api_key() -> Option<String> {
    daemon_config_context(None).api_key
}

fn cmd_stop(config: Option<PathBuf>) {
    let daemon = daemon_config_context(config.as_deref());
    match find_daemon_in_home(&daemon.home_dir) {
        Some(base) => {
            let client = daemon_client_with_api_key(daemon.api_key.as_deref());
            match client.post(format!("{base}/api/shutdown")).send() {
                Ok(r) if r.status().is_success() => {
                    // Wait for daemon to actually stop (up to 5 seconds)
                    for _ in 0..10 {
                        std::thread::sleep(std::time::Duration::from_millis(500));
                        if find_daemon_in_home(&daemon.home_dir).is_none() {
                            ui::success(&i18n::t("daemon-stopped-ok"));
                            return;
                        }
                    }
                    // Still alive — force kill via PID
                    if let Some(info) = read_daemon_info(&daemon.home_dir) {
                        force_kill_pid(info.pid);
                        let _ = std::fs::remove_file(daemon.home_dir.join("daemon.json"));
                    }
                    ui::success(&i18n::t("daemon-stopped-forced"));
                }
                Ok(r) => {
                    ui::error(&i18n::t_args(
                        "shutdown-request-fail",
                        &[("status", &r.status().to_string())],
                    ));
                }
                Err(e) => {
                    ui::error(&i18n::t_args(
                        "could-not-reach-daemon",
                        &[("error", &e.to_string())],
                    ));
                }
            }
        }
        None => {
            ui::warn_with_fix(
                &i18n::t("daemon-no-running-found"),
                &i18n::t("daemon-no-running-found-fix"),
            );
        }
    }
}

fn cmd_restart(config: Option<PathBuf>, tail: bool, foreground: bool) {
    let daemon = daemon_config_context(config.as_deref());
    if find_daemon_in_home(&daemon.home_dir).is_some() {
        ui::hint(&i18n::t("daemon-restarting"));
        cmd_stop(config.clone());
    } else {
        ui::hint(&i18n::t("daemon-no-running-starting"));
    }

    cmd_start(config, tail, false, foreground, None);
}

fn force_kill_pid(pid: u32) {
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .output();
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .output();
    }
}

/// Show context-aware error for kernel boot failures.
fn boot_kernel_error(e: &librefang_kernel::error::KernelError) {
    let msg = e.to_string();
    if msg.contains("parse") || msg.contains("toml") || msg.contains("config") {
        ui::error_with_fix(
            &i18n::t("error-boot-config"),
            &i18n::t("error-boot-config-fix"),
        );
    } else if msg.contains("database") || msg.contains("locked") || msg.contains("sqlite") {
        ui::error_with_fix(&i18n::t("error-boot-db"), &i18n::t("error-boot-db-fix"));
    } else if msg.contains("key") || msg.contains("API") || msg.contains("auth") {
        ui::error_with_fix(&i18n::t("error-boot-auth"), &i18n::t("error-boot-auth-fix"));
    } else {
        ui::error_with_fix(
            &i18n::t_args("error-boot-generic", &[("error", &msg)]),
            &i18n::t("error-boot-generic-fix"),
        );
    }
}

struct PreparedAgentManifest {
    manifest: AgentManifest,
    manifest_toml: String,
    source_label: String,
}

fn cmd_agent_spawn(
    config: Option<PathBuf>,
    manifest_path: PathBuf,
    name_override: Option<String>,
    dry_run: bool,
) {
    let prepared = prepared_agent_manifest_from_path(&manifest_path, name_override.as_deref());
    if dry_run {
        preview_agent_manifest(&prepared);
        return;
    }
    spawn_prepared_agent(config, prepared);
}

fn cmd_spawn_alias(
    config: Option<PathBuf>,
    target: Option<String>,
    template_path: Option<PathBuf>,
    name_override: Option<String>,
    dry_run: bool,
) {
    if template_path.is_some() && target.is_some() {
        ui::error_with_fix(
            "Choose either a positional target or `--template`, not both.",
            "Use `librefang spawn coder` or `librefang spawn --template agents/custom/my-agent.toml`.",
        );
        std::process::exit(1);
    }

    if target.is_none() && template_path.is_none() {
        if name_override.is_some() {
            ui::error_with_fix(
                "`--name` requires a template name or manifest path.",
                "Use `librefang spawn coder --name backend-coder` or `librefang spawn --template path/to/agent.toml --name backend-coder`.",
            );
            std::process::exit(1);
        }
        if dry_run {
            ui::error_with_fix(
                "Dry run needs a template name or manifest path.",
                "Use `librefang spawn coder --dry-run` or `librefang spawn --template path/to/agent.toml --dry-run`.",
            );
            std::process::exit(1);
        }
        cmd_agent_new(config, None);
        return;
    }

    if let Some(path) = template_path {
        let prepared = prepared_agent_manifest_from_path(&path, name_override.as_deref());
        if dry_run {
            preview_agent_manifest(&prepared);
        } else {
            spawn_prepared_agent(config, prepared);
        }
        return;
    }

    let target = target.expect("target checked above");
    let manifest_path = PathBuf::from(&target);
    if manifest_path.exists() {
        let prepared = prepared_agent_manifest_from_path(&manifest_path, name_override.as_deref());
        if dry_run {
            preview_agent_manifest(&prepared);
        } else {
            spawn_prepared_agent(config, prepared);
        }
        return;
    }

    let templates = templates::load_all_templates();
    let template = templates
        .iter()
        .find(|t| t.name == target)
        .unwrap_or_else(|| {
            ui::error_with_fix(
                &format!("Template or manifest path not found: {target}"),
                "Run `librefang agent new` to browse templates, or pass a valid manifest path.",
            );
            std::process::exit(1);
        });
    if dry_run {
        let prepared = prepared_agent_manifest_from_template(template, name_override.as_deref());
        preview_agent_manifest(&prepared);
    } else {
        spawn_template_agent(config, template, name_override.as_deref());
    }
}

fn prepared_agent_manifest_from_path(
    manifest_path: &std::path::Path,
    name_override: Option<&str>,
) -> PreparedAgentManifest {
    if !manifest_path.exists() {
        ui::error_with_fix(
            &i18n::t_args(
                "manifest-not-found",
                &[("path", &manifest_path.display().to_string())],
            ),
            &i18n::t("manifest-not-found-fix"),
        );
        std::process::exit(1);
    }

    let contents = std::fs::read_to_string(manifest_path).unwrap_or_else(|e| {
        eprintln!(
            "{}",
            i18n::t_args("error-reading-manifest", &[("error", &e.to_string())])
        );
        std::process::exit(1);
    });

    prepared_agent_manifest_from_contents(
        &contents,
        manifest_path.display().to_string(),
        name_override,
    )
}

fn prepared_agent_manifest_from_template(
    template: &templates::AgentTemplate,
    name_override: Option<&str>,
) -> PreparedAgentManifest {
    prepared_agent_manifest_from_contents(
        &template.content,
        format!("template:{}", template.name),
        name_override,
    )
}

fn prepared_agent_manifest_from_contents(
    contents: &str,
    source_label: String,
    name_override: Option<&str>,
) -> PreparedAgentManifest {
    let mut manifest: AgentManifest = toml::from_str(contents).unwrap_or_else(|e| {
        ui::error_with_fix(
            &format!("Failed to parse agent manifest from {source_label}: {e}"),
            "Check the manifest TOML syntax and required fields.",
        );
        std::process::exit(1);
    });

    if let Some(name) = name_override {
        manifest.name = name.to_string();
    }

    let manifest_toml = if name_override.is_some() {
        toml::to_string_pretty(&manifest).unwrap_or_else(|e| {
            ui::error(&format!("Failed to serialize updated manifest: {e}"));
            std::process::exit(1);
        })
    } else {
        contents.to_string()
    };

    PreparedAgentManifest {
        manifest,
        manifest_toml,
        source_label,
    }
}

fn preview_agent_manifest(prepared: &PreparedAgentManifest) {
    ui::section("Agent Dry Run");
    ui::kv("Source", &prepared.source_label);
    ui::kv("Name", &prepared.manifest.name);
    ui::kv("Version", &prepared.manifest.version);
    ui::kv("Module", &prepared.manifest.module);
    ui::kv(
        "Model",
        &format!(
            "{}/{}",
            prepared.manifest.model.provider, prepared.manifest.model.model
        ),
    );
    ui::kv(
        "Tools",
        &prepared.manifest.capabilities.tools.len().to_string(),
    );
    ui::kv("Skills", &prepared.manifest.skills.len().to_string());
    if !prepared.manifest.tags.is_empty() {
        ui::kv("Tags", &prepared.manifest.tags.join(", "));
    }
    if !prepared.manifest.description.is_empty() {
        ui::kv("Description", &prepared.manifest.description);
    }
    ui::success("Manifest parsed successfully. No agent was spawned.");
}

fn spawn_prepared_agent(config: Option<PathBuf>, prepared: PreparedAgentManifest) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(
            client
                .post(format!("{base}/api/agents"))
                .json(&serde_json::json!({"manifest_toml": prepared.manifest_toml}))
                .send(),
        );
        if body.get("agent_id").is_some() {
            println!("{}", i18n::t("agent-spawn-success"));
            println!("  ID:   {}", body["agent_id"].as_str().unwrap_or("?"));
            println!(
                "  Name: {}",
                body["name"]
                    .as_str()
                    .unwrap_or(prepared.manifest.name.as_str())
            );
        } else {
            eprintln!(
                "{}",
                i18n::t_args(
                    "agent-spawn-agent-failed",
                    &[("error", body["error"].as_str().unwrap_or("Unknown error"))]
                )
            );
            std::process::exit(1);
        }
    } else {
        let agent_name = prepared.manifest.name.clone();
        let kernel = boot_kernel(config);
        match kernel.spawn_agent_with_source(prepared.manifest, None) {
            Ok(id) => {
                println!("{}", i18n::t("agent-spawn-inprocess-mode"));
                println!("  ID:   {id}");
                println!("  Name: {agent_name}");
                println!("\n  {}", i18n::t("agent-note-lost"));
                println!("  {}", i18n::t("agent-note-persistent"));
            }
            Err(e) => {
                eprintln!(
                    "{}",
                    i18n::t_args("agent-spawn-agent-failed", &[("error", &e.to_string())])
                );
                std::process::exit(1);
            }
        }
    }
}

fn cmd_agent_list(config: Option<PathBuf>, json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(client.get(format!("{base}/api/agents")).send());

        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
            return;
        }

        let agents = body
            .get("items")
            .and_then(|v| v.as_array())
            .or_else(|| body.as_array());

        match agents {
            Some(agents) if agents.is_empty() => println!("{}", i18n::t("agent-no-agents")),
            Some(agents) => {
                println!(
                    "{:<38} {:<16} {:<10} {:<12} MODEL",
                    "ID", "NAME", "STATE", "PROVIDER"
                );
                println!("{}", "-".repeat(95));
                for a in agents {
                    println!(
                        "{:<38} {:<16} {:<10} {:<12} {}",
                        a["id"].as_str().unwrap_or("?"),
                        a["name"].as_str().unwrap_or("?"),
                        a["state"].as_str().unwrap_or("?"),
                        a["model_provider"].as_str().unwrap_or("?"),
                        a["model_name"].as_str().unwrap_or("?"),
                    );
                }
            }
            None => println!("{}", i18n::t("agent-no-agents")),
        }
    } else {
        let kernel = boot_kernel(config);
        let agents = kernel.agent_registry().list();

        if json {
            let list: Vec<serde_json::Value> = agents
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "id": e.id.to_string(),
                        "name": e.name,
                        "state": format!("{:?}", e.state),
                        "created_at": e.created_at.to_rfc3339(),
                    })
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&list).unwrap_or_default()
            );
            return;
        }

        if agents.is_empty() {
            println!("{}", i18n::t("agent-no-agents"));
            return;
        }

        println!("{:<38} {:<20} {:<12} CREATED", "ID", "NAME", "STATE");
        println!("{}", "-".repeat(85));
        for entry in agents {
            println!(
                "{:<38} {:<20} {:<12} {}",
                entry.id,
                entry.name,
                format!("{:?}", entry.state),
                entry.created_at.format("%Y-%m-%d %H:%M")
            );
        }
    }
}

fn cmd_agent_chat(config: Option<PathBuf>, agent_id_str: &str) {
    ensure_initialized(&config);
    tui::chat_runner::run_chat_tui(config, Some(agent_id_str.to_string()));
}

fn cmd_agent_kill(config: Option<PathBuf>, agent_id_str: &str) {
    if let Some(base) = find_daemon() {
        let agent_id = resolve_agent_id(&base, agent_id_str);
        let client = daemon_client();
        let body = daemon_json(
            client
                .delete(format!("{base}/api/agents/{agent_id}"))
                .send(),
        );
        if body.get("status").is_some() {
            println!("{}", i18n::t_args("agent-killed", &[("id", &agent_id)]));
        } else {
            eprintln!(
                "{}",
                i18n::t_args(
                    "agent-kill-failed",
                    &[("error", body["error"].as_str().unwrap_or("Unknown error"))]
                )
            );
            std::process::exit(1);
        }
    } else {
        let agent_id: AgentId = agent_id_str.parse().unwrap_or_else(|_| {
            eprintln!(
                "{}",
                i18n::t_args("agent-invalid-id", &[("id", agent_id_str)])
            );
            std::process::exit(1);
        });
        let kernel = boot_kernel(config);
        match kernel.kill_agent(agent_id) {
            Ok(()) => println!(
                "{}",
                i18n::t_args("agent-killed", &[("id", &agent_id.to_string())])
            ),
            Err(e) => {
                eprintln!(
                    "{}",
                    i18n::t_args("agent-kill-failed", &[("error", &e.to_string())])
                );
                std::process::exit(1);
            }
        }
    }
}

fn cmd_agent_set(agent_id_str: &str, field: &str, value: &str) {
    match field {
        "model" => {
            if let Some(base) = find_daemon() {
                let agent_id = resolve_agent_id(&base, agent_id_str);
                let client = daemon_client();
                let body = daemon_json(
                    client
                        .put(format!("{base}/api/agents/{agent_id}/model"))
                        .json(&serde_json::json!({"model": value}))
                        .send(),
                );
                if body.get("status").is_some() {
                    println!("Agent {agent_id} model set to {value}.");
                } else {
                    eprintln!(
                        "Failed to set model: {}",
                        body["error"].as_str().unwrap_or("Unknown error")
                    );
                    std::process::exit(1);
                }
            } else {
                eprintln!("No running daemon found. Start one with: librefang start");
                std::process::exit(1);
            }
        }
        _ => {
            eprintln!("Unknown field: {field}. Supported fields: model");
            std::process::exit(1);
        }
    }
}

fn cmd_agent_new(config: Option<PathBuf>, template_name: Option<String>) {
    let all_templates = templates::load_all_templates();
    if all_templates.is_empty() {
        ui::error_with_fix(
            "No agent templates found",
            "Run `librefang init` to set up the agents directory",
        );
        std::process::exit(1);
    }

    // Resolve template: by name or interactive picker
    let chosen = match template_name {
        Some(ref name) => match all_templates.iter().find(|t| t.name == *name) {
            Some(t) => t,
            None => {
                ui::error_with_fix(
                    &format!("Template '{name}' not found"),
                    "Run `librefang agent new` to see available templates",
                );
                std::process::exit(1);
            }
        },
        None => {
            ui::section(&i18n::t("section-agent-templates"));
            ui::blank();
            for (i, t) in all_templates.iter().enumerate() {
                let desc = if t.description.is_empty() {
                    String::new()
                } else {
                    format!("  {}", t.description)
                };
                println!(
                    "    {:>2}. {:<22}{}",
                    i + 1,
                    t.name,
                    colored::Colorize::dimmed(desc.as_str())
                );
            }
            ui::blank();
            let choice = prompt_input("  Choose template [1]: ");
            let idx = if choice.is_empty() {
                0
            } else {
                choice
                    .parse::<usize>()
                    .unwrap_or(1)
                    .saturating_sub(1)
                    .min(all_templates.len() - 1)
            };
            &all_templates[idx]
        }
    };

    // Spawn the agent
    spawn_template_agent(config, chosen, None);
}

/// Spawn an agent from a template, via daemon or in-process.
fn spawn_template_agent(
    config: Option<PathBuf>,
    template: &templates::AgentTemplate,
    name_override: Option<&str>,
) {
    let prepared = prepared_agent_manifest_from_template(template, name_override);
    let agent_name = prepared.manifest.name.clone();

    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(
            client
                .post(format!("{base}/api/agents"))
                .json(&serde_json::json!({"manifest_toml": prepared.manifest_toml}))
                .send(),
        );
        if let Some(id) = body["agent_id"].as_str() {
            ui::blank();
            ui::success(&i18n::t_args("agent-spawned", &[("name", &agent_name)]));
            ui::kv(&i18n::t("label-id"), id);
            if let Some(model) = body["model_name"].as_str() {
                let provider = body["model_provider"].as_str().unwrap_or("?");
                ui::kv(&i18n::t("label-model"), &format!("{provider}/{model}"));
            }
            ui::blank();
            ui::hint(&i18n::t_args(
                "hint-chat-with-agent",
                &[("name", &agent_name)],
            ));
        } else {
            ui::error(&i18n::t_args(
                "agent-spawn-failed",
                &[("error", body["error"].as_str().unwrap_or("Unknown error"))],
            ));
            std::process::exit(1);
        }
    } else {
        let kernel = boot_kernel(config);
        match kernel.spawn_agent(prepared.manifest) {
            Ok(id) => {
                ui::blank();
                ui::success(&i18n::t_args(
                    "agent-spawned-inprocess",
                    &[("name", &agent_name)],
                ));
                ui::kv(&i18n::t("label-id"), &id.to_string());
                ui::blank();
                ui::hint(&i18n::t_args(
                    "hint-chat-with-agent",
                    &[("name", &agent_name)],
                ));
                ui::hint(&i18n::t("hint-agent-lost-on-exit"));
                ui::hint(&i18n::t("hint-persistent-agents"));
            }
            Err(e) => {
                ui::error(&i18n::t_args(
                    "agent-spawn-agent-failed",
                    &[("error", &e.to_string())],
                ));
                std::process::exit(1);
            }
        }
    }
}

/// Render the daemon status page.
///
/// Layered data model:
/// - **Local** (always available): `daemon.json` + `config.toml` fields the
///   CLI reads directly, so we never show `?` for information we already have.
/// - **Public** (daemon alive, no auth): `/api/health` for liveness.
/// - **Authenticated** (requires `api_key`): `/api/status` for agent list,
///   session count, and memory usage. When the key is missing we show a
///   locked section with a one-line fix hint instead of leaking empty fields.
fn cmd_status(config: Option<PathBuf>, json: bool, verbose: bool, quiet: bool, watch: Option<u64>) {
    if let Some(secs) = watch {
        let interval = std::time::Duration::from_secs(secs.max(1));
        // Watch mode: redraw indefinitely. A non-zero exit code from a single
        // iteration just means "daemon is currently down or degraded" — we
        // don't bail out of the watch loop for that, the whole point is to
        // keep watching. Ctrl+C (handled upstream in main) is the exit.
        loop {
            // ANSI: clear screen + home cursor. Falls back to ugly output on
            // terminals that don't speak ANSI, which is acceptable for a
            // mode the user explicitly opted into.
            print!("\x1b[2J\x1b[H");
            use std::io::Write;
            let _ = std::io::stdout().flush();
            let _ = render_status_once(config.clone(), false, verbose, false);
            ui::blank();
            println!(
                "  {} (refreshing every {}s, Ctrl+C to exit)",
                "hint:".dimmed(),
                secs.max(1),
            );
            std::thread::sleep(interval);
        }
    }

    let code = render_status_once(config, json, verbose, quiet);
    if code != 0 {
        std::process::exit(code);
    }
}

fn render_status_once(config: Option<PathBuf>, json: bool, verbose: bool, quiet: bool) -> i32 {
    let daemon = daemon_config_context(config.as_deref());
    if let Some(base) = find_daemon_in_home(&daemon.home_dir) {
        render_status_daemon(config.as_deref(), &base, &daemon, json, verbose, quiet)
    } else {
        render_status_inprocess(config, json, quiet)
    }
}

fn render_status_daemon(
    config: Option<&std::path::Path>,
    base: &str,
    daemon: &DaemonConfigContext,
    json: bool,
    verbose: bool,
    quiet: bool,
) -> i32 {
    let info = read_daemon_info(&daemon.home_dir);
    let (health, health_latency) = fetch_health_timed(base);
    let detail = daemon
        .api_key
        .as_deref()
        .and_then(|k| fetch_status_detail(base, k));
    let cfg = load_config(config);

    let exit_code = classify_exit(health.as_ref());
    let is_public_bind = info
        .as_ref()
        .map(|i| listener_is_public(&i.listen_addr))
        .unwrap_or(false);
    let (key_env, key_present, key_required) = provider_key_state(&cfg);
    let uptime = uptime_secs(info.as_ref(), detail.as_ref());

    if quiet {
        return render_status_quiet_daemon(
            base,
            info.as_ref(),
            health.as_ref(),
            detail.as_ref(),
            uptime,
            exit_code,
        );
    }

    if json {
        let merged = serde_json::json!({
            "daemon": true,
            "api": base,
            "dashboard": format!("{base}/"),
            "home": daemon.home_dir.display().to_string(),
            "daemon_info": info.as_ref().map(|i| serde_json::json!({
                "pid": i.pid,
                "listen_addr": i.listen_addr,
                "started_at": i.started_at,
                "version": i.version,
                "platform": i.platform,
                "publicly_bound": listener_is_public(&i.listen_addr),
            })),
            "health": health,
            "health_latency_ms": health_latency.map(|d| d.as_millis() as u64),
            "default_provider": cfg.default_model.provider,
            "default_model": cfg.default_model.model,
            "default_model_api_key_env": key_env,
            "default_model_api_key_present": key_present,
            "default_model_api_key_required": key_required,
            "detail": detail,
            "uptime_seconds": uptime_secs(info.as_ref(), detail.as_ref()),
            "authenticated": detail.is_some(),
            "exit_code": exit_code,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&merged).unwrap_or_default()
        );
        return exit_code;
    }

    // --- Overview -----------------------------------------------------------
    ui::section(&i18n::t("section-daemon-status"));
    ui::blank();

    let (status_label, status_good) = match health.as_ref() {
        Some(h) => match h["status"].as_str() {
            Some("ok") => ("ok".to_string(), true),
            Some(other) => (other.to_string(), false),
            None => ("unreachable".to_string(), false),
        },
        None => ("unreachable".to_string(), false),
    };
    if status_good {
        ui::kv_ok(&i18n::t("label-status"), &status_label);
    } else {
        ui::kv_warn(&i18n::t("label-status"), &status_label);
    }

    if let Some(v) = info
        .as_ref()
        .map(|i| i.version.as_str())
        .or_else(|| health.as_ref().and_then(|h| h["version"].as_str()))
    {
        ui::kv(&i18n::t("label-version"), v);
    }
    if let Some(info) = info.as_ref() {
        ui::kv(&i18n::t("label-pid"), &info.pid.to_string());
    }
    if let Some(u) = uptime_secs(info.as_ref(), detail.as_ref()) {
        ui::kv(&i18n::t("label-uptime"), &format_uptime(u));
    }
    if let Some(lat) = health_latency {
        ui::kv(&i18n::t("label-response"), &format_latency(lat));
    }
    // (B) 0.0.0.0 listener: surface the risk inline on the API row so the
    // user sees the bind scope without having to cross-reference anything.
    if is_public_bind {
        ui::kv_warn(
            &i18n::t("label-api"),
            &format!("{base}  \u{26A0} {}", i18n::t("warn-public-bind")),
        );
    } else {
        ui::kv(&i18n::t("label-api"), base);
    }
    ui::kv(&i18n::t("label-dashboard"), &format!("{base}/"));
    ui::kv(
        &i18n::t("label-home"),
        &daemon.home_dir.display().to_string(),
    );
    if let Some(info) = info.as_ref() {
        ui::kv(&i18n::t("label-platform"), &info.platform);
    }
    if let Some(bytes) = dir_size_bytes(&daemon.home_dir.join("data")) {
        ui::kv(&i18n::t("label-data-dir"), &format_bytes(bytes));
    }

    // --- Default model ------------------------------------------------------
    ui::blank();
    // (D) Missing provider key: show the concrete env-var name so the user
    // knows exactly which one to set.
    if key_required && !key_present {
        ui::kv_warn(
            &i18n::t("label-provider"),
            &format!(
                "{}  \u{26A0} {} {}",
                cfg.default_model.provider,
                key_env,
                i18n::t("warn-key-missing"),
            ),
        );
    } else {
        ui::kv(&i18n::t("label-provider"), &cfg.default_model.provider);
    }
    ui::kv(&i18n::t("label-model"), &cfg.default_model.model);

    // --- Health checks (C: always list all, not just degraded) --------------
    if let Some(h) = health.as_ref() {
        if let Some(checks) = h["checks"].as_array() {
            if !checks.is_empty() {
                ui::blank();
                ui::section(&i18n::t("label-checks"));
                for c in checks {
                    let name = c["name"].as_str().unwrap_or("?");
                    let st = c["status"].as_str().unwrap_or("?");
                    if st == "ok" {
                        ui::kv_ok(name, st);
                    } else {
                        ui::kv_warn(name, st);
                    }
                }
            }
        }
    }

    // --- Detail tier --------------------------------------------------------
    match detail.as_ref() {
        Some(body) => render_detail_section(body),
        None => {
            ui::blank();
            ui::section(&i18n::t("section-status-locked"));
            ui::hint(&i18n::t("hint-status-locked"));
        }
    }

    // --- Verbose extras -----------------------------------------------------
    if verbose {
        render_verbose_section(base, &cfg, detail.is_some(), daemon.api_key.as_deref());
    }

    // --- Recent errors (always, if any) -------------------------------------
    let errors = recent_daemon_errors(&daemon.home_dir, 3);
    if !errors.is_empty() {
        ui::blank();
        ui::section(&i18n::t("section-recent-errors"));
        for line in &errors {
            println!("    {}", line.red());
        }
    }

    exit_code
}

/// Map health response to a semantic exit code.
///
/// - `0` — daemon running and `/api/health` reports `ok`.
/// - `2` — daemon running but `/api/health` reports a non-ok status
///   (`degraded`, `error`, anything else the handler introduces later).
/// - `3` — daemon claims to be listening (we got a `/api/health` URL from
///   `daemon.json`) but the request didn't yield parseable JSON — the
///   process is unreachable even though the port is.
fn classify_exit(health: Option<&serde_json::Value>) -> i32 {
    match health.and_then(|h| h["status"].as_str()) {
        Some("ok") => 0,
        Some(_) => 2,
        None => 3,
    }
}

/// Heuristic for "this port is reachable from the internet if the machine
/// has a public IP." Catches the two common foot-guns: `0.0.0.0` (IPv4 any)
/// and `::` / `[::]` (IPv6 any). IPv4 loopback, IPv6 loopback, and named
/// localhost stay quiet.
fn listener_is_public(listen_addr: &str) -> bool {
    let host = listen_addr
        .rsplit_once(':')
        .map(|(h, _)| h.trim_start_matches('[').trim_end_matches(']'))
        .unwrap_or(listen_addr);
    matches!(host, "0.0.0.0" | "::" | "[::]")
}

/// Compute whether the configured default provider has a usable API key in
/// the environment (or in `provider_api_keys` in config.toml). Local
/// providers (ollama/vllm/lmstudio/lemonade) don't need one.
fn provider_key_state(cfg: &librefang_types::config::KernelConfig) -> (String, bool, bool) {
    let provider = cfg.default_model.provider.as_str();
    let key_required = !librefang_runtime::provider_health::is_local_provider(provider);
    let key_env = if cfg.default_model.api_key_env.trim().is_empty() {
        format!("{}_API_KEY", provider.to_uppercase())
    } else {
        cfg.default_model.api_key_env.clone()
    };
    let env_has_key = std::env::var(&key_env)
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);
    let config_has_key = cfg
        .provider_api_keys
        .get(provider)
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);
    (key_env, env_has_key || config_has_key, key_required)
}

fn format_latency(d: std::time::Duration) -> String {
    let ms = d.as_millis();
    if ms < 1 {
        format!("{}µs", d.as_micros())
    } else {
        format!("{ms}ms")
    }
}

/// Recursively sum file sizes under `dir`. Returns `None` if `dir` does not
/// exist or cannot be read. Symlinks are followed because the default data
/// directory may legitimately symlink subdirs onto another disk.
fn dir_size_bytes(dir: &std::path::Path) -> Option<u64> {
    if !dir.exists() {
        return None;
    }
    let mut total: u64 = 0;
    for entry in walkdir::WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if let Ok(md) = entry.metadata() {
            if md.is_file() {
                total = total.saturating_add(md.len());
            }
        }
    }
    Some(total)
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: &[(&str, u64)] = &[
        ("GiB", 1u64 << 30),
        ("MiB", 1u64 << 20),
        ("KiB", 1u64 << 10),
    ];
    for (unit, thresh) in UNITS {
        if bytes >= *thresh {
            return format!("{:.2} {}", bytes as f64 / *thresh as f64, unit);
        }
    }
    format!("{bytes} B")
}

/// Scan the last chunk of `daemon.log` for ERROR-level entries. We read a
/// capped suffix of the file so a multi-GB log doesn't blow up memory, then
/// walk it backwards and collect the most recent N. An empty result means
/// either the log is missing or genuinely has no recent errors — the caller
/// treats both the same way (no section rendered).
fn recent_daemon_errors(home_dir: &std::path::Path, limit: usize) -> Vec<String> {
    use std::io::{Read, Seek, SeekFrom};
    let log = home_dir.join("daemon.log");
    let mut file = match std::fs::File::open(&log) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    const TAIL_BYTES: u64 = 128 * 1024;
    let start = len.saturating_sub(TAIL_BYTES);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return Vec::new();
    }
    let mut buf = String::new();
    if file.read_to_string(&mut buf).is_err() {
        return Vec::new();
    }
    buf.lines()
        .rev()
        // Match the tracing-subscriber default format. ` ERROR ` with padding
        // before and after is specific enough to avoid false positives from
        // log lines that happen to contain the word "error".
        .filter(|line| line.contains(" ERROR ") || line.starts_with("ERROR "))
        .take(limit)
        .map(|l| l.trim_end().to_string())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

/// One-line quiet summary for `librefang status -q`. Stays stable across
/// releases so scripts can parse it — prefix is always `librefang`, second
/// token is a state word, remaining tokens are `key=value`.
fn render_status_quiet_daemon(
    base: &str,
    info: Option<&librefang_api::server::DaemonInfo>,
    health: Option<&serde_json::Value>,
    detail: Option<&serde_json::Value>,
    uptime: Option<u64>,
    exit_code: i32,
) -> i32 {
    let state = match health.and_then(|h| h["status"].as_str()) {
        Some("ok") => "ok",
        Some(other) => other,
        None => "unreachable",
    };
    let version = info
        .map(|i| i.version.as_str())
        .or_else(|| health.and_then(|h| h["version"].as_str()))
        .unwrap_or("?");
    let uptime_s = uptime.map(format_uptime).unwrap_or_else(|| "?".to_string());
    let auth_s = if detail.is_some() {
        let agents = detail.and_then(|d| d["agent_count"].as_u64()).unwrap_or(0);
        format!("agents={agents}")
    } else {
        "locked".to_string()
    };
    println!("librefang {version} {state} uptime={uptime_s} {auth_s} ({base})");
    exit_code
}

/// Extra verbose-only section. Everything in here is best-effort: anything
/// that fails to load just isn't shown — we never stop the main render.
fn render_verbose_section(
    base: &str,
    cfg: &librefang_types::config::KernelConfig,
    authenticated: bool,
    api_key: Option<&str>,
) {
    ui::blank();
    ui::section(&i18n::t("section-verbose"));

    // --- Auth mode ----------------------------------------------------------
    let mut auth_bits: Vec<String> = Vec::new();
    if !cfg.api_key.trim().is_empty() {
        auth_bits.push(i18n::t("auth-api-key"));
    }
    // Dashboard auth / user keys live under [auth] in config. Detect by
    // presence of non-empty dashboard credentials so we don't depend on
    // features that may vary across versions.
    if !cfg.dashboard_pass_hash.trim().is_empty() || !cfg.dashboard_pass.trim().is_empty() {
        auth_bits.push(i18n::t("auth-dashboard-login"));
    }
    let auth_value = if auth_bits.is_empty() {
        i18n::t("auth-none")
    } else {
        auth_bits.join(" + ")
    };
    ui::kv(&i18n::t("label-auth"), &auth_value);

    // --- MCP server count ---------------------------------------------------
    let mcp_count = cfg.mcp_servers.len();
    if mcp_count > 0 {
        ui::kv(&i18n::t("label-mcp"), &mcp_count.to_string());
    }

    // --- OFP peers ----------------------------------------------------------
    // Pass the API key when we have one: `/api/network/status` is in the
    // dashboard-read allowlist, so it transitions from public to
    // auth-required the moment `require_auth_for_reads` kicks in (which
    // happens automatically as soon as *any* auth is configured).
    if let Some((enabled, connected, total)) = fetch_peer_status(base, api_key) {
        if enabled {
            ui::kv(
                &i18n::t("label-peers"),
                &format!("{connected} connected / {total} known"),
            );
        }
    }

    // --- Authenticated counts ----------------------------------------------
    if authenticated {
        if let Some(key) = api_key {
            if let Some(n) = fetch_array_count(base, "/api/channels", key) {
                ui::kv(&i18n::t("label-channels"), &n.to_string());
            }
            if let Some(n) = fetch_array_count(base, "/api/skills", key) {
                ui::kv(&i18n::t("label-skills"), &n.to_string());
            }
            if let Some(n) = fetch_array_count(base, "/api/hands", key) {
                ui::kv(&i18n::t("label-hands"), &n.to_string());
            }
        }
    }

    // --- Config warnings ----------------------------------------------------
    let warnings = cfg.validate();
    if !warnings.is_empty() {
        ui::blank();
        ui::section(&i18n::t("label-config-warnings"));
        for w in warnings {
            ui::check_warn(&w);
        }
    }
}

fn fetch_peer_status(base: &str, api_key: Option<&str>) -> Option<(bool, u64, u64)> {
    let client = daemon_client_with_api_key(api_key);
    let resp = client
        .get(format!("{base}/api/network/status"))
        .send()
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: serde_json::Value = resp.json().ok()?;
    let enabled = body["enabled"].as_bool().unwrap_or(false);
    let connected = body["connected_peers"].as_u64().unwrap_or(0);
    let total = body["total_peers"].as_u64().unwrap_or(0);
    Some((enabled, connected, total))
}

fn fetch_array_count(base: &str, path: &str, api_key: &str) -> Option<u64> {
    let client = daemon_client_with_api_key(Some(api_key));
    let resp = client.get(format!("{base}{path}")).send().ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: serde_json::Value = resp.json().ok()?;
    // Dashboard endpoints vary: agents/skills/hands/channels each shape
    // their list differently. Probe in order: bare array → {total} →
    // common array keys. Whichever matches first wins; if none do we
    // return None so the caller quietly omits the row.
    if let Some(a) = body.as_array() {
        return Some(a.len() as u64);
    }
    if let Some(n) = body["total"].as_u64() {
        return Some(n);
    }
    for key in ["items", "channels", "skills", "hands", "agents"] {
        if let Some(a) = body[key].as_array() {
            return Some(a.len() as u64);
        }
    }
    None
}

fn render_detail_section(body: &serde_json::Value) {
    let total = body["agent_count"].as_u64().unwrap_or(0);
    let active = body["active_agent_count"].as_u64().unwrap_or(0);
    let sessions = body["session_count"].as_u64().unwrap_or(0);
    let memory_mb = body["memory_used_mb"].as_u64();

    ui::blank();
    ui::kv(
        &i18n::t("label-agents"),
        &format!("{active} running / {total} total"),
    );
    ui::kv(&i18n::t("label-sessions"), &sessions.to_string());
    if let Some(mb) = memory_mb {
        ui::kv(&i18n::t("label-memory"), &format!("{mb} MB"));
    }

    if let Some(agents) = body["agents"].as_array() {
        if !agents.is_empty() {
            ui::blank();
            ui::section(&i18n::t("section-active-agents"));
            render_agents_table(agents);
        }
    }
}

/// Render the agent list as a column-aligned table. Empty input is a no-op
/// so the caller can unconditionally call this after a non-empty check.
fn render_agents_table(agents: &[serde_json::Value]) {
    // Compute per-column widths so names and ids line up even when one entry
    // is much longer than the others. Keep a minimum width so a single-row
    // table doesn't look squashed against the header.
    let mut rows: Vec<[String; 4]> = Vec::with_capacity(agents.len());
    for a in agents {
        let name = a["name"].as_str().unwrap_or("?").to_string();
        let id = a["id"].as_str().unwrap_or("?").to_string();
        let state = a["state"].as_str().unwrap_or("?").to_string();
        let model = format!(
            "{}:{}",
            a["model_provider"].as_str().unwrap_or("?"),
            a["model_name"].as_str().unwrap_or("?"),
        );
        rows.push([name, id, state, model]);
    }
    let headers = ["NAME", "ID", "STATE", "MODEL"];
    let mut widths = [0usize; 4];
    for (i, h) in headers.iter().enumerate() {
        widths[i] = h.len();
    }
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.len());
        }
    }
    // Cap ID column at 12 so we don't push the model column off the screen
    // — users rarely need more than a handful of id bytes for correlation.
    widths[1] = widths[1].min(12);
    let id_trim = |s: &str| -> String {
        if s.len() <= widths[1] {
            s.to_string()
        } else {
            s.chars().take(widths[1]).collect()
        }
    };
    let header_line = format!(
        "    {:<w0$}  {:<w1$}  {:<w2$}  {}",
        headers[0],
        headers[1],
        headers[2],
        headers[3],
        w0 = widths[0],
        w1 = widths[1],
        w2 = widths[2],
    );
    println!("{}", header_line.dimmed());
    for row in &rows {
        println!(
            "    {:<w0$}  {:<w1$}  {:<w2$}  {}",
            row[0],
            id_trim(&row[1]),
            row[2],
            row[3],
            w0 = widths[0],
            w1 = widths[1],
            w2 = widths[2],
        );
    }
}

fn render_status_inprocess(config: Option<PathBuf>, json: bool, quiet: bool) -> i32 {
    // Quiet mode short-circuits the kernel boot — we don't need to load 22
    // workflow templates just to print "daemon down". Pull what we can from
    // the config file alone.
    if quiet {
        let cfg = load_config(config.as_deref());
        println!(
            "librefang down home={} default={}/{}",
            cfg.home_dir.display(),
            cfg.default_model.provider,
            cfg.default_model.model,
        );
        return 1;
    }

    let kernel = boot_kernel(config);
    let agent_count = kernel.agent_registry().count();
    let cfg = kernel.config_ref();

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "status": "in-process",
                "agent_count": agent_count,
                "home": cfg.home_dir.display().to_string(),
                "data_dir": cfg.data_dir.display().to_string(),
                "data_dir_bytes": dir_size_bytes(&cfg.data_dir),
                "default_provider": cfg.default_model.provider,
                "default_model": cfg.default_model.model,
                "daemon": false,
                "exit_code": 1,
            }))
            .unwrap_or_default()
        );
        return 1;
    }

    ui::section(&i18n::t("section-status-inprocess"));
    ui::blank();
    ui::kv(&i18n::t("label-agents"), &agent_count.to_string());
    ui::kv(&i18n::t("label-provider"), &cfg.default_model.provider);
    ui::kv(&i18n::t("label-model"), &cfg.default_model.model);
    ui::kv(&i18n::t("label-home"), &cfg.home_dir.display().to_string());
    if let Some(bytes) = dir_size_bytes(&cfg.data_dir) {
        ui::kv(
            &i18n::t("label-data-dir"),
            &format!("{} ({})", cfg.data_dir.display(), format_bytes(bytes)),
        );
    } else {
        ui::kv(
            &i18n::t("label-data-dir"),
            &cfg.data_dir.display().to_string(),
        );
    }
    ui::kv_warn(
        &i18n::t("label-daemon"),
        &i18n::t("label-daemon-not-running"),
    );
    ui::blank();
    ui::hint(&i18n::t("hint-run-start"));

    if agent_count > 0 {
        ui::blank();
        ui::section(&i18n::t("section-persisted-agents"));
        for entry in kernel.agent_registry().list() {
            println!("    {} ({}) -- {:?}", entry.name, entry.id, entry.state);
        }
    }

    1
}

/// Fetch the public `/api/health` payload along with the round-trip time.
/// Returns `(None, None)` on network failure and `(None, Some(_))` when the
/// server responded but the body didn't parse, so the caller can still
/// surface "responded in 42ms but unreadable" if needed.
fn fetch_health_timed(base: &str) -> (Option<serde_json::Value>, Option<std::time::Duration>) {
    let client = daemon_client_with_api_key(None);
    let start = std::time::Instant::now();
    let resp = match client.get(format!("{base}/api/health")).send() {
        Ok(r) => r,
        Err(_) => return (None, None),
    };
    let elapsed = start.elapsed();
    if !resp.status().is_success() {
        return (None, Some(elapsed));
    }
    (resp.json::<serde_json::Value>().ok(), Some(elapsed))
}

/// Fetch the authenticated `/api/status` payload. Returns `None` on any
/// failure — including 401 — so the renderer falls back to the locked
/// section rather than printing `?` for every field.
fn fetch_status_detail(base: &str, api_key: &str) -> Option<serde_json::Value> {
    let client = daemon_client_with_api_key(Some(api_key));
    let resp = client.get(format!("{base}/api/status")).send().ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json::<serde_json::Value>().ok()
}

/// Prefer authoritative uptime from the daemon; fall back to `now - started_at`
/// from `daemon.json` when the detail tier is unavailable.
fn uptime_secs(
    info: Option<&librefang_api::server::DaemonInfo>,
    detail: Option<&serde_json::Value>,
) -> Option<u64> {
    if let Some(body) = detail {
        if let Some(u) = body["uptime_seconds"].as_u64() {
            return Some(u);
        }
    }
    let info = info?;
    let started = chrono::DateTime::parse_from_rfc3339(&info.started_at).ok()?;
    let now = chrono::Utc::now();
    let delta = now.signed_duration_since(started.with_timezone(&chrono::Utc));
    u64::try_from(delta.num_seconds()).ok()
}

fn format_uptime(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else if secs < 86400 {
        format!("{}h {}m {}s", secs / 3600, (secs % 3600) / 60, secs % 60)
    } else {
        format!(
            "{}d {}h {}m",
            secs / 86400,
            (secs % 86400) / 3600,
            (secs % 3600) / 60
        )
    }
}

fn cmd_doctor(json: bool, repair: bool) {
    let mut checks: Vec<serde_json::Value> = Vec::new();
    let mut all_ok = true;
    let mut repaired = false;

    if !json {
        ui::step(&i18n::t("doctor-title"));
        println!();
    }

    let home = dirs::home_dir();
    if let Some(_h) = &home {
        let librefang_dir = cli_librefang_home();

        // --- Check 1: LibreFang directory ---
        if librefang_dir.exists() {
            if !json {
                ui::check_ok(&format!("LibreFang directory: {}", librefang_dir.display()));
            }
            checks.push(serde_json::json!({"check": "librefang_dir", "status": "ok", "path": librefang_dir.display().to_string()}));
        } else if repair {
            if !json {
                ui::check_fail("LibreFang directory not found.");
            }
            let answer = prompt_input("    Create it now? [Y/n] ");
            if answer.is_empty() || answer.starts_with('y') || answer.starts_with('Y') {
                if std::fs::create_dir_all(&librefang_dir).is_ok() {
                    restrict_dir_permissions(&librefang_dir);
                    let _ = std::fs::create_dir_all(librefang_dir.join("data"));
                    let _ =
                        std::fs::create_dir_all(librefang_dir.join("workspaces").join("agents"));
                    if !json {
                        ui::check_ok("Created LibreFang directory");
                    }
                    repaired = true;
                } else {
                    if !json {
                        ui::check_fail("Failed to create directory");
                    }
                    all_ok = false;
                }
            } else {
                all_ok = false;
            }
            checks.push(serde_json::json!({"check": "librefang_dir", "status": if repaired { "repaired" } else { "fail" }}));
        } else {
            if !json {
                ui::check_fail("LibreFang directory not found. Run `librefang init` first.");
            }
            checks.push(serde_json::json!({"check": "librefang_dir", "status": "fail"}));
            all_ok = false;
        }

        // --- Check 2: .env file exists + permissions ---
        let env_path = librefang_dir.join(".env");
        if env_path.exists() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(meta) = std::fs::metadata(&env_path) {
                    let mode = meta.permissions().mode() & 0o777;
                    if mode == 0o600 {
                        if !json {
                            ui::check_ok(".env file (permissions OK)");
                        }
                    } else if repair {
                        let _ = std::fs::set_permissions(
                            &env_path,
                            std::fs::Permissions::from_mode(0o600),
                        );
                        if !json {
                            ui::check_ok(".env file (permissions fixed to 0600)");
                        }
                        repaired = true;
                    } else if !json {
                        ui::check_warn(&format!(
                            ".env file has loose permissions ({:o}), should be 0600",
                            mode
                        ));
                    }
                } else if !json {
                    ui::check_ok(".env file");
                }
            }
            #[cfg(not(unix))]
            {
                if !json {
                    ui::check_ok(".env file");
                }
            }
            checks.push(serde_json::json!({"check": "env_file", "status": "ok"}));
        } else {
            if !json {
                ui::check_warn(
                    ".env file not found (create with: librefang config set-key <provider>)",
                );
            }
            checks.push(serde_json::json!({"check": "env_file", "status": "warn"}));
        }

        // --- Check 3: Config TOML syntax validation ---
        let config_path = librefang_dir.join("config.toml");
        if config_path.exists() {
            let config_content = std::fs::read_to_string(&config_path).unwrap_or_default();
            match toml::from_str::<toml::Value>(&config_content) {
                Ok(_) => {
                    if !json {
                        ui::check_ok(&format!("Config file: {}", config_path.display()));
                    }
                    checks.push(serde_json::json!({"check": "config_file", "status": "ok"}));
                }
                Err(e) => {
                    if !json {
                        ui::check_fail(&format!("Config file has syntax errors: {e}"));
                        ui::hint(&i18n::t("hint-config-edit"));
                    }
                    checks.push(serde_json::json!({"check": "config_syntax", "status": "fail", "error": e.to_string()}));
                    all_ok = false;
                }
            }
        } else if repair {
            if !json {
                ui::check_fail("Config file not found.");
            }
            let answer = prompt_input("    Create default config? [Y/n] ");
            if answer.is_empty() || answer.starts_with('y') || answer.starts_with('Y') {
                let (provider, api_key_env, model) = detect_best_provider();
                let default_config = render_init_default_config(&provider, &model, &api_key_env);
                let _ = std::fs::create_dir_all(&librefang_dir);
                if std::fs::write(&config_path, default_config).is_ok() {
                    restrict_file_permissions(&config_path);
                    if !json {
                        ui::check_ok("Created default config.toml");
                    }
                    repaired = true;
                } else {
                    if !json {
                        ui::check_fail("Failed to create config.toml");
                    }
                    all_ok = false;
                }
            } else {
                all_ok = false;
            }
            checks.push(serde_json::json!({"check": "config_file", "status": if repaired { "repaired" } else { "fail" }}));
        } else {
            if !json {
                ui::check_fail("Config file not found.");
            }
            checks.push(serde_json::json!({"check": "config_file", "status": "fail"}));
            all_ok = false;
        }

        // --- Check: Version update ---
        {
            let current_version = env!("CARGO_PKG_VERSION");
            let update_channel = load_update_channel_from_config().unwrap_or_default();
            if !json {
                ui::check_ok(&format!(
                    "CLI version: {current_version} (channel: {update_channel})"
                ));
            }
            checks.push(serde_json::json!({"check": "cli_version", "status": "ok", "version": current_version, "channel": update_channel.to_string()}));

            // Try to fetch latest release for the configured channel (best-effort)
            match fetch_latest_release_tag(update_channel) {
                Ok(tag) => {
                    let latest = tag.strip_prefix('v').unwrap_or(&tag);
                    if latest != current_version {
                        if !json {
                            ui::check_warn(&format!(
                                "Update available: {current_version} -> {latest} (see https://github.com/librefang/librefang/releases)"
                            ));
                        }
                        checks.push(serde_json::json!({"check": "version_update", "status": "warn", "current": current_version, "latest": latest}));
                    } else {
                        if !json {
                            ui::check_ok("CLI is up to date");
                        }
                        checks.push(serde_json::json!({"check": "version_update", "status": "ok"}));
                    }
                }
                Err(_) => {
                    if !json {
                        ui::check_warn("Could not check for updates (network unavailable)");
                    }
                    checks.push(serde_json::json!({"check": "version_update", "status": "warn", "reason": "network_error"}));
                }
            }
        }

        // --- Check 4: Port availability ---
        // Read api_listen from config (default: 0.0.0.0:4545)
        let api_listen = {
            let cfg_path = librefang_dir.join("config.toml");
            if cfg_path.exists() {
                std::fs::read_to_string(&cfg_path)
                    .ok()
                    .and_then(|s| toml::from_str::<librefang_types::config::KernelConfig>(&s).ok())
                    .map(|c| c.api_listen)
                    .unwrap_or_else(|| librefang_types::config::DEFAULT_API_LISTEN.to_string())
            } else {
                librefang_types::config::DEFAULT_API_LISTEN.to_string()
            }
        };
        if !json {
            println!();
        }
        let daemon_running = find_daemon();
        if let Some(ref base) = daemon_running {
            if !json {
                ui::check_ok(&format!("Daemon running at {base}"));
            }
            checks.push(serde_json::json!({"check": "daemon", "status": "ok", "url": base}));
        } else {
            if !json {
                ui::check_warn("Daemon not running (start with `librefang start`)");
            }
            checks.push(serde_json::json!({"check": "daemon", "status": "warn"}));

            // Check if the configured port is available
            let bind_addr = if api_listen.starts_with("0.0.0.0") {
                api_listen.replacen("0.0.0.0", "127.0.0.1", 1)
            } else {
                api_listen.clone()
            };
            match std::net::TcpListener::bind(&bind_addr) {
                Ok(_) => {
                    if !json {
                        ui::check_ok(&format!("Port {api_listen} is available"));
                    }
                    checks.push(
                        serde_json::json!({"check": "port", "status": "ok", "address": api_listen}),
                    );
                }
                Err(_) => {
                    if !json {
                        ui::check_warn(&format!("Port {api_listen} is in use by another process"));
                    }
                    checks.push(serde_json::json!({"check": "port", "status": "warn", "address": api_listen}));
                }
            }
        }

        // --- Check 5: Stale daemon.json ---
        let daemon_json_path = librefang_dir.join("daemon.json");
        if daemon_json_path.exists() && daemon_running.is_none() {
            if repair {
                let _ = std::fs::remove_file(&daemon_json_path);
                if !json {
                    ui::check_ok("Removed stale daemon.json");
                }
                repaired = true;
            } else if !json {
                ui::check_warn(
                    "Stale daemon.json found (daemon not running). Run with --repair to clean up.",
                );
            }
            checks.push(serde_json::json!({"check": "stale_daemon_json", "status": if repair { "repaired" } else { "warn" }}));
        }

        // --- Check 6: Database file ---
        let db_path = librefang_dir.join("data").join("librefang.db");
        if db_path.exists() {
            // Quick SQLite magic bytes check
            if let Ok(bytes) = std::fs::read(&db_path) {
                if bytes.len() >= 16 && bytes.starts_with(b"SQLite format 3") {
                    if !json {
                        ui::check_ok("Database file (valid SQLite)");
                    }
                    checks.push(serde_json::json!({"check": "database", "status": "ok"}));
                } else {
                    if !json {
                        ui::check_fail("Database file exists but is not valid SQLite");
                    }
                    checks.push(serde_json::json!({"check": "database", "status": "fail"}));
                    all_ok = false;
                }
            }
        } else {
            if !json {
                ui::check_warn("No database file (will be created on first run)");
            }
            checks.push(serde_json::json!({"check": "database", "status": "warn"}));
        }

        // --- Check 7: Disk space ---
        #[cfg(unix)]
        {
            if let Ok(output) = std::process::Command::new("df")
                .args(["-m", &librefang_dir.display().to_string()])
                .output()
            {
                let stdout = String::from_utf8_lossy(&output.stdout);
                // Parse the available MB from df output (4th column of 2nd line)
                if let Some(line) = stdout.lines().nth(1) {
                    let cols: Vec<&str> = line.split_whitespace().collect();
                    if cols.len() >= 4 {
                        if let Ok(available_mb) = cols[3].parse::<u64>() {
                            if available_mb < 100 {
                                if !json {
                                    ui::check_warn(&format!(
                                        "Low disk space: {available_mb}MB available"
                                    ));
                                }
                                checks.push(serde_json::json!({"check": "disk_space", "status": "warn", "available_mb": available_mb}));
                            } else {
                                if !json {
                                    ui::check_ok(&format!(
                                        "Disk space: {available_mb}MB available"
                                    ));
                                }
                                checks.push(serde_json::json!({"check": "disk_space", "status": "ok", "available_mb": available_mb}));
                            }
                        }
                    }
                }
            }
        }

        // --- Check 8: Agent manifests parse correctly ---
        let agents_dir = librefang_dir.join("workspaces").join("agents");
        if agents_dir.exists() {
            let mut agent_errors = Vec::new();
            if let Ok(entries) = std::fs::read_dir(&agents_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            if let Err(e) = toml::from_str::<AgentManifest>(&content) {
                                agent_errors.push((
                                    path.file_name()
                                        .unwrap_or_default()
                                        .to_string_lossy()
                                        .to_string(),
                                    e.to_string(),
                                ));
                            }
                        }
                    }
                }
            }
            if agent_errors.is_empty() {
                if !json {
                    ui::check_ok("Agent manifests are valid");
                }
                checks.push(serde_json::json!({"check": "agent_manifests", "status": "ok"}));
            } else {
                for (file, err) in &agent_errors {
                    if !json {
                        ui::check_fail(&format!("Invalid manifest {file}: {err}"));
                    }
                }
                checks.push(serde_json::json!({"check": "agent_manifests", "status": "fail", "errors": agent_errors.len()}));
                all_ok = false;
            }
        }
    } else {
        if !json {
            ui::check_fail("Could not determine home directory");
        }
        checks.push(serde_json::json!({"check": "home_dir", "status": "fail"}));
        all_ok = false;
    }

    // --- LLM providers ---
    if !json {
        println!("\n  LLM Providers:");
    }
    let provider_keys = [
        ("GROQ_API_KEY", "Groq", "groq"),
        ("OPENROUTER_API_KEY", "OpenRouter", "openrouter"),
        ("ANTHROPIC_API_KEY", "Anthropic", "anthropic"),
        ("OPENAI_API_KEY", "OpenAI", "openai"),
        ("DEEPSEEK_API_KEY", "DeepSeek", "deepseek"),
        ("GEMINI_API_KEY", "Gemini", "gemini"),
        ("GOOGLE_API_KEY", "Google", "google"),
        ("TOGETHER_API_KEY", "Together", "together"),
        ("MISTRAL_API_KEY", "Mistral", "mistral"),
        ("FIREWORKS_API_KEY", "Fireworks", "fireworks"),
    ];

    let mut any_key_set = false;
    for (env_var, name, provider_id) in &provider_keys {
        let set = std::env::var(env_var).is_ok();
        if set {
            // --- Check 9: Live key validation ---
            let valid = test_api_key(provider_id, &std::env::var(env_var).unwrap_or_default());
            if valid {
                if !json {
                    ui::provider_status(name, env_var, true);
                }
            } else if !json {
                ui::check_warn(&format!("{name} ({env_var}) - key rejected (401/403)"));
            }
            any_key_set = true;
            checks.push(serde_json::json!({"check": "provider", "name": name, "env_var": env_var, "status": if valid { "ok" } else { "warn" }, "live_test": !valid}));
        } else {
            if !json {
                ui::provider_status(name, env_var, false);
            }
            checks.push(serde_json::json!({"check": "provider", "name": name, "env_var": env_var, "status": "warn"}));
        }
    }

    if !any_key_set {
        if !json {
            println!();
            ui::check_fail(&i18n::t("doctor-no-api-keys"));
            ui::blank();
            ui::section(&i18n::t("section-getting-api-key"));
            ui::suggest_cmd("Groq:", "https://console.groq.com       (free, fast)");
            ui::suggest_cmd("Gemini:", "https://aistudio.google.com    (free tier)");
            ui::suggest_cmd("DeepSeek:", "https://platform.deepseek.com  (low cost)");
            ui::blank();
            ui::hint(&i18n::t("hint-set-key"));
        }
        all_ok = false;
    }

    // --- Check: Network connectivity to configured LLM provider endpoints ---
    {
        let provider_endpoints: &[(&str, &str, &str)] = &[
            ("OPENAI_API_KEY", "OpenAI", "api.openai.com:443"),
            ("ANTHROPIC_API_KEY", "Anthropic", "api.anthropic.com:443"),
            ("GROQ_API_KEY", "Groq", "api.groq.com:443"),
            ("DEEPSEEK_API_KEY", "DeepSeek", "api.deepseek.com:443"),
            (
                "GEMINI_API_KEY",
                "Gemini",
                "generativelanguage.googleapis.com:443",
            ),
            (
                "GOOGLE_API_KEY",
                "Google",
                "generativelanguage.googleapis.com:443",
            ),
            ("OPENROUTER_API_KEY", "OpenRouter", "openrouter.ai:443"),
            ("TOGETHER_API_KEY", "Together", "api.together.xyz:443"),
            ("MISTRAL_API_KEY", "Mistral", "api.mistral.ai:443"),
            ("FIREWORKS_API_KEY", "Fireworks", "api.fireworks.ai:443"),
        ];

        let configured: Vec<_> = provider_endpoints
            .iter()
            .filter(|(env_var, _, _)| std::env::var(env_var).is_ok())
            .collect();

        if !configured.is_empty() {
            if !json {
                println!("\n  Network Connectivity:");
            }
            for (env_var, name, endpoint) in &configured {
                use std::net::{TcpStream, ToSocketAddrs};
                let reachable = endpoint
                    .to_socket_addrs()
                    .ok()
                    .and_then(|mut addrs| addrs.next())
                    .map(|addr| {
                        TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(3)).is_ok()
                    })
                    .unwrap_or(false);

                if reachable {
                    if !json {
                        ui::check_ok(&format!("{name} endpoint reachable ({endpoint})"));
                    }
                    checks.push(serde_json::json!({"check": "network_connectivity", "provider": name, "endpoint": endpoint, "env_var": env_var, "status": "ok"}));
                } else {
                    if !json {
                        ui::check_warn(&format!("{name} endpoint unreachable ({endpoint})"));
                    }
                    checks.push(serde_json::json!({"check": "network_connectivity", "provider": name, "endpoint": endpoint, "env_var": env_var, "status": "warn"}));
                }
            }
        }
    }

    // --- Check 10: Channel token format validation ---
    if !json {
        println!("\n  Channel Integrations:");
    }
    let channel_keys = [
        ("TELEGRAM_BOT_TOKEN", "Telegram"),
        ("DISCORD_BOT_TOKEN", "Discord"),
        ("SLACK_APP_TOKEN", "Slack App"),
        ("SLACK_BOT_TOKEN", "Slack Bot"),
    ];
    for (env_var, name) in &channel_keys {
        let set = std::env::var(env_var).is_ok();
        if set {
            // Format validation
            let val = std::env::var(env_var).unwrap_or_default();
            let format_ok = match *env_var {
                "TELEGRAM_BOT_TOKEN" => val.contains(':'), // Telegram tokens have format "123456:ABC-DEF..."
                "DISCORD_BOT_TOKEN" => val.len() > 50,     // Discord tokens are typically 59+ chars
                "SLACK_APP_TOKEN" => val.starts_with("xapp-"),
                "SLACK_BOT_TOKEN" => val.starts_with("xoxb-"),
                _ => true,
            };
            if format_ok {
                if !json {
                    ui::provider_status(name, env_var, true);
                }
            } else if !json {
                ui::check_warn(&format!("{name} ({env_var}) - unexpected token format"));
            }
            checks.push(serde_json::json!({"check": "channel", "name": name, "env_var": env_var, "status": if format_ok { "ok" } else { "warn" }}));
        } else {
            if !json {
                ui::provider_status(name, env_var, false);
            }
            checks.push(serde_json::json!({"check": "channel", "name": name, "env_var": env_var, "status": "warn"}));
        }
    }

    // --- Check 11: .env keys vs config api_key_env consistency ---
    {
        let librefang_dir = cli_librefang_home();
        let config_path = librefang_dir.join("config.toml");
        if config_path.exists() {
            let config_str = std::fs::read_to_string(&config_path).unwrap_or_default();
            // Look for api_key_env references in config
            for line in config_str.lines() {
                let trimmed = line.trim();
                if let Some(rest) = trimmed.strip_prefix("api_key_env") {
                    if let Some(val_part) = rest.strip_prefix('=') {
                        let val = val_part.trim().trim_matches('"');
                        if !val.is_empty() && std::env::var(val).is_err() {
                            if !json {
                                ui::check_warn(&format!(
                                    "Config references {val} but it is not set in env or .env"
                                ));
                            }
                            checks.push(serde_json::json!({"check": "env_consistency", "status": "warn", "missing_var": val}));
                        }
                    }
                }
            }
        }
    }

    // --- Check 12: Config deserialization into KernelConfig ---
    {
        let librefang_dir = cli_librefang_home();
        let config_path = librefang_dir.join("config.toml");
        if config_path.exists() {
            if !json {
                println!("\n  Config Validation:");
            }
            let config_content = std::fs::read_to_string(&config_path).unwrap_or_default();
            match toml::from_str::<librefang_types::config::KernelConfig>(&config_content) {
                Ok(cfg) => {
                    if !json {
                        ui::check_ok("Config deserializes into KernelConfig");
                    }
                    checks.push(serde_json::json!({"check": "config_deser", "status": "ok"}));

                    // Check exec policy
                    let mode = format!("{:?}", cfg.exec_policy.mode);
                    let safe_bins_count = cfg.exec_policy.safe_bins.len();
                    if !json {
                        ui::check_ok(&format!(
                            "Exec policy: mode={mode}, safe_bins={safe_bins_count}"
                        ));
                    }
                    checks.push(serde_json::json!({"check": "exec_policy", "status": "ok", "mode": mode, "safe_bins": safe_bins_count}));

                    // Check includes
                    if !cfg.include.is_empty() {
                        let mut include_ok = true;
                        for inc in &cfg.include {
                            let inc_path = librefang_dir.join(inc);
                            if inc_path.exists() {
                                if !json {
                                    ui::check_ok(&format!("Include file: {inc}"));
                                }
                            } else if repair {
                                if !json {
                                    ui::check_warn(&format!("Include file missing: {inc}"));
                                }
                                include_ok = false;
                            } else {
                                if !json {
                                    ui::check_fail(&format!("Include file not found: {inc}"));
                                }
                                include_ok = false;
                                all_ok = false;
                            }
                        }
                        checks.push(serde_json::json!({"check": "config_includes", "status": if include_ok { "ok" } else { "fail" }, "count": cfg.include.len()}));
                    }

                    // Check MCP server configs
                    if !cfg.mcp_servers.is_empty() {
                        let mcp_count = cfg.mcp_servers.len();
                        if !json {
                            ui::check_ok(&format!("MCP servers configured: {mcp_count}"));
                        }
                        for server in &cfg.mcp_servers {
                            // Validate transport config
                            let Some(ref transport) = server.transport else {
                                continue;
                            };
                            match transport {
                                librefang_types::config::McpTransportEntry::Stdio {
                                    command,
                                    ..
                                } => {
                                    if command.is_empty() {
                                        if !json {
                                            ui::check_warn(&format!(
                                                "MCP server '{}' has empty command",
                                                server.name
                                            ));
                                        }
                                        checks.push(serde_json::json!({"check": "mcp_server_config", "status": "warn", "name": server.name}));
                                    }
                                }
                                librefang_types::config::McpTransportEntry::Sse { url }
                                | librefang_types::config::McpTransportEntry::Http { url } => {
                                    if url.is_empty() {
                                        if !json {
                                            ui::check_warn(&format!(
                                                "MCP server '{}' has empty URL",
                                                server.name
                                            ));
                                        }
                                        checks.push(serde_json::json!({"check": "mcp_server_config", "status": "warn", "name": server.name}));
                                    }
                                }
                                librefang_types::config::McpTransportEntry::HttpCompat {
                                    base_url,
                                    headers,
                                    tools,
                                } => {
                                    if base_url.is_empty() {
                                        if !json {
                                            ui::check_warn(&format!(
                                                "MCP server '{}' has empty base_url",
                                                server.name
                                            ));
                                        }
                                        checks.push(serde_json::json!({"check": "mcp_server_config", "status": "warn", "name": server.name}));
                                    }
                                    if tools.is_empty() {
                                        if !json {
                                            ui::check_warn(&format!(
                                                "MCP server '{}' has no http_compat tools configured",
                                                server.name
                                            ));
                                        }
                                        checks.push(serde_json::json!({"check": "mcp_server_config", "status": "warn", "name": server.name}));
                                    }
                                    if headers.iter().any(|h| h.name.trim().is_empty()) {
                                        if !json {
                                            ui::check_warn(&format!(
                                                "MCP server '{}' has an http_compat header with empty name",
                                                server.name
                                            ));
                                        }
                                        checks.push(serde_json::json!({"check": "mcp_server_config", "status": "warn", "name": server.name}));
                                    }
                                    if headers.iter().any(|h| {
                                        h.value.as_ref().is_none_or(|value| value.trim().is_empty())
                                            && h.value_env
                                                .as_ref()
                                                .is_none_or(|value| value.trim().is_empty())
                                    }) {
                                        if !json {
                                            ui::check_warn(&format!(
                                                "MCP server '{}' has an http_compat header without value/value_env",
                                                server.name
                                            ));
                                        }
                                        checks.push(serde_json::json!({"check": "mcp_server_config", "status": "warn", "name": server.name}));
                                    }
                                    if tools.iter().any(|tool| tool.name.trim().is_empty()) {
                                        if !json {
                                            ui::check_warn(&format!(
                                                "MCP server '{}' has an http_compat tool with empty name",
                                                server.name
                                            ));
                                        }
                                        checks.push(serde_json::json!({"check": "mcp_server_config", "status": "warn", "name": server.name}));
                                    }
                                    if tools.iter().any(|tool| tool.path.trim().is_empty()) {
                                        if !json {
                                            ui::check_warn(&format!(
                                                "MCP server '{}' has an http_compat tool with empty path",
                                                server.name
                                            ));
                                        }
                                        checks.push(serde_json::json!({"check": "mcp_server_config", "status": "warn", "name": server.name}));
                                    }
                                }
                            }
                        }
                        checks.push(serde_json::json!({"check": "mcp_servers", "status": "ok", "count": mcp_count}));
                    }
                }
                Err(e) => {
                    if !json {
                        ui::check_fail(&format!("Config fails KernelConfig deserialization: {e}"));
                    }
                    checks.push(serde_json::json!({"check": "config_deser", "status": "fail", "error": e.to_string()}));
                    all_ok = false;
                }
            }
        }
    }

    // --- Check 13: Skill registry health ---
    {
        if !json {
            println!("\n  Skills:");
        }
        let skills_dir = cli_librefang_home().join("skills");
        let mut skill_reg = librefang_skills::registry::SkillRegistry::new(skills_dir.clone());
        match skill_reg.load_all() {
            Ok(count) => {
                if !json {
                    ui::check_ok(&format!("Skills loaded: {count}"));
                }
                checks.push(serde_json::json!({"check": "skills", "status": "ok", "count": count}));
            }
            Err(e) => {
                if !json {
                    ui::check_warn(&format!("Failed to load skills: {e}"));
                }
                checks.push(serde_json::json!({"check": "skills", "status": "warn", "error": e.to_string()}));
            }
        }

        // Check for prompt injection issues in skill definitions.
        // Only flag Critical-severity warnings.
        let skills = skill_reg.list();
        let mut injection_warnings = 0;
        for skill in &skills {
            if let Some(ref prompt) = skill.manifest.prompt_context {
                let warnings = librefang_skills::verify::SkillVerifier::scan_prompt_content(prompt);
                let has_critical = warnings.iter().any(|w| {
                    matches!(
                        w.severity,
                        librefang_skills::verify::WarningSeverity::Critical
                    )
                });
                if has_critical {
                    injection_warnings += 1;
                    if !json {
                        ui::check_warn(&format!(
                            "Prompt injection warning in skill: {}",
                            skill.manifest.skill.name
                        ));
                    }
                }
            }
        }
        if injection_warnings > 0 {
            checks.push(serde_json::json!({"check": "skill_injection_scan", "status": "warn", "warnings": injection_warnings}));
        } else {
            if !json {
                ui::check_ok("All skills pass prompt injection scan");
            }
            checks.push(serde_json::json!({"check": "skill_injection_scan", "status": "ok"}));
        }
    }

    // --- Check 14: MCP catalog + configured servers ---
    {
        if !json {
            println!("\n  MCP servers:");
        }
        let librefang_dir = cli_librefang_home();
        let mut catalog = librefang_extensions::catalog::McpCatalog::new(&librefang_dir);
        catalog.load(&librefang_runtime::registry_sync::resolve_home_dir_for_tests());
        let template_count = catalog.len();

        // Count configured [[mcp_servers]] entries in config.toml (if any).
        let configured_count = {
            let config_path = librefang_dir.join("config.toml");
            if config_path.is_file() {
                let raw = std::fs::read_to_string(&config_path).unwrap_or_default();
                toml::from_str::<toml::Value>(&raw)
                    .ok()
                    .and_then(|v| v.as_table().cloned())
                    .and_then(|t| t.get("mcp_servers").cloned())
                    .and_then(|v| v.as_array().cloned())
                    .map(|a| a.len())
                    .unwrap_or(0)
            } else {
                0
            }
        };
        if !json {
            ui::check_ok(&format!("MCP catalog templates: {template_count}"));
            ui::check_ok(&format!("Configured MCP servers: {configured_count}"));
        }
        checks.push(
            serde_json::json!({"check": "mcp_catalog", "status": "ok", "count": template_count}),
        );
        checks.push(serde_json::json!({"check": "mcp_servers_configured", "status": "ok", "count": configured_count}));
    }

    // --- Check 15: Daemon health detail (if running) ---
    if let Some(ref base) = find_daemon() {
        if !json {
            println!("\n  Daemon Health:");
        }
        let client = daemon_client();
        match client.get(format!("{base}/api/health/detail")).send() {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    if let Some(agents) = body.get("agent_count").and_then(|v| v.as_u64()) {
                        if !json {
                            ui::check_ok(&format!("Running agents: {agents}"));
                        }
                        checks.push(serde_json::json!({"check": "daemon_agents", "status": "ok", "count": agents}));
                    }
                    if let Some(uptime) = body.get("uptime_secs").and_then(|v| v.as_u64()) {
                        let hours = uptime / 3600;
                        let mins = (uptime % 3600) / 60;
                        if !json {
                            ui::check_ok(&format!("Daemon uptime: {hours}h {mins}m"));
                        }
                        checks.push(serde_json::json!({"check": "daemon_uptime", "status": "ok", "secs": uptime}));
                    }
                    if let Some(db_status) = body.get("database").and_then(|v| v.as_str()) {
                        if db_status == "connected" || db_status == "ok" {
                            if !json {
                                ui::check_ok("Database connectivity: OK");
                            }
                        } else {
                            if !json {
                                ui::check_fail(&format!("Database status: {db_status}"));
                            }
                            all_ok = false;
                        }
                        checks.push(serde_json::json!({"check": "daemon_db", "status": db_status}));
                    }
                }
            }
            Ok(resp) => {
                if !json {
                    ui::check_warn(&format!("Health detail returned {}", resp.status()));
                }
                checks.push(serde_json::json!({"check": "daemon_health", "status": "warn"}));
            }
            Err(e) => {
                if !json {
                    ui::check_warn(&format!("Failed to query daemon health: {e}"));
                }
                checks.push(serde_json::json!({"check": "daemon_health", "status": "warn", "error": e.to_string()}));
            }
        }

        // Check skills endpoint
        match client.get(format!("{base}/api/skills")).send() {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    if let Some(arr) = body
                        .get("skills")
                        .and_then(|v| v.as_array())
                        .or_else(|| body.as_array())
                    {
                        if !json {
                            ui::check_ok(&format!("Skills loaded in daemon: {}", arr.len()));
                        }
                        checks.push(serde_json::json!({"check": "daemon_skills", "status": "ok", "count": arr.len()}));
                    }
                }
            }
            _ => {}
        }

        // Check MCP servers endpoint
        match client.get(format!("{base}/api/mcp/servers")).send() {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    if let Some(arr) = body
                        .get("configured")
                        .and_then(|v| v.as_array())
                        .or_else(|| body.as_array())
                    {
                        let connected = arr
                            .iter()
                            .filter(|s| {
                                s.get("connected")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(false)
                            })
                            .count();
                        if !json {
                            ui::check_ok(&format!(
                                "MCP servers: {} configured, {} connected",
                                arr.len(),
                                connected
                            ));
                        }
                        checks.push(serde_json::json!({"check": "daemon_mcp", "status": "ok", "configured": arr.len(), "connected": connected}));
                    }
                }
            }
            _ => {}
        }

        // Check MCP health endpoint
        match client.get(format!("{base}/api/mcp/health")).send() {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let entries = body.get("health").and_then(|h| h.as_array());
                    if let Some(arr) = entries {
                        let healthy = arr
                            .iter()
                            .filter(|v| {
                                v.get("status")
                                    .and_then(|s| s.as_str())
                                    .map(|s| s.eq_ignore_ascii_case("ready"))
                                    .unwrap_or(false)
                            })
                            .count();
                        let total = arr.len();
                        if healthy == total {
                            if !json {
                                ui::check_ok(&format!(
                                    "MCP server health: {healthy}/{total} healthy"
                                ));
                            }
                        } else if !json {
                            ui::check_warn(&format!(
                                "MCP server health: {healthy}/{total} healthy"
                            ));
                        }
                        checks.push(serde_json::json!({"check": "mcp_health", "status": if healthy == total { "ok" } else { "warn" }, "healthy": healthy, "total": total}));
                    }
                }
            }
            _ => {}
        }
    }

    if !json {
        println!();
    }
    match std::process::Command::new("rustc")
        .arg("--version")
        .output()
    {
        Ok(output) => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !json {
                ui::check_ok(&format!("Rust: {version}"));
            }
            checks.push(serde_json::json!({"check": "rust", "status": "ok", "version": version}));
        }
        Err(_) => {
            if !json {
                ui::check_fail("Rust toolchain not found");
            }
            checks.push(serde_json::json!({"check": "rust", "status": "fail"}));
            all_ok = false;
        }
    }

    // Python runtime check
    match std::process::Command::new("python3")
        .arg("--version")
        .output()
    {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !json {
                ui::check_ok(&format!("Python: {version}"));
            }
            checks.push(serde_json::json!({"check": "python", "status": "ok", "version": version}));
        }
        _ => {
            // Try `python` instead
            match std::process::Command::new("python")
                .arg("--version")
                .output()
            {
                Ok(output) if output.status.success() => {
                    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !json {
                        ui::check_ok(&format!("Python: {version}"));
                    }
                    checks.push(
                        serde_json::json!({"check": "python", "status": "ok", "version": version}),
                    );
                }
                _ => {
                    if !json {
                        ui::check_warn("Python not found (needed for Python skill runtime)");
                    }
                    checks.push(serde_json::json!({"check": "python", "status": "warn"}));
                }
            }
        }
    }

    // Node.js runtime check
    match std::process::Command::new("node").arg("--version").output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !json {
                ui::check_ok(&format!("Node.js: {version}"));
            }
            checks.push(serde_json::json!({"check": "node", "status": "ok", "version": version}));
        }
        _ => {
            if !json {
                ui::check_warn("Node.js not found (needed for Node skill runtime)");
            }
            checks.push(serde_json::json!({"check": "node", "status": "warn"}));
        }
    }

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "all_ok": all_ok,
                "checks": checks,
            }))
            .unwrap_or_default()
        );
    } else {
        println!();
        if all_ok {
            ui::success(&i18n::t("doctor-all-passed"));
            ui::hint(&i18n::t("hint-start-daemon-cmd"));
        } else if repaired {
            ui::success(&i18n::t("doctor-repairs-applied"));
        } else {
            ui::error(&i18n::t("doctor-some-failed"));
            if !repair {
                ui::hint(&i18n::t("hint-doctor-repair"));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Dashboard command
// ---------------------------------------------------------------------------

fn cmd_dashboard() {
    let base = if let Some(url) = find_daemon() {
        url
    } else {
        // Auto-start the daemon
        ui::hint(&i18n::t("daemon-no-running-auto"));
        match start_daemon_background() {
            Ok(url) => {
                ui::success(&i18n::t("daemon-started"));
                url
            }
            Err(e) => {
                ui::error_with_fix(
                    &i18n::t_args("daemon-start-fail", &[("error", &e.to_string())]),
                    &i18n::t("daemon-start-fail-fix"),
                );
                std::process::exit(1);
            }
        }
    };

    let url = format!("{base}/");
    ui::success(&i18n::t_args("dashboard-opening", &[("url", &url)]));
    if copy_to_clipboard(&url) {
        ui::hint(&i18n::t("hint-url-copied"));
    }
    if !open_in_browser(&url) {
        ui::hint(&i18n::t_args(
            "hint-could-not-open-browser-visit",
            &[("url", &url)],
        ));
    }
}

/// Copy text to the system clipboard. Returns true on success.
fn copy_to_clipboard(text: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        // Use PowerShell to set clipboard (handles special characters better than cmd)
        std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!("Set-Clipboard '{}'", text.replace('\'', "''")),
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(target_os = "macos")]
    {
        use std::io::Write as IoWrite;
        std::process::Command::new("pbcopy")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                if let Some(ref mut stdin) = child.stdin {
                    let _ = stdin.write_all(text.as_bytes());
                }
                child.wait()
            })
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(target_os = "linux")]
    {
        use std::io::Write as IoWrite;
        // Try xclip first, then xsel
        let result = std::process::Command::new("xclip")
            .args(["-selection", "clipboard"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                if let Some(ref mut stdin) = child.stdin {
                    let _ = stdin.write_all(text.as_bytes());
                }
                child.wait()
            })
            .map(|s| s.success())
            .unwrap_or(false);
        if result {
            return true;
        }
        std::process::Command::new("xsel")
            .args(["--clipboard", "--input"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                if let Some(ref mut stdin) = child.stdin {
                    let _ = stdin.write_all(text.as_bytes());
                }
                child.wait()
            })
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = text;
        false
    }
}

/// Try to open a URL in the default browser. Returns true on success.
pub(crate) fn open_in_browser(url: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
            .is_ok()
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).spawn().is_ok()
    }
    #[cfg(target_os = "linux")]
    {
        // Try multiple openers in order. xdg-open is the standard, but it
        // (or the browser it launches) can fail with EPERM in sandboxed
        // environments (containers, Snap, Flatpak, user-namespace
        // restrictions). Fall through to alternatives if any opener fails.
        let openers = [
            "xdg-open",
            "sensible-browser",
            "x-www-browser",
            "firefox",
            "google-chrome",
            "chromium",
            "chromium-browser",
        ];
        for opener in &openers {
            let result = std::process::Command::new(opener)
                .arg(url)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
            if result.is_ok() {
                return true;
            }
        }
        false
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = url;
        false
    }
}

// ---------------------------------------------------------------------------
// Shell completion command
// ---------------------------------------------------------------------------

fn cmd_completion(shell: clap_complete::Shell) {
    use clap::CommandFactory;
    let mut cmd = Cli::command();
    clap_complete::generate(shell, &mut cmd, "librefang", &mut std::io::stdout());
}

// ---------------------------------------------------------------------------
// Workflow commands
// ---------------------------------------------------------------------------

fn cmd_workflow_list() {
    let base = require_daemon("workflow list");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/workflows")).send());

    match body.as_array() {
        Some(workflows) if workflows.is_empty() => println!("No workflows registered."),
        Some(workflows) => {
            println!("{:<38} {:<20} {:<6} CREATED", "ID", "NAME", "STEPS");
            println!("{}", "-".repeat(80));
            for w in workflows {
                println!(
                    "{:<38} {:<20} {:<6} {}",
                    w["id"].as_str().unwrap_or("?"),
                    w["name"].as_str().unwrap_or("?"),
                    w["steps"].as_u64().unwrap_or(0),
                    w["created_at"].as_str().unwrap_or("?"),
                );
            }
        }
        None => println!("No workflows registered."),
    }
}

fn cmd_workflow_create(file: PathBuf) {
    let base = require_daemon("workflow create");
    if !file.exists() {
        eprintln!("Workflow file not found: {}", file.display());
        std::process::exit(1);
    }
    let contents = std::fs::read_to_string(&file).unwrap_or_else(|e| {
        eprintln!("Error reading workflow file: {e}");
        std::process::exit(1);
    });
    let json_body: serde_json::Value = serde_json::from_str(&contents).unwrap_or_else(|e| {
        eprintln!("Invalid JSON: {e}");
        std::process::exit(1);
    });

    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/workflows"))
            .json(&json_body)
            .send(),
    );

    if let Some(id) = body["workflow_id"].as_str() {
        println!("Workflow created successfully!");
        println!("  ID: {id}");
    } else {
        eprintln!(
            "Failed to create workflow: {}",
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }
}

fn cmd_workflow_run(workflow_id: &str, input: &str) {
    let base = require_daemon("workflow run");
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/workflows/{workflow_id}/run"))
            .json(&serde_json::json!({"input": input}))
            .send(),
    );

    if let Some(output) = body["output"].as_str() {
        println!("Workflow completed!");
        println!("  Run ID: {}", body["run_id"].as_str().unwrap_or("?"));
        println!("  Output:\n{output}");
    } else {
        eprintln!(
            "Workflow failed: {}",
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// Trigger commands
// ---------------------------------------------------------------------------

fn cmd_trigger_list(agent_id: Option<&str>) {
    let base = require_daemon("trigger list");
    let client = daemon_client();

    let url = match agent_id {
        Some(id) => format!("{base}/api/triggers?agent_id={id}"),
        None => format!("{base}/api/triggers"),
    };
    let body = daemon_json(client.get(&url).send());

    let arr = body["triggers"].as_array().or_else(|| body.as_array());
    match arr {
        Some(triggers) if triggers.is_empty() => println!("No triggers registered."),
        Some(triggers) => {
            println!(
                "{:<38} {:<38} {:<8} {:<6} PATTERN",
                "TRIGGER ID", "AGENT ID", "ENABLED", "FIRES"
            );
            println!("{}", "-".repeat(110));
            for t in triggers {
                println!(
                    "{:<38} {:<38} {:<8} {:<6} {}",
                    t["id"].as_str().unwrap_or("?"),
                    t["agent_id"].as_str().unwrap_or("?"),
                    t["enabled"].as_bool().unwrap_or(false),
                    t["fire_count"].as_u64().unwrap_or(0),
                    t["pattern"],
                );
            }
        }
        None => println!("No triggers registered."),
    }
}

fn cmd_trigger_create(
    agent_id: &str,
    pattern_json: &str,
    prompt: &str,
    max_fires: u64,
    target_agent: Option<&str>,
    cooldown: Option<u64>,
    session_mode: Option<&str>,
) {
    let base = require_daemon("trigger create");
    let agent_id = resolve_agent_id(&base, agent_id);
    let pattern: serde_json::Value = serde_json::from_str(pattern_json).unwrap_or_else(|e| {
        eprintln!("Invalid pattern JSON: {e}");
        eprintln!("Examples:");
        eprintln!("  '\"lifecycle\"'");
        eprintln!("  '{{\"agent_spawned\":{{\"name_pattern\":\"*\"}}}}'");
        eprintln!("  '\"agent_terminated\"'");
        eprintln!("  '\"all\"'");
        std::process::exit(1);
    });

    let mut payload = serde_json::json!({
        "agent_id": agent_id,
        "pattern": pattern,
        "prompt_template": prompt,
        "max_fires": max_fires,
    });
    if let Some(t) = target_agent {
        payload["target_agent_id"] = serde_json::json!(t);
    }
    if let Some(c) = cooldown {
        payload["cooldown_secs"] = serde_json::json!(c);
    }
    if let Some(m) = session_mode {
        payload["session_mode"] = serde_json::json!(m);
    }

    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/triggers"))
            .json(&payload)
            .send(),
    );

    if let Some(id) = body["trigger_id"].as_str() {
        println!("Trigger created successfully!");
        println!("  Trigger ID: {id}");
        println!("  Agent ID:   {agent_id}");
        if let Some(t) = target_agent {
            println!("  Target:     {t}");
        }
    } else {
        eprintln!(
            "Failed to create trigger: {}",
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }
}

fn cmd_trigger_delete(trigger_id: &str) {
    let base = require_daemon("trigger delete");
    let client = daemon_client();
    let body = daemon_json(
        client
            .delete(format!("{base}/api/triggers/{trigger_id}"))
            .send(),
    );

    if body.get("status").is_some() {
        println!("Trigger {trigger_id} deleted.");
    } else {
        eprintln!(
            "Failed to delete trigger: {}",
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }
}

fn cmd_trigger_get(trigger_id: &str) {
    let base = require_daemon("trigger get");
    let client = daemon_client();
    let body = daemon_json(
        client
            .get(format!("{base}/api/triggers/{trigger_id}"))
            .send(),
    );

    if body.get("error").is_some() {
        eprintln!(
            "Failed to get trigger: {}",
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }

    println!("Trigger ID:    {}", body["id"].as_str().unwrap_or("-"));
    println!(
        "Agent ID:      {}",
        body["agent_id"].as_str().unwrap_or("-")
    );
    println!("Pattern:       {}", body["pattern"]);
    println!(
        "Prompt:        {}",
        body["prompt_template"].as_str().unwrap_or("-")
    );
    println!(
        "Enabled:       {}",
        body["enabled"].as_bool().unwrap_or(false)
    );
    println!(
        "Fire count:    {}",
        body["fire_count"].as_u64().unwrap_or(0)
    );
    println!(
        "Max fires:     {}",
        body["max_fires"]
            .as_u64()
            .map(|n| n.to_string())
            .unwrap_or_else(|| "unlimited".to_string())
    );
    if let Some(t) = body["target_agent_id"].as_str() {
        println!("Target agent:  {t}");
    }
    if let Some(c) = body["cooldown_secs"].as_u64() {
        println!("Cooldown:      {c}s");
    }
    if let Some(m) = body["session_mode"].as_str() {
        println!("Session mode:  {m}");
    }
}

#[allow(clippy::too_many_arguments)]
fn cmd_trigger_update(
    trigger_id: &str,
    pattern: Option<&str>,
    prompt: Option<&str>,
    enabled: Option<bool>,
    max_fires: Option<u64>,
    cooldown: Option<u64>,
    clear_cooldown: bool,
    session_mode: Option<&str>,
    clear_session_mode: bool,
    target_agent: Option<&str>,
    clear_target_agent: bool,
) {
    let base = require_daemon("trigger update");
    let client = daemon_client();

    let mut payload = serde_json::json!({});
    if let Some(p) = pattern {
        let parsed: serde_json::Value = serde_json::from_str(p).unwrap_or_else(|e| {
            eprintln!("Invalid pattern JSON: {e}");
            std::process::exit(1);
        });
        payload["pattern"] = parsed;
    }
    if let Some(t) = prompt {
        payload["prompt_template"] = serde_json::json!(t);
    }
    if let Some(e) = enabled {
        payload["enabled"] = serde_json::json!(e);
    }
    if let Some(m) = max_fires {
        payload["max_fires"] = serde_json::json!(m);
    }
    if clear_cooldown {
        payload["cooldown_secs"] = serde_json::Value::Null;
    } else if let Some(c) = cooldown {
        payload["cooldown_secs"] = serde_json::json!(c);
    }
    if clear_session_mode {
        payload["session_mode"] = serde_json::Value::Null;
    } else if let Some(m) = session_mode {
        payload["session_mode"] = serde_json::json!(m);
    }
    if clear_target_agent {
        payload["target_agent_id"] = serde_json::Value::Null;
    } else if let Some(a) = target_agent {
        payload["target_agent_id"] = serde_json::json!(a);
    }

    let body = daemon_json(
        client
            .patch(format!("{base}/api/triggers/{trigger_id}"))
            .json(&payload)
            .send(),
    );

    if body.get("error").is_some() {
        eprintln!(
            "Failed to update trigger: {}",
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }
    println!("Trigger {trigger_id} updated.");
}

fn cmd_trigger_set_enabled(trigger_id: &str, enabled: bool) {
    let base = require_daemon(if enabled {
        "trigger enable"
    } else {
        "trigger disable"
    });
    let client = daemon_client();
    let payload = serde_json::json!({ "enabled": enabled });
    let body = daemon_json(
        client
            .patch(format!("{base}/api/triggers/{trigger_id}"))
            .json(&payload)
            .send(),
    );

    if body.get("error").is_some() {
        eprintln!(
            "Failed to {} trigger: {}",
            if enabled { "enable" } else { "disable" },
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }
    println!(
        "Trigger {trigger_id} {}.",
        if enabled { "enabled" } else { "disabled" }
    );
}

/// Require a running daemon — exit with helpful message if not found.
fn require_daemon(command: &str) -> String {
    find_daemon().unwrap_or_else(|| {
        ui::error_with_fix(
            &i18n::t_args("error-require-daemon", &[("command", command)]),
            &i18n::t("error-require-daemon-fix"),
        );
        ui::hint(&i18n::t("hint-or-chat"));
        std::process::exit(1);
    })
}

fn boot_kernel(config: Option<PathBuf>) -> LibreFangKernel {
    match LibreFangKernel::boot(config.as_deref()) {
        Ok(k) => k,
        Err(e) => {
            boot_kernel_error(&e);
            std::process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Migrate command
// ---------------------------------------------------------------------------

fn cmd_migrate(args: MigrateArgs) {
    let source = match args.from {
        MigrateSourceArg::Openclaw => librefang_migrate::MigrateSource::OpenClaw,
        MigrateSourceArg::Langchain => librefang_migrate::MigrateSource::LangChain,
        MigrateSourceArg::Autogpt => librefang_migrate::MigrateSource::AutoGpt,
        MigrateSourceArg::Openfang => librefang_migrate::MigrateSource::OpenFang,
    };

    let source_dir = args.source_dir.unwrap_or_else(|| {
        let home = dirs::home_dir().unwrap_or_else(|| {
            eprintln!("Error: Could not determine home directory");
            std::process::exit(1);
        });
        match source {
            librefang_migrate::MigrateSource::OpenClaw => home.join(".openclaw"),
            librefang_migrate::MigrateSource::LangChain => home.join(".langchain"),
            librefang_migrate::MigrateSource::AutoGpt => home.join("Auto-GPT"),
            librefang_migrate::MigrateSource::OpenFang => home.join(".openfang"),
        }
    });

    let target_dir = cli_librefang_home();

    println!("Migrating from {} ({})...", source, source_dir.display());
    if args.dry_run {
        println!("  (dry run — no changes will be made)\n");
    }

    let options = librefang_migrate::MigrateOptions {
        source,
        source_dir,
        target_dir,
        dry_run: args.dry_run,
    };

    match librefang_migrate::run_migration(&options) {
        Ok(report) => {
            report.print_summary();

            // Save migration report
            if !args.dry_run {
                let report_path = options.target_dir.join("migration_report.md");
                if let Err(e) = std::fs::write(&report_path, report.to_markdown()) {
                    eprintln!("Warning: Could not save migration report: {e}");
                } else {
                    println!("\n  Report saved to: {}", report_path.display());
                }
            }
        }
        Err(e) => {
            eprintln!("Migration failed: {e}");
            std::process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Skill commands
// ---------------------------------------------------------------------------

/// Resolve the skills directory: global or per-hand workspace.
fn resolve_skills_dir(hand: Option<&str>) -> PathBuf {
    let home = librefang_home();
    match hand {
        None => home.join("skills"),
        Some(hand_id) => {
            let hand_dir = home.join("workspaces").join("hands").join(hand_id);
            if !hand_dir.exists() {
                eprintln!("Hand '{hand_id}' not found at {}", hand_dir.display());
                std::process::exit(1);
            }
            hand_dir.join("skills")
        }
    }
}

fn cmd_skill_install(source: &str, hand: Option<&str>) {
    let skills_dir = resolve_skills_dir(hand);
    std::fs::create_dir_all(&skills_dir).unwrap_or_else(|e| {
        eprintln!("Error creating skills directory: {e}");
        std::process::exit(1);
    });

    let source_path = PathBuf::from(source);
    if source_path.exists() && source_path.is_dir() {
        // Local directory install
        let manifest_path = source_path.join("skill.toml");
        if !manifest_path.exists() {
            // Check if it's an OpenClaw skill
            if librefang_skills::openclaw_compat::detect_openclaw_skill(&source_path) {
                println!("Detected OpenClaw skill format. Converting...");
                match librefang_skills::openclaw_compat::convert_openclaw_skill(&source_path) {
                    Ok(manifest) => {
                        let dest = skills_dir.join(&manifest.skill.name);
                        // Copy skill directory
                        copy_dir_recursive(&source_path, &dest);
                        if let Err(e) = librefang_skills::openclaw_compat::write_librefang_manifest(
                            &dest, &manifest,
                        ) {
                            eprintln!("Failed to write manifest: {e}");
                            std::process::exit(1);
                        }
                        if let Some(h) = hand {
                            println!(
                                "Installed OpenClaw skill '{}' to hand '{h}'",
                                manifest.skill.name
                            );
                        } else {
                            println!("Installed OpenClaw skill: {}", manifest.skill.name);
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to convert OpenClaw skill: {e}");
                        std::process::exit(1);
                    }
                }
                return;
            }
            eprintln!("No skill.toml found in {source}");
            std::process::exit(1);
        }

        // Read manifest to get skill name
        let toml_str = std::fs::read_to_string(&manifest_path).unwrap_or_else(|e| {
            eprintln!("Error reading skill.toml: {e}");
            std::process::exit(1);
        });
        let manifest: librefang_skills::SkillManifest =
            toml::from_str(&toml_str).unwrap_or_else(|e| {
                eprintln!("Error parsing skill.toml: {e}");
                std::process::exit(1);
            });

        let dest = skills_dir.join(&manifest.skill.name);
        copy_dir_recursive(&source_path, &dest);
        if let Some(h) = hand {
            println!(
                "Installed skill '{}' v{} to hand '{h}'",
                manifest.skill.name, manifest.skill.version
            );
        } else {
            println!(
                "Installed skill: {} v{}",
                manifest.skill.name, manifest.skill.version
            );
        }
    } else {
        // Remote install from FangHub
        println!("Installing {source} from FangHub...");
        let rt = tokio::runtime::Runtime::new().unwrap();
        let client = librefang_skills::marketplace::MarketplaceClient::new(
            librefang_skills::marketplace::MarketplaceConfig::default(),
        );
        match rt.block_on(client.install(source, &skills_dir)) {
            Ok(version) => {
                if let Some(h) = hand {
                    println!("Installed {source} {version} to hand '{h}'");
                } else {
                    println!("Installed {source} {version}");
                }
            }
            Err(e) => {
                eprintln!("Failed to install skill: {e}");
                std::process::exit(1);
            }
        }
    }
}

fn cmd_skill_list(hand: Option<&str>) {
    let skills_dir = resolve_skills_dir(hand);

    let mut registry = librefang_skills::registry::SkillRegistry::new(skills_dir);
    match registry.load_all() {
        Ok(0) => {
            if let Some(h) = hand {
                println!("No skills installed for hand '{h}'.");
            } else {
                println!("No skills installed.");
            }
        }
        Ok(count) => {
            if let Some(h) = hand {
                println!("{count} skill(s) installed for hand '{h}':\n");
            } else {
                println!("{count} skill(s) installed:\n");
            }
            println!(
                "{:<20} {:<10} {:<8} DESCRIPTION",
                "NAME", "VERSION", "TOOLS"
            );
            println!("{}", "-".repeat(70));
            for skill in registry.list() {
                println!(
                    "{:<20} {:<10} {:<8} {}",
                    skill.manifest.skill.name,
                    skill.manifest.skill.version,
                    skill.manifest.tools.provided.len(),
                    skill.manifest.skill.description,
                );
            }
        }
        Err(e) => {
            eprintln!("Error loading skills: {e}");
            std::process::exit(1);
        }
    }
}

fn cmd_skill_remove(name: &str, hand: Option<&str>) {
    // Route through the safe uninstall path (lock + path-traversal
    // guard) instead of `registry.remove()` which calls `remove_dir_all`
    // with no serialisation against concurrent evolve operations.
    let skills_dir = resolve_skills_dir(hand);
    match librefang_skills::evolution::uninstall_skill(&skills_dir, name) {
        Ok(_) => {
            if let Some(h) = hand {
                println!("Removed skill '{name}' from hand '{h}'");
            } else {
                println!("Removed skill: {name}");
            }
        }
        Err(e) => {
            eprintln!("Failed to remove skill: {e}");
            std::process::exit(1);
        }
    }
}

fn cmd_skill_search(query: &str) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let client = librefang_skills::marketplace::MarketplaceClient::new(
        librefang_skills::marketplace::MarketplaceConfig::default(),
    );
    match rt.block_on(client.search(query)) {
        Ok(results) if results.is_empty() => println!("No skills found for \"{query}\"."),
        Ok(results) => {
            println!("Skills matching \"{query}\":\n");
            for r in results {
                println!("  {} ({})", r.name, r.stars);
                if !r.description.is_empty() {
                    println!("    {}", r.description);
                }
                println!("    {}", r.url);
                println!();
            }
        }
        Err(e) => {
            eprintln!("Search failed: {e}");
            std::process::exit(1);
        }
    }
}

fn cmd_skill_test(path: Option<PathBuf>, tool: Option<String>, input: Option<String>) {
    let skill_path = resolve_skill_path(path);
    let prepared =
        librefang_skills::publish::prepare_local_skill(&skill_path).unwrap_or_else(|e| {
            eprintln!("Skill validation failed: {e}");
            std::process::exit(1);
        });

    println!(
        "Validated skill: {} v{}",
        prepared.manifest.skill.name, prepared.manifest.skill.version
    );
    println!(
        "  Runtime: {:?}\n  Source: {}",
        prepared.manifest.runtime.runtime_type,
        prepared.source_dir.display()
    );
    if !prepared.manifest.skill.description.is_empty() {
        println!("  Description: {}", prepared.manifest.skill.description);
    }
    if !prepared.manifest.tools.provided.is_empty() {
        println!(
            "  Tools: {}",
            prepared
                .manifest
                .tools
                .provided
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    print_skill_warnings(&prepared.warnings);

    if prepared.has_critical_warnings() {
        eprintln!("Refusing to execute a skill with critical validation warnings.");
        std::process::exit(1);
    }

    let Some(tool_name) = tool.or_else(|| {
        prepared
            .manifest
            .tools
            .provided
            .first()
            .map(|tool| tool.name.clone())
    }) else {
        println!("Validation only: no tool declared to execute.");
        return;
    };

    let input_json = match input {
        Some(input) => serde_json::from_str::<serde_json::Value>(&input).unwrap_or_else(|err| {
            eprintln!("Invalid --input JSON: {err}");
            std::process::exit(1);
        }),
        None => serde_json::json!({}),
    };

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(librefang_skills::loader::execute_skill_tool(
        &prepared.manifest,
        &prepared.source_dir,
        &tool_name,
        &input_json,
    ));
    match result {
        Ok(result) => {
            println!("\nTool result ({tool_name}):");
            println!(
                "{}",
                serde_json::to_string_pretty(&result.output).unwrap_or_default()
            );
            if result.is_error {
                std::process::exit(1);
            }
        }
        Err(librefang_skills::SkillError::RuntimeNotAvailable(message)) => {
            println!("\nValidation complete.");
            println!("Execution skipped: {message}");
        }
        Err(err) => {
            eprintln!("Skill execution failed: {err}");
            std::process::exit(1);
        }
    }
}

fn cmd_skill_publish(
    path: Option<PathBuf>,
    repo: Option<String>,
    tag: Option<String>,
    output: Option<PathBuf>,
    dry_run: bool,
) {
    let skill_path = resolve_skill_path(path);
    let prepared =
        librefang_skills::publish::prepare_local_skill(&skill_path).unwrap_or_else(|e| {
            eprintln!("Skill validation failed: {e}");
            std::process::exit(1);
        });

    println!(
        "Preparing skill: {} v{}",
        prepared.manifest.skill.name, prepared.manifest.skill.version
    );
    print_skill_warnings(&prepared.warnings);
    if prepared.has_critical_warnings() {
        eprintln!("Refusing to publish a skill with critical validation warnings.");
        std::process::exit(1);
    }

    let output_dir = output.unwrap_or_else(|| prepared.source_dir.join("dist"));
    let packaged = librefang_skills::publish::package_prepared_skill(&prepared, &output_dir)
        .unwrap_or_else(|e| {
            eprintln!("Failed to package skill: {e}");
            std::process::exit(1);
        });

    println!(
        "Bundle created: {}\n  SHA256: {}\n  Size: {} bytes",
        packaged.archive_path.display(),
        packaged.sha256,
        packaged.size_bytes
    );

    let repo = repo.unwrap_or_else(|| format!("librefang-skills/{}", packaged.manifest.skill.name));
    let tag = tag.unwrap_or_else(|| format!("v{}", packaged.manifest.skill.version));

    if dry_run {
        println!("Dry run only.");
        println!("  Repo: {repo}\n  Tag: {tag}");
        return;
    }

    let token = std::env::var("GITHUB_TOKEN")
        .or_else(|_| std::env::var("GH_TOKEN"))
        .unwrap_or_else(|_| {
            eprintln!("Set GITHUB_TOKEN or GH_TOKEN to publish, or re-run with --dry-run.");
            std::process::exit(1);
        });

    let release_notes = format!(
        "{}\n\nSHA256: `{}`\n\nInstall with:\n`librefang skill install {}`",
        packaged.manifest.skill.description, packaged.sha256, packaged.manifest.skill.name
    );
    let release_name = format!(
        "{} {}",
        packaged.manifest.skill.name, packaged.manifest.skill.version
    );

    let rt = tokio::runtime::Runtime::new().unwrap();
    let client = librefang_skills::marketplace::MarketplaceClient::new(
        librefang_skills::marketplace::MarketplaceConfig::default(),
    );
    let published = rt
        .block_on(
            client.publish_bundle(librefang_skills::marketplace::MarketplacePublishRequest {
                repo: &repo,
                tag: &tag,
                bundle_path: &packaged.archive_path,
                release_name: &release_name,
                release_notes: &release_notes,
                token: &token,
            }),
        )
        .unwrap_or_else(|e| {
            eprintln!("Publish failed: {e}");
            std::process::exit(1);
        });

    println!(
        "Published {} to {}@{}",
        published.asset_name, published.repo, published.tag
    );
    if !published.html_url.is_empty() {
        println!("Release: {}", published.html_url);
    }
}

fn resolve_skill_path(path: Option<PathBuf>) -> PathBuf {
    path.unwrap_or_else(|| {
        std::env::current_dir().unwrap_or_else(|e| {
            eprintln!("Could not determine current directory: {e}");
            std::process::exit(1);
        })
    })
}

fn print_skill_warnings(warnings: &[librefang_skills::verify::SkillWarning]) {
    if warnings.is_empty() {
        println!("  Warnings: none");
        return;
    }

    println!("  Warnings:");
    for warning in warnings {
        println!(
            "    [{}] {}",
            severity_label(warning.severity),
            warning.message
        );
    }
}

fn severity_label(severity: librefang_skills::verify::WarningSeverity) -> &'static str {
    match severity {
        librefang_skills::verify::WarningSeverity::Info => "info",
        librefang_skills::verify::WarningSeverity::Warning => "warn",
        librefang_skills::verify::WarningSeverity::Critical => "critical",
    }
}

fn cmd_skill_create() {
    let name = prompt_input("Skill name: ");
    let description = prompt_input("Description: ");
    let runtime = prompt_input("Runtime (python/node/wasm) [python]: ");
    let runtime = if runtime.is_empty() {
        "python".to_string()
    } else {
        runtime
    };

    let home = librefang_home();
    let skill_dir = home.join("skills").join(&name);
    std::fs::create_dir_all(skill_dir.join("src")).unwrap_or_else(|e| {
        eprintln!("Error creating skill directory: {e}");
        std::process::exit(1);
    });

    let manifest = format!(
        r#"[skill]
name = "{name}"
version = "{version}"
description = "{description}"
author = ""
license = "MIT"
tags = []

[runtime]
type = "{runtime}"
entry = "src/main.py"

[[tools.provided]]
name = "{tool_name}"
description = "{description}"
input_schema = {{ type = "object", properties = {{ input = {{ type = "string" }} }}, required = ["input"] }}

[requirements]
tools = []
capabilities = []
"#,
        version = librefang_types::VERSION,
        tool_name = name.replace('-', "_"),
    );

    std::fs::write(skill_dir.join("skill.toml"), &manifest).unwrap();

    // Create entry point
    let entry_content = match runtime.as_str() {
        "python" => format!(
            r#"#!/usr/bin/env python3
"""LibreFang skill: {name}"""
import json
import sys

def main():
    payload = json.loads(sys.stdin.read())
    tool_name = payload["tool"]
    input_data = payload["input"]

    # TODO: Implement your skill logic here
    result = {{"result": f"Processed: {{input_data.get('input', '')}}"}}

    print(json.dumps(result))

if __name__ == "__main__":
    main()
"#
        ),
        _ => "// TODO: Implement your skill\n".to_string(),
    };

    let entry_path = if runtime == "python" {
        "src/main.py"
    } else {
        "src/index.js"
    };
    std::fs::write(skill_dir.join(entry_path), entry_content).unwrap();

    println!("\nSkill created: {}", skill_dir.display());
    println!("\nFiles:");
    println!("  skill.toml");
    println!("  {entry_path}");
    println!("\nNext steps:");
    println!("  1. Edit the entry point to implement your skill logic");
    println!(
        "  2. Test locally: librefang skill test {}",
        skill_dir.display()
    );
    println!(
        "  3. Install: librefang skill install {}",
        skill_dir.display()
    );
}

// ---------------------------------------------------------------------------
// Skill evolve commands — thin CLI wrappers over librefang_skills::evolution
// ---------------------------------------------------------------------------

/// Read a file path, or stdin if path is "-".
fn read_file_or_stdin(path: &std::path::Path) -> std::io::Result<String> {
    if path == std::path::Path::new("-") {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        Ok(buf)
    } else {
        std::fs::read_to_string(path)
    }
}

/// Print an EvolutionResult as a one-line status.
fn print_evolution_result(result: &librefang_skills::evolution::EvolutionResult) {
    let marker = if result.success { "OK" } else { "FAIL" };
    match &result.version {
        Some(v) => println!("[{marker}] {} (v{v})", result.message),
        None => println!("[{marker}] {}", result.message),
    }
}

/// Resolve a skill by name. Respects `--hand` so evolve operations can
/// target a per-hand workspace skills dir just like `install`/`list`.
fn load_installed_skill(
    name: &str,
    hand: Option<&str>,
) -> (PathBuf, librefang_skills::InstalledSkill) {
    let skills_dir = resolve_skills_dir(hand);
    let mut registry = librefang_skills::registry::SkillRegistry::new(skills_dir.clone());
    if let Err(e) = registry.load_all() {
        eprintln!("Error loading skill registry: {e}");
        std::process::exit(1);
    }
    match registry.get(name) {
        Some(skill) => (skills_dir, skill.clone()),
        None => {
            eprintln!("Skill '{name}' not found in {}", skills_dir.display());
            std::process::exit(1);
        }
    }
}

fn cmd_skill_evolve(sub: EvolveCommands) {
    match sub {
        EvolveCommands::Create {
            name,
            description,
            context_file,
            tags,
            hand,
        } => {
            let prompt_context = match read_file_or_stdin(&context_file) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to read {}: {e}", context_file.display());
                    std::process::exit(1);
                }
            };
            let tag_list: Vec<String> = tags
                .split(',')
                .map(|t| t.trim())
                .filter(|t| !t.is_empty())
                .map(String::from)
                .collect();
            let skills_dir = resolve_skills_dir(hand.as_deref());
            if let Err(e) = std::fs::create_dir_all(&skills_dir) {
                eprintln!("Failed to create skills dir: {e}");
                std::process::exit(1);
            }
            match librefang_skills::evolution::create_skill(
                &skills_dir,
                &name,
                &description,
                &prompt_context,
                tag_list,
                Some("cli"),
            ) {
                Ok(r) => print_evolution_result(&r),
                Err(e) => {
                    eprintln!("Create failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        EvolveCommands::Update {
            name,
            context_file,
            changelog,
            hand,
        } => {
            let new_ctx = match read_file_or_stdin(&context_file) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to read {}: {e}", context_file.display());
                    std::process::exit(1);
                }
            };
            let (_, skill) = load_installed_skill(&name, hand.as_deref());
            match librefang_skills::evolution::update_skill(
                &skill,
                &new_ctx,
                &changelog,
                Some("cli"),
            ) {
                Ok(r) => print_evolution_result(&r),
                Err(e) => {
                    eprintln!("Update failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        EvolveCommands::Patch {
            name,
            old_file,
            new_file,
            changelog,
            replace_all,
            hand,
        } => {
            let old_str = match read_file_or_stdin(&old_file) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to read {}: {e}", old_file.display());
                    std::process::exit(1);
                }
            };
            let new_str = match read_file_or_stdin(&new_file) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to read {}: {e}", new_file.display());
                    std::process::exit(1);
                }
            };
            let (_, skill) = load_installed_skill(&name, hand.as_deref());
            match librefang_skills::evolution::patch_skill(
                &skill,
                &old_str,
                &new_str,
                &changelog,
                replace_all,
                Some("cli"),
            ) {
                Ok(r) => print_evolution_result(&r),
                Err(e) => {
                    eprintln!("Patch failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        EvolveCommands::Delete { name, hand } => {
            let skills_dir = resolve_skills_dir(hand.as_deref());
            match librefang_skills::evolution::delete_skill(&skills_dir, &name) {
                Ok(r) => print_evolution_result(&r),
                Err(e) => {
                    eprintln!("Delete failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        EvolveCommands::Rollback { name, hand } => {
            let (_, skill) = load_installed_skill(&name, hand.as_deref());
            match librefang_skills::evolution::rollback_skill(&skill, Some("cli")) {
                Ok(r) => print_evolution_result(&r),
                Err(e) => {
                    eprintln!("Rollback failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        EvolveCommands::WriteFile {
            name,
            path,
            source,
            hand,
        } => {
            let content = match read_file_or_stdin(&source) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to read {}: {e}", source.display());
                    std::process::exit(1);
                }
            };
            let (_, skill) = load_installed_skill(&name, hand.as_deref());
            match librefang_skills::evolution::write_supporting_file(&skill, &path, &content) {
                Ok(r) => print_evolution_result(&r),
                Err(e) => {
                    eprintln!("Write-file failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        EvolveCommands::RemoveFile { name, path, hand } => {
            let (_, skill) = load_installed_skill(&name, hand.as_deref());
            match librefang_skills::evolution::remove_supporting_file(&skill, &path) {
                Ok(r) => print_evolution_result(&r),
                Err(e) => {
                    eprintln!("Remove-file failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        EvolveCommands::History { name, json, hand } => {
            let (_, skill) = load_installed_skill(&name, hand.as_deref());
            let meta = librefang_skills::evolution::get_evolution_info(&skill);
            if json {
                match serde_json::to_string_pretty(&meta) {
                    Ok(s) => println!("{s}"),
                    Err(e) => {
                        eprintln!("Failed to serialize history: {e}");
                        std::process::exit(1);
                    }
                }
                return;
            }
            println!("Skill: {}", skill.manifest.skill.name);
            println!("Current version: {}", skill.manifest.skill.version);
            println!("Use count: {}", meta.use_count);
            println!("Evolution count: {}", meta.evolution_count);
            if meta.versions.is_empty() {
                println!("\nNo version history recorded.");
                return;
            }
            println!("\n{:<10} {:<25} CHANGELOG", "VERSION", "TIMESTAMP");
            println!("{}", "-".repeat(70));
            for v in meta.versions.iter().rev() {
                println!("{:<10} {:<25} {}", v.version, v.timestamp, v.changelog);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Channel commands
// ---------------------------------------------------------------------------

fn cmd_channel_list() {
    let home = librefang_home();
    let config_path = home.join("config.toml");

    if !config_path.exists() {
        println!("No configuration found. Run `librefang init` first.");
        return;
    }

    let config_str = std::fs::read_to_string(&config_path).unwrap_or_default();

    println!("Channel Integrations:\n");
    println!("{:<12} {:<10} STATUS", "CHANNEL", "ENV VAR");
    println!("{}", "-".repeat(50));

    let channels: Vec<(&str, &str)> = vec![
        ("webchat", ""),
        ("telegram", "TELEGRAM_BOT_TOKEN"),
        ("discord", "DISCORD_BOT_TOKEN"),
        ("slack", "SLACK_BOT_TOKEN"),
        ("whatsapp", "WA_ACCESS_TOKEN"),
        ("signal", ""),
        ("matrix", "MATRIX_TOKEN"),
        ("email", "EMAIL_PASSWORD"),
    ];

    for (name, env_var) in channels {
        let configured = config_str.contains(&format!("[channels.{name}]"));
        let env_set = if env_var.is_empty() {
            true
        } else {
            std::env::var(env_var).is_ok()
        };

        let status = match (configured, env_set) {
            (true, true) => "Ready",
            (true, false) => "Missing env",
            (false, _) => "Not configured",
        };

        println!(
            "{:<12} {:<10} {}",
            name,
            if env_var.is_empty() { "—" } else { env_var },
            status,
        );
    }

    println!("\nUse `librefang channel setup <channel>` to configure a channel.");
}

fn cmd_channel_setup(channel: Option<&str>) {
    let channel = match channel {
        Some(c) => c.to_string(),
        None => {
            // Interactive channel picker
            ui::section(&i18n::t("section-channel-setup"));
            ui::blank();
            let channel_list = [
                ("telegram", "Telegram bot (BotFather)"),
                ("discord", "Discord bot"),
                ("slack", "Slack app (Socket Mode)"),
                ("whatsapp", "WhatsApp Cloud API"),
                ("email", "Email (IMAP/SMTP)"),
                ("signal", "Signal (signal-cli)"),
                ("matrix", "Matrix homeserver"),
            ];

            for (i, (name, desc)) in channel_list.iter().enumerate() {
                println!("    {:>2}. {:<12} {}", i + 1, name, desc.dimmed());
            }
            ui::blank();

            let choice = prompt_input("  Choose channel [1]: ");
            let idx = if choice.is_empty() {
                0
            } else {
                choice
                    .parse::<usize>()
                    .unwrap_or(1)
                    .saturating_sub(1)
                    .min(channel_list.len() - 1)
            };
            channel_list[idx].0.to_string()
        }
    };

    match channel.as_str() {
        "telegram" => {
            ui::section(&i18n::t("section-setup-telegram"));
            ui::blank();
            println!("  1. Open Telegram and message @BotFather");
            println!("  2. Send /newbot and follow the prompts");
            println!("  3. Copy the bot token");
            ui::blank();

            let token = prompt_input("  Paste your bot token: ");
            if token.is_empty() {
                ui::error(&i18n::t("channel-no-token"));
                return;
            }

            let config_block = "\n[channels.telegram]\nbot_token_env = \"TELEGRAM_BOT_TOKEN\"\ndefault_agent = \"assistant\"\n";
            maybe_write_channel_config("telegram", config_block);

            // Save token to .env
            match dotenv::save_env_key("TELEGRAM_BOT_TOKEN", &token) {
                Ok(()) => ui::success(&i18n::t("channel-token-saved")),
                Err(_) => println!("    export TELEGRAM_BOT_TOKEN={token}"),
            }

            ui::blank();
            ui::success(&i18n::t_args("channel-configured", &[("name", "Telegram")]));
            notify_daemon_restart();
        }
        "discord" => {
            ui::section(&i18n::t("section-setup-discord"));
            ui::blank();
            println!("  1. Go to https://discord.com/developers/applications");
            println!("  2. Create a New Application");
            println!("  3. Go to Bot section and click 'Add Bot'");
            println!("  4. Copy the bot token");
            println!("  5. Under Privileged Gateway Intents, enable:");
            println!("     - Message Content Intent");
            println!("  6. Use OAuth2 URL Generator to invite bot to your server");
            ui::blank();

            let token = prompt_input("  Paste your bot token: ");
            if token.is_empty() {
                ui::error(&i18n::t("channel-no-token"));
                return;
            }

            let config_block = "\n[channels.discord]\nbot_token_env = \"DISCORD_BOT_TOKEN\"\ndefault_agent = \"coder\"\n";
            maybe_write_channel_config("discord", config_block);

            match dotenv::save_env_key("DISCORD_BOT_TOKEN", &token) {
                Ok(()) => ui::success(&i18n::t("channel-token-saved")),
                Err(_) => println!("    export DISCORD_BOT_TOKEN={token}"),
            }

            ui::blank();
            ui::success(&i18n::t_args("channel-configured", &[("name", "Discord")]));
            notify_daemon_restart();
        }
        "slack" => {
            ui::section(&i18n::t("section-setup-slack"));
            ui::blank();
            println!("  1. Go to https://api.slack.com/apps");
            println!("  2. Create New App -> From Scratch");
            println!("  3. Enable Socket Mode (Settings -> Socket Mode)");
            println!("  4. Copy the App-Level Token (xapp-...)");
            println!("  5. Go to OAuth & Permissions, add scopes:");
            println!("     - chat:write, app_mentions:read, im:history");
            println!("  6. Install to workspace and copy Bot Token (xoxb-...)");
            ui::blank();

            let app_token = prompt_input("  Paste your App Token (xapp-...): ");
            let bot_token = prompt_input("  Paste your Bot Token (xoxb-...): ");

            let config_block = "\n[channels.slack]\napp_token_env = \"SLACK_APP_TOKEN\"\nbot_token_env = \"SLACK_BOT_TOKEN\"\ndefault_agent = \"assistant\"\n";
            maybe_write_channel_config("slack", config_block);

            if !app_token.is_empty() {
                match dotenv::save_env_key("SLACK_APP_TOKEN", &app_token) {
                    Ok(()) => ui::success(&i18n::t("channel-app-token-saved")),
                    Err(_) => println!("    export SLACK_APP_TOKEN={app_token}"),
                }
            }
            if !bot_token.is_empty() {
                match dotenv::save_env_key("SLACK_BOT_TOKEN", &bot_token) {
                    Ok(()) => ui::success(&i18n::t("channel-bot-token-saved")),
                    Err(_) => println!("    export SLACK_BOT_TOKEN={bot_token}"),
                }
            }

            ui::blank();
            ui::success(&i18n::t_args("channel-configured", &[("name", "Slack")]));
            notify_daemon_restart();
        }
        "whatsapp" => {
            ui::section(&i18n::t("section-setup-whatsapp"));
            ui::blank();
            println!("  WhatsApp Cloud API (recommended for production):");
            println!("  1. Go to https://developers.facebook.com");
            println!("  2. Create a Business App");
            println!("  3. Add WhatsApp product");
            println!("  4. Set up a test phone number");
            println!("  5. Copy Phone Number ID and Access Token");
            ui::blank();

            let phone_id = prompt_input("  Phone Number ID: ");
            let access_token = prompt_input("  Access Token: ");
            let verify_token = prompt_input("  Verify Token: ");

            let config_block = "\n[channels.whatsapp]\nmode = \"cloud_api\"\nphone_number_id_env = \"WA_PHONE_ID\"\naccess_token_env = \"WA_ACCESS_TOKEN\"\nverify_token_env = \"WA_VERIFY_TOKEN\"\nwebhook_port = 8443\ndefault_agent = \"assistant\"\n";
            maybe_write_channel_config("whatsapp", config_block);

            for (key, val) in [
                ("WA_PHONE_ID", &phone_id),
                ("WA_ACCESS_TOKEN", &access_token),
                ("WA_VERIFY_TOKEN", &verify_token),
            ] {
                if !val.is_empty() {
                    match dotenv::save_env_key(key, val) {
                        Ok(()) => ui::success(&i18n::t_args("channel-key-saved", &[("key", key)])),
                        Err(_) => println!("    export {key}={val}"),
                    }
                }
            }

            ui::blank();
            ui::success(&i18n::t_args("channel-configured", &[("name", "WhatsApp")]));
            notify_daemon_restart();
        }
        "email" => {
            ui::section(&i18n::t("section-setup-email"));
            ui::blank();
            println!("  For Gmail, use an App Password:");
            println!("  https://myaccount.google.com/apppasswords");
            ui::blank();

            let username = prompt_input("  Email address: ");
            if username.is_empty() {
                ui::error(&i18n::t("channel-no-email"));
                return;
            }

            let password = prompt_input("  App password (or Enter to set later): ");

            let config_block = format!(
                "\n[channels.email]\nimap_host = \"imap.gmail.com\"\nimap_port = 993\nsmtp_host = \"smtp.gmail.com\"\nsmtp_port = 587\nusername = \"{username}\"\npassword_env = \"EMAIL_PASSWORD\"\npoll_interval = 30\ndefault_agent = \"assistant\"\n"
            );
            maybe_write_channel_config("email", &config_block);

            if !password.is_empty() {
                match dotenv::save_env_key("EMAIL_PASSWORD", &password) {
                    Ok(()) => ui::success(&i18n::t("channel-password-saved")),
                    Err(_) => println!("    export EMAIL_PASSWORD=your_app_password"),
                }
            } else {
                ui::hint(&i18n::t("hint-set-key-provider"));
            }

            ui::blank();
            ui::success(&i18n::t_args("channel-configured", &[("name", "Email")]));
            notify_daemon_restart();
        }
        "signal" => {
            ui::section(&i18n::t("section-setup-signal"));
            ui::blank();
            println!("  Signal requires signal-cli (https://github.com/AsamK/signal-cli).");
            ui::blank();
            println!("  1. Install signal-cli:");
            println!("     - macOS: brew install signal-cli");
            println!("     - Linux: download from GitHub releases");
            println!("     - Or use the Docker image");
            println!("  2. Register or link a phone number:");
            println!("     signal-cli -u +1YOURPHONE register");
            println!("     signal-cli -u +1YOURPHONE verify CODE");
            println!("  3. Start signal-cli in JSON-RPC mode:");
            println!("     signal-cli -u +1YOURPHONE jsonRpc --socket /tmp/signal-cli.sock");
            ui::blank();

            let phone = prompt_input("  Your phone number (+1XXXX, or Enter to skip): ");

            let config_block = "\n[channels.signal]\nphone_env = \"SIGNAL_PHONE\"\nsocket_path = \"/tmp/signal-cli.sock\"\ndefault_agent = \"assistant\"\n";
            maybe_write_channel_config("signal", config_block);

            if !phone.is_empty() {
                match dotenv::save_env_key("SIGNAL_PHONE", &phone) {
                    Ok(()) => ui::success(&i18n::t("channel-phone-saved")),
                    Err(_) => println!("    export SIGNAL_PHONE={phone}"),
                }
            }

            ui::blank();
            ui::success(&i18n::t_args("channel-configured", &[("name", "Signal")]));
            notify_daemon_restart();
        }
        "matrix" => {
            ui::section(&i18n::t("section-setup-matrix"));
            ui::blank();
            println!("  1. Create a bot account on your Matrix homeserver");
            println!("     (e.g., register @librefang-bot:matrix.org)");
            println!("  2. Obtain an access token:");
            println!("     curl -X POST https://matrix.org/_matrix/client/r0/login \\");
            println!("       -d '{{\"type\":\"m.login.password\",\"user\":\"librefang-bot\",\"password\":\"...\"}}'");
            println!("     Copy the access_token from the response.");
            println!("  3. Invite the bot to rooms you want it to monitor.");
            ui::blank();

            let homeserver = prompt_input("  Homeserver URL [https://matrix.org]: ");
            let homeserver = if homeserver.is_empty() {
                "https://matrix.org".to_string()
            } else {
                homeserver
            };
            let token = prompt_input("  Access token: ");

            let config_block = "\n[channels.matrix]\nhomeserver_env = \"MATRIX_HOMESERVER\"\naccess_token_env = \"MATRIX_ACCESS_TOKEN\"\ndefault_agent = \"assistant\"\n";
            maybe_write_channel_config("matrix", config_block);

            let _ = dotenv::save_env_key("MATRIX_HOMESERVER", &homeserver);
            if !token.is_empty() {
                match dotenv::save_env_key("MATRIX_ACCESS_TOKEN", &token) {
                    Ok(()) => ui::success(&i18n::t("channel-token-saved")),
                    Err(_) => println!("    export MATRIX_ACCESS_TOKEN={token}"),
                }
            }

            ui::blank();
            ui::success(&i18n::t_args("channel-configured", &[("name", "Matrix")]));
            notify_daemon_restart();
        }
        other => {
            ui::error_with_fix(
                &i18n::t_args("channel-unknown", &[("name", other)]),
                &i18n::t("channel-unknown-fix"),
            );
            std::process::exit(1);
        }
    }
}

/// Offer to append a channel config block to config.toml if it doesn't already exist.
fn maybe_write_channel_config(channel: &str, config_block: &str) {
    let home = librefang_home();
    let config_path = home.join("config.toml");

    if !config_path.exists() {
        ui::hint(&i18n::t("hint-run-init"));
        return;
    }

    let existing = std::fs::read_to_string(&config_path).unwrap_or_default();
    let section_header = format!("[channels.{channel}]");
    if existing.contains(&section_header) {
        ui::check_ok(&format!("{section_header} already in config.toml"));
        return;
    }

    let answer = prompt_input("  Write to config.toml? [Y/n] ");
    if answer.is_empty() || answer.starts_with('y') || answer.starts_with('Y') {
        let mut content = existing;
        content.push_str(config_block);
        if std::fs::write(&config_path, &content).is_ok() {
            restrict_file_permissions(&config_path);
            ui::check_ok(&format!("Added {section_header} to config.toml"));
        } else {
            ui::check_fail("Failed to write config.toml");
        }
    }
}

/// After channel config changes, warn user if daemon is running.
fn notify_daemon_restart() {
    if find_daemon().is_some() {
        ui::check_warn("Restart the daemon to activate this channel");
    } else {
        ui::hint(&i18n::t("hint-start-daemon-cmd"));
    }
}

fn channel_test_request_body(
    channel_id: Option<&str>,
    chat_id: Option<&str>,
) -> Option<serde_json::Value> {
    channel_id
        .map(|id| serde_json::json!({ "channel_id": id }))
        .or_else(|| chat_id.map(|id| serde_json::json!({ "chat_id": id })))
}

fn cmd_channel_test(channel: &str, channel_id: Option<&str>, chat_id: Option<&str>) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let request = client.post(format!("{base}/api/channels/{channel}/test"));
        let body = if let Some(payload) = channel_test_request_body(channel_id, chat_id) {
            daemon_json(request.json(&payload).send())
        } else {
            daemon_json(request.send())
        };
        if body["status"].as_str() == Some("ok") {
            println!(
                "{}",
                body["message"]
                    .as_str()
                    .unwrap_or("Channel test completed successfully.")
            );
        } else {
            eprintln!(
                "Failed: {}",
                body["message"]
                    .as_str()
                    .or_else(|| body["error"].as_str())
                    .unwrap_or("Unknown error")
            );
            std::process::exit(1);
        }
    } else {
        eprintln!("Channel test requires a running daemon. Start with: librefang start");
        std::process::exit(1);
    }
}

fn cmd_channel_toggle(channel: &str, enable: bool) {
    let action = if enable { "enabled" } else { "disabled" };
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let endpoint = if enable { "enable" } else { "disable" };
        let body = daemon_json(
            client
                .post(format!("{base}/api/channels/{channel}/{endpoint}"))
                .send(),
        );
        if body.get("status").is_some() {
            println!("Channel {channel} {action}.");
        } else {
            eprintln!(
                "Failed: {}",
                body["error"].as_str().unwrap_or("Unknown error")
            );
        }
    } else {
        println!("Note: Channel {channel} will be {action} when the daemon starts.");
        println!("Edit ~/.librefang/config.toml to persist this change.");
    }
}

// ---------------------------------------------------------------------------
// Hand commands
// ---------------------------------------------------------------------------

fn cmd_hand_install(path: &str) {
    let base = require_daemon("hand install");
    let dir = std::path::Path::new(path);
    let toml_path = dir.join("HAND.toml");
    let skill_path = dir.join("SKILL.md");

    if !toml_path.exists() {
        eprintln!(
            "Error: No HAND.toml found in {}",
            dir.canonicalize()
                .unwrap_or_else(|_| dir.to_path_buf())
                .display()
        );
        std::process::exit(1);
    }

    let toml_content = std::fs::read_to_string(&toml_path).unwrap_or_else(|e| {
        eprintln!("Error reading {}: {e}", toml_path.display());
        std::process::exit(1);
    });
    let skill_content = std::fs::read_to_string(&skill_path).unwrap_or_default();

    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/hands/install"))
            .json(&serde_json::json!({
                "toml_content": toml_content,
                "skill_content": skill_content,
            }))
            .send(),
    );

    if let Some(err) = body.get("error").and_then(|v| v.as_str()) {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }

    println!(
        "Installed hand: {} ({})",
        body["name"].as_str().unwrap_or("?"),
        body["id"].as_str().unwrap_or("?"),
    );
    println!(
        "Use `librefang hand activate {}` to start it.",
        body["id"].as_str().unwrap_or("?")
    );
}

fn cmd_hand_list() {
    let base = require_daemon("hand list");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/hands")).send());
    // API returns {"hands": [...]} or a bare array
    let arr_val;
    if let Some(arr) = body.get("hands").and_then(|v| v.as_array()) {
        arr_val = arr.clone();
    } else if let Some(arr) = body.as_array() {
        arr_val = arr.clone();
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = Some(&arr_val) {
        if arr.is_empty() {
            println!("No hands available.");
            return;
        }
        println!("{:<14} {:<20} {:<10} DESCRIPTION", "ID", "NAME", "CATEGORY");
        println!("{}", "-".repeat(72));
        for h in arr {
            println!(
                "{:<14} {:<20} {:<10} {}",
                h["id"].as_str().unwrap_or("?"),
                h["name"].as_str().unwrap_or("?"),
                h["category"].as_str().unwrap_or("?"),
                h["description"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(40)
                    .collect::<String>(),
            );
        }
        println!("\nUse `librefang hand activate <id>` to activate a hand.");
    }
}

fn cmd_hand_active() {
    let base = require_daemon("hand active");
    let client = daemon_client();
    let arr = fetch_active_hand_instances(&base, &client);
    if arr.is_empty() {
        println!("No active hands.");
        return;
    }
    println!("{:<38} {:<14} {:<10} AGENT", "INSTANCE", "HAND", "STATUS");
    println!("{}", "-".repeat(72));
    for i in &arr {
        println!(
            "{:<38} {:<14} {:<10} {}",
            i["instance_id"].as_str().unwrap_or("?"),
            i["hand_id"].as_str().unwrap_or("?"),
            i["status"].as_str().unwrap_or("?"),
            i["agent_name"].as_str().unwrap_or("?"),
        );
    }
}

fn cmd_hand_status(id: Option<&str>) {
    if id.is_none() {
        cmd_hand_active();
        return;
    }

    let id = id.unwrap_or_default();
    let base = require_daemon("hand status");
    let client = daemon_client();
    let active = fetch_active_hand_instances(&base, &client);

    if let Some(instance) = resolve_hand_instance(&active, id) {
        let hand_id = instance["hand_id"].as_str().unwrap_or(id);
        let hand_body = daemon_json(client.get(format!("{base}/api/hands/{hand_id}")).send());
        let name = hand_body["name"].as_str().unwrap_or(hand_id);
        let status = instance["status"].as_str().unwrap_or("unknown");
        let instance_id = instance["instance_id"].as_str().unwrap_or("?");
        let agent_name = instance["agent_name"].as_str().unwrap_or("?");

        ui::section("Hand Status");
        ui::kv("Hand", hand_id);
        ui::kv("Name", name);
        ui::kv("Instance", instance_id);
        ui::kv("Status", status);
        ui::kv("Agent", agent_name);
        return;
    }

    let hand_body = daemon_json(client.get(format!("{base}/api/hands/{id}")).send());
    if hand_body.get("error").is_some() {
        ui::error(&format!(
            "No active hand or installed hand found for '{id}'."
        ));
        std::process::exit(1);
    }

    ui::section("Hand Status");
    ui::kv("Hand", hand_body["id"].as_str().unwrap_or(id));
    ui::kv("Name", hand_body["name"].as_str().unwrap_or(id));
    ui::kv("Status", "inactive");
    if let Some(description) = hand_body["description"].as_str() {
        if !description.is_empty() {
            ui::kv("Description", description);
        }
    }
}

fn cmd_hand_activate(id: &str) {
    let base = require_daemon("hand activate");
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/hands/{id}/activate"))
            .header("content-type", "application/json")
            .body("{}")
            .send(),
    );
    if body.get("instance_id").is_some() {
        println!(
            "Hand '{}' activated (instance: {}, agent: {})",
            id,
            body["instance_id"].as_str().unwrap_or("?"),
            body["agent_name"].as_str().unwrap_or("?"),
        );
    } else {
        eprintln!(
            "Failed to activate hand '{}': {}",
            id,
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }
}

fn cmd_hand_deactivate(id: &str) {
    let base = require_daemon("hand deactivate");
    let client = daemon_client();
    // First find the instance ID for this hand
    let arr = fetch_active_hand_instances(&base, &client);
    let instance_id = arr.iter().find_map(|i| {
        if i["hand_id"].as_str() == Some(id) {
            i["instance_id"].as_str().map(|s| s.to_string())
        } else {
            None
        }
    });

    match instance_id {
        Some(iid) => {
            let body = daemon_json(
                client
                    .delete(format!("{base}/api/hands/instances/{iid}"))
                    .send(),
            );
            if body.get("status").is_some() {
                println!("Hand '{id}' deactivated.");
            } else {
                eprintln!(
                    "Failed: {}",
                    body["error"].as_str().unwrap_or("Unknown error")
                );
                std::process::exit(1);
            }
        }
        None => {
            eprintln!("No active instance found for hand '{id}'.");
            std::process::exit(1);
        }
    }
}

fn cmd_hand_info(id: &str) {
    let base = require_daemon("hand info");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/hands/{id}")).send());
    if body.get("error").is_some() {
        eprintln!("Hand not found: {}", body["error"].as_str().unwrap_or(id));
        std::process::exit(1);
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&body).unwrap_or_default()
    );
}

fn cmd_hand_check_deps(id: &str) {
    let base = require_daemon("hand check-deps");
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/hands/{id}/check-deps"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_hand_install_deps(id: &str) {
    let base = require_daemon("hand install-deps");
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/hands/{id}/install-deps"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    } else {
        ui::success(&i18n::t_args("hand-install-deps-success", &[("id", id)]));
        if let Some(results) = body.get("results") {
            println!(
                "{}",
                serde_json::to_string_pretty(results).unwrap_or_default()
            );
        }
    }
}

fn cmd_hand_pause(id: &str) {
    let base = require_daemon("hand pause");
    let client = daemon_client();
    let active = fetch_active_hand_instances(&base, &client);
    let resolved = resolve_hand_instance(&active, id);
    let instance_id = resolved
        .as_ref()
        .and_then(|instance| instance["instance_id"].as_str())
        .unwrap_or(id);
    let hand_label = resolved
        .as_ref()
        .and_then(|instance| instance["hand_id"].as_str())
        .unwrap_or(id);
    let body = daemon_json(
        client
            .post(format!("{base}/api/hands/instances/{instance_id}/pause"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
        std::process::exit(1);
    } else {
        ui::success(&i18n::t_args(
            "hand-paused",
            &[("id", &format!("{hand_label} (instance: {instance_id})"))],
        ));
    }
}

fn cmd_hand_resume(id: &str) {
    let base = require_daemon("hand resume");
    let client = daemon_client();
    let active = fetch_active_hand_instances(&base, &client);
    let resolved = resolve_hand_instance(&active, id);
    let instance_id = resolved
        .as_ref()
        .and_then(|instance| instance["instance_id"].as_str())
        .unwrap_or(id);
    let hand_label = resolved
        .as_ref()
        .and_then(|instance| instance["hand_id"].as_str())
        .unwrap_or(id);
    let body = daemon_json(
        client
            .post(format!("{base}/api/hands/instances/{instance_id}/resume"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
        std::process::exit(1);
    } else {
        ui::success(&i18n::t_args(
            "hand-resumed",
            &[("id", &format!("{hand_label} (instance: {instance_id})"))],
        ));
    }
}

fn cmd_hand_settings(id: &str) {
    let base = require_daemon("hand settings");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/hands/{id}/settings")).send());
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
        std::process::exit(1);
    }
    if let Some(config) = body.get("config").and_then(|c| c.as_object()) {
        if config.is_empty() {
            ui::step(&format!("Hand '{id}' has no configurable settings."));
        } else {
            ui::section(&format!("Settings for '{id}'"));
            for (k, v) in config {
                println!("  {}: {}", k.bold(), v);
            }
        }
    } else {
        ui::step(&format!("Hand '{id}' has no configurable settings."));
    }
}

fn cmd_hand_set(id: &str, key: &str, value: &str) {
    let base = require_daemon("hand set");
    let client = daemon_client();
    let mut config = serde_json::Map::new();
    config.insert(
        key.to_string(),
        serde_json::Value::String(value.to_string()),
    );
    let body = daemon_json(
        client
            .put(format!("{base}/api/hands/{id}/settings"))
            .json(&serde_json::json!({ "config": config }))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
        std::process::exit(1);
    }
    ui::success(&format!("Set {key}={value} for hand '{id}'."));
}

fn cmd_hand_reload() {
    let base = require_daemon("hand reload");
    let client = daemon_client();
    let body = daemon_json(client.post(format!("{base}/api/hands/reload")).send());
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
        std::process::exit(1);
    }
    let added = body["added"].as_u64().unwrap_or(0);
    let updated = body["updated"].as_u64().unwrap_or(0);
    let total = body["total"].as_u64().unwrap_or(0);
    ui::success(&format!(
        "Reloaded hands: {added} added, {updated} updated, {total} total."
    ));
}

fn cmd_hand_chat(id: &str) {
    let base = require_daemon("hand chat");
    let client = daemon_client();
    let active = fetch_active_hand_instances(&base, &client);
    let resolved = match resolve_hand_instance(&active, id) {
        Some(instance) => instance,
        None => {
            ui::error(&format!("No active hand instance found for '{id}'."));
            ui::hint("Activate it first: librefang hand activate");
            std::process::exit(1);
        }
    };
    let instance_id = resolved["instance_id"]
        .as_str()
        .expect("instance_id missing");
    let hand_id = resolved["hand_id"].as_str().unwrap_or(id);
    let hand_name = resolved["hand_name"]
        .as_str()
        .or_else(|| resolved["name"].as_str())
        .unwrap_or(hand_id);

    install_ctrlc_handler();

    println!(
        "{} {} {}",
        "Chat with".bold(),
        hand_name.cyan().bold(),
        "(type /quit to exit)".dimmed()
    );
    println!();

    loop {
        print!("{} ", "you >".green().bold());
        io::stdout().flush().unwrap();
        let mut line = String::new();
        if io::stdin().lock().read_line(&mut line).unwrap_or(0) == 0 {
            break; // EOF
        }
        let msg = line.trim();
        if msg.is_empty() {
            continue;
        }
        if msg == "/quit" || msg == "/exit" || msg == "/q" {
            break;
        }

        let resp = client
            .post(format!("{base}/api/hands/instances/{instance_id}/message"))
            .json(&serde_json::json!({"message": msg}))
            .send();

        let body = daemon_json(resp);
        if let Some(err) = body["error"].as_str() {
            ui::error(err);
            continue;
        }
        let reply = body["response"]
            .as_str()
            .or_else(|| body["reply"].as_str())
            .unwrap_or("[no response]");
        println!("{} {}\n", format!("{hand_name} >").cyan().bold(), reply);
    }
}

fn fetch_active_hand_instances(
    base: &str,
    client: &reqwest::blocking::Client,
) -> Vec<serde_json::Value> {
    let body = daemon_json(client.get(format!("{base}/api/hands/active")).send());
    body.get("instances")
        .and_then(|v| v.as_array())
        .or_else(|| body.as_array())
        .cloned()
        .unwrap_or_default()
}

fn resolve_hand_instance(
    active_instances: &[serde_json::Value],
    id_or_hand: &str,
) -> Option<serde_json::Value> {
    active_instances
        .iter()
        .find(|instance| {
            instance["instance_id"].as_str() == Some(id_or_hand)
                || instance["hand_id"].as_str() == Some(id_or_hand)
        })
        .cloned()
}

// ---------------------------------------------------------------------------
// Provider / API key helpers
// ---------------------------------------------------------------------------

/// Map a provider name to its conventional environment variable name.
fn provider_to_env_var(provider: &str) -> String {
    match provider.to_lowercase().as_str() {
        "groq" => "GROQ_API_KEY".to_string(),
        "anthropic" => "ANTHROPIC_API_KEY".to_string(),
        "openai" => "OPENAI_API_KEY".to_string(),
        "gemini" => "GEMINI_API_KEY".to_string(),
        "google" => "GOOGLE_API_KEY".to_string(),
        "deepseek" => "DEEPSEEK_API_KEY".to_string(),
        "openrouter" => "OPENROUTER_API_KEY".to_string(),
        "together" => "TOGETHER_API_KEY".to_string(),
        "mistral" => "MISTRAL_API_KEY".to_string(),
        "fireworks" => "FIREWORKS_API_KEY".to_string(),
        "perplexity" => "PERPLEXITY_API_KEY".to_string(),
        "cohere" => "COHERE_API_KEY".to_string(),
        "xai" => "XAI_API_KEY".to_string(),
        "brave" => "BRAVE_API_KEY".to_string(),
        "tavily" => "TAVILY_API_KEY".to_string(),
        other => format!("{}_API_KEY", other.to_uppercase()),
    }
}

/// Test an API key by hitting the provider's models/health endpoint.
///
/// Returns true if the key is accepted (status != 401/403).
/// Returns true on timeout/network errors (best-effort — don't block setup).
pub(crate) fn test_api_key(provider: &str, key: &str) -> bool {
    if key.is_empty() {
        return false;
    }

    let client = match crate::http_client::client_builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(_) => return true, // can't build client — assume ok
    };

    let result = match provider.to_lowercase().as_str() {
        "groq" => client
            .get("https://api.groq.com/openai/v1/models")
            .bearer_auth(key)
            .send(),
        "anthropic" => client
            .get("https://api.anthropic.com/v1/models")
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01")
            .send(),
        "openai" => client
            .get("https://api.openai.com/v1/models")
            .bearer_auth(key)
            .send(),
        "gemini" | "google" => client
            .get(format!(
                "https://generativelanguage.googleapis.com/v1beta/models?key={key}"
            ))
            .send(),
        "deepseek" => client
            .get("https://api.deepseek.com/models")
            .bearer_auth(key)
            .send(),
        "openrouter" => client
            .get("https://openrouter.ai/api/v1/models")
            .bearer_auth(key)
            .send(),
        "elevenlabs" => client
            .get("https://api.elevenlabs.io/v1/user")
            .header("xi-api-key", key)
            .send(),
        _ => return true, // unknown provider — skip test
    };

    match result {
        Ok(resp) => {
            let status = resp.status().as_u16();
            status != 401 && status != 403
        }
        Err(_) => true, // network error — don't block setup
    }
}

// ---------------------------------------------------------------------------
// Background daemon start
// ---------------------------------------------------------------------------

/// Spawn `librefang start` as a detached background process.
///
/// Polls for daemon health for up to 10 seconds. Returns the daemon URL on success.
pub(crate) fn start_daemon_background() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| format!("Cannot find executable: {e}"))?;

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x00000008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        std::process::Command::new(&exe)
            .arg("start")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
            .spawn()
            .map_err(|e| format!("Failed to spawn daemon: {e}"))?;
    }

    #[cfg(not(windows))]
    {
        std::process::Command::new(&exe)
            .arg("start")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to spawn daemon: {e}"))?;
    }

    // Poll for daemon readiness
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        if let Some(url) = find_daemon() {
            return Ok(url);
        }
    }

    Err("Daemon did not become ready within 10 seconds".to_string())
}

// ---------------------------------------------------------------------------
// Config commands
// ---------------------------------------------------------------------------

fn cmd_config_show() {
    let home = librefang_home();
    let config_path = home.join("config.toml");

    if !config_path.exists() {
        println!("No configuration found at: {}", config_path.display());
        println!("Run `librefang init` to create one.");
        return;
    }

    let content = std::fs::read_to_string(&config_path).unwrap_or_else(|e| {
        eprintln!("Error reading config: {e}");
        std::process::exit(1);
    });

    println!("# {}\n", config_path.display());
    println!("{content}");
}

fn cmd_config_edit() {
    let home = librefang_home();
    let config_path = home.join("config.toml");

    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| {
            if cfg!(windows) {
                "notepad".to_string()
            } else {
                "vi".to_string()
            }
        });

    let status = std::process::Command::new(&editor)
        .arg(&config_path)
        .status();

    match status {
        Ok(s) if s.success() => {}
        Ok(s) => {
            eprintln!("Editor exited with: {s}");
        }
        Err(e) => {
            eprintln!("Failed to open editor '{editor}': {e}");
            eprintln!("Set $EDITOR to your preferred editor.");
        }
    }
}

fn cmd_config_get(key: &str) {
    let home = librefang_home();
    let config_path = home.join("config.toml");

    if !config_path.exists() {
        ui::error_with_fix(&i18n::t("config-no-file"), &i18n::t("config-no-file-fix"));
        std::process::exit(1);
    }

    let content = std::fs::read_to_string(&config_path).unwrap_or_else(|e| {
        ui::error(&i18n::t_args(
            "config-read-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    });

    let table: toml::Value = toml::from_str(&content).unwrap_or_else(|e| {
        ui::error_with_fix(
            &i18n::t_args("config-parse-error", &[("error", &e.to_string())]),
            &i18n::t("config-parse-fix"),
        );
        std::process::exit(1);
    });

    // Navigate dotted path
    let mut current = &table;
    for part in key.split('.') {
        match current.get(part) {
            Some(v) => current = v,
            None => {
                ui::error(&i18n::t_args("config-key-not-found", &[("key", key)]));
                std::process::exit(1);
            }
        }
    }

    // Print value
    match current {
        toml::Value::String(s) => println!("{s}"),
        toml::Value::Integer(i) => println!("{i}"),
        toml::Value::Float(f) => println!("{f}"),
        toml::Value::Boolean(b) => println!("{b}"),
        other => println!("{other}"),
    }
}

fn cmd_config_set(key: &str, value: &str) {
    let home = librefang_home();
    let config_path = home.join("config.toml");

    if !config_path.exists() {
        ui::error_with_fix(&i18n::t("config-no-file"), &i18n::t("config-no-file-fix"));
        std::process::exit(1);
    }

    let content = std::fs::read_to_string(&config_path).unwrap_or_else(|e| {
        ui::error(&i18n::t_args(
            "config-read-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    });

    let mut table: toml::Value = toml::from_str(&content).unwrap_or_else(|e| {
        ui::error_with_fix(
            &i18n::t_args("config-parse-error", &[("error", &e.to_string())]),
            &i18n::t("config-parse-fix-alt"),
        );
        std::process::exit(1);
    });

    // Navigate to parent and set key
    let parts: Vec<&str> = key.split('.').collect();
    if parts.is_empty() {
        ui::error(&i18n::t("config-empty-key"));
        std::process::exit(1);
    }

    let mut current = &mut table;
    for part in &parts[..parts.len() - 1] {
        current = current
            .as_table_mut()
            .and_then(|t| t.get_mut(*part))
            .unwrap_or_else(|| {
                ui::error(&i18n::t_args("config-key-path-not-found", &[("key", key)]));
                std::process::exit(1);
            });
    }

    let last_key = parts[parts.len() - 1];

    // Validate: single-part keys must be known scalar fields, not sections.
    // Writing a section name as a scalar silently breaks config deserialization.
    if parts.len() == 1 {
        let known_scalars = [
            "home_dir",
            "data_dir",
            "log_level",
            "api_listen",
            "network_enabled",
            "api_key",
            "language",
            "max_cron_jobs",
            "usage_footer",
            "workspaces_dir",
        ];
        if !known_scalars.contains(&last_key) {
            ui::error_with_fix(
                &i18n::t_args("config-section-not-scalar", &[("key", last_key)]),
                &i18n::t_args("config-section-not-scalar-fix", &[("key", last_key)]),
            );
            std::process::exit(1);
        }
    }

    let tbl = current.as_table_mut().unwrap_or_else(|| {
        ui::error(&i18n::t_args("config-parent-not-table", &[("key", key)]));
        std::process::exit(1);
    });

    // Try to preserve type: if the existing value is an integer, parse as int, etc.
    let new_value = if let Some(existing) = tbl.get(last_key) {
        match existing {
            toml::Value::Integer(_) => value
                .parse::<u64>()
                .map(|v| toml::Value::Integer(v as i64))
                .or_else(|_| value.parse::<i64>().map(toml::Value::Integer))
                .unwrap_or_else(|_| toml::Value::String(value.to_string())),
            toml::Value::Float(_) => value
                .parse::<f64>()
                .map(toml::Value::Float)
                .unwrap_or_else(|_| toml::Value::String(value.to_string())),
            toml::Value::Boolean(_) => value
                .parse::<bool>()
                .map(toml::Value::Boolean)
                .unwrap_or_else(|_| toml::Value::String(value.to_string())),
            _ => toml::Value::String(value.to_string()),
        }
    } else {
        // No existing value — infer type from the string content
        if let Ok(b) = value.parse::<bool>() {
            toml::Value::Boolean(b)
        } else if let Ok(i) = value.parse::<u64>() {
            toml::Value::Integer(i as i64)
        } else if let Ok(i) = value.parse::<i64>() {
            toml::Value::Integer(i)
        } else if let Ok(f) = value.parse::<f64>() {
            toml::Value::Float(f)
        } else {
            toml::Value::String(value.to_string())
        }
    };

    tbl.insert(last_key.to_string(), new_value);

    // Write back (note: this strips comments — warned in help text)
    let serialized = toml::to_string_pretty(&table).unwrap_or_else(|e| {
        ui::error(&i18n::t_args(
            "config-serialize-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    });

    std::fs::write(&config_path, &serialized).unwrap_or_else(|e| {
        ui::error(&i18n::t_args(
            "config-write-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    });
    restrict_file_permissions(&config_path);

    ui::success(&i18n::t_args(
        "config-set-kv",
        &[("key", key), ("value", value)],
    ));
}

fn cmd_config_unset(key: &str) {
    let home = librefang_home();
    let config_path = home.join("config.toml");

    if !config_path.exists() {
        ui::error_with_fix(&i18n::t("config-no-file"), &i18n::t("config-no-file-fix"));
        std::process::exit(1);
    }

    let content = std::fs::read_to_string(&config_path).unwrap_or_else(|e| {
        ui::error(&i18n::t_args(
            "config-read-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    });

    let mut table: toml::Value = toml::from_str(&content).unwrap_or_else(|e| {
        ui::error_with_fix(
            &i18n::t_args("config-parse-error", &[("error", &e.to_string())]),
            &i18n::t("config-parse-fix-alt"),
        );
        std::process::exit(1);
    });

    // Navigate to parent table and remove the final key
    let parts: Vec<&str> = key.split('.').collect();
    if parts.is_empty() {
        ui::error(&i18n::t("config-empty-key"));
        std::process::exit(1);
    }

    let mut current = &mut table;
    for part in &parts[..parts.len() - 1] {
        current = current
            .as_table_mut()
            .and_then(|t| t.get_mut(*part))
            .unwrap_or_else(|| {
                ui::error(&i18n::t_args("config-key-path-not-found", &[("key", key)]));
                std::process::exit(1);
            });
    }

    let last_key = parts[parts.len() - 1];
    let tbl = current.as_table_mut().unwrap_or_else(|| {
        ui::error(&i18n::t_args("config-parent-not-table", &[("key", key)]));
        std::process::exit(1);
    });

    if tbl.remove(last_key).is_none() {
        ui::error(&i18n::t_args("config-key-not-found", &[("key", key)]));
        std::process::exit(1);
    }

    // Write back (note: this strips comments — warned in help text)
    let serialized = toml::to_string_pretty(&table).unwrap_or_else(|e| {
        ui::error(&i18n::t_args(
            "config-serialize-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    });

    std::fs::write(&config_path, &serialized).unwrap_or_else(|e| {
        ui::error(&i18n::t_args(
            "config-write-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    });
    restrict_file_permissions(&config_path);

    ui::success(&i18n::t_args("config-removed-key", &[("key", key)]));
}

fn cmd_config_set_key(provider: &str) {
    let env_var = provider_to_env_var(provider);

    let key = prompt_input(&format!("  Paste your {provider} API key: "));
    if key.is_empty() {
        ui::error(&i18n::t("config-no-key"));
        return;
    }

    match dotenv::save_env_key(&env_var, &key) {
        Ok(()) => {
            ui::success(&i18n::t_args("config-saved-key", &[("env_var", &env_var)]));
            // Test the key
            print!("  Testing key... ");
            io::stdout().flush().unwrap();
            if test_api_key(provider, &key) {
                println!("{}", "OK".bright_green());
            } else {
                println!("{}", "could not verify (may still work)".bright_yellow());
            }
        }
        Err(e) => {
            ui::error(&i18n::t_args(
                "config-save-key-failed",
                &[("error", &e.to_string())],
            ));
            std::process::exit(1);
        }
    }
}

fn cmd_config_delete_key(provider: &str) {
    let env_var = provider_to_env_var(provider);

    match dotenv::remove_env_key(&env_var) {
        Ok(()) => ui::success(&i18n::t_args(
            "config-removed-env",
            &[("env_var", &env_var)],
        )),
        Err(e) => {
            ui::error(&i18n::t_args(
                "config-remove-key-failed",
                &[("error", &e.to_string())],
            ));
            std::process::exit(1);
        }
    }
}

fn cmd_config_test_key(provider: &str) {
    let env_var = provider_to_env_var(provider);

    if std::env::var(&env_var).is_err() {
        ui::error(&i18n::t_args(
            "config-env-not-set",
            &[("env_var", &env_var)],
        ));
        ui::hint(&i18n::t_args(
            "config-set-key-hint",
            &[("provider", provider)],
        ));
        std::process::exit(1);
    }

    print!("  Testing {provider} ({env_var})... ");
    io::stdout().flush().unwrap();
    if test_api_key(provider, &std::env::var(&env_var).unwrap_or_default()) {
        println!("{}", "OK".bright_green());
    } else {
        println!("{}", "FAILED (401/403)".bright_red());
        ui::hint(&i18n::t_args(
            "config-update-key-hint",
            &[("provider", provider)],
        ));
        std::process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// Quick chat (OpenClaw alias)
// ---------------------------------------------------------------------------

fn cmd_quick_chat(config: Option<PathBuf>, agent: Option<String>) {
    ensure_initialized(&config);
    tui::chat_runner::run_chat_tui(config, agent);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(crate) fn librefang_home() -> PathBuf {
    if let Ok(home) = std::env::var("LIBREFANG_HOME") {
        return PathBuf::from(home);
    }
    dirs::home_dir()
        .unwrap_or_else(|| {
            eprintln!("Error: Could not determine home directory");
            std::process::exit(1);
        })
        .join(".librefang")
}

fn prompt_input(prompt: &str) -> String {
    print!("{prompt}");
    io::stdout().flush().unwrap();
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line).unwrap_or(0);
    line.trim().to_string()
}

pub(crate) fn copy_dir_recursive(src: &PathBuf, dst: &PathBuf) {
    std::fs::create_dir_all(dst).unwrap();
    if let Ok(entries) = std::fs::read_dir(src) {
        for entry in entries.flatten() {
            let path = entry.path();
            let dest_path = dst.join(entry.file_name());
            if path.is_dir() {
                copy_dir_recursive(&path, &dest_path);
            } else {
                let _ = std::fs::copy(&path, &dest_path);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// MCP server commands (librefang mcp {add,remove,list,catalog})
// ---------------------------------------------------------------------------

fn cmd_mcp_add(name: &str, key: Option<&str>) {
    let home = librefang_home();
    let mut catalog = librefang_extensions::catalog::McpCatalog::new(&home);
    catalog.load(&home);

    // Check template exists
    let template = match catalog.get(name) {
        Some(t) => t.clone(),
        None => {
            ui::error(&format!("Unknown MCP catalog entry: '{name}'"));
            println!("\nAvailable MCP servers (catalog):");
            for t in catalog.list() {
                println!("  {} {} — {}", t.icon, t.id, t.description);
            }
            std::process::exit(1);
        }
    };

    // Reject re-install of an already-configured server by name/template_id.
    // The API path returns 409 here; the CLI was silently overwriting the
    // existing [[mcp_servers]] entry (including edited transport/env/oauth)
    // because upsert_mcp_server_local replaces by name. Users should remove
    // first if they want to re-install.
    let config_path = home.join("config.toml");
    if config_path.is_file() {
        let content = match std::fs::read_to_string(&config_path) {
            Ok(c) => c,
            Err(e) => {
                ui::error(&format!("Failed to read {}: {e}", config_path.display()));
                std::process::exit(1);
            }
        };
        let parsed: toml::value::Table = match toml::from_str(&content) {
            Ok(t) => t,
            Err(e) => {
                ui::error(&format!("{} is not valid TOML: {e}", config_path.display()));
                std::process::exit(1);
            }
        };
        if let Some(toml::Value::Array(servers)) = parsed.get("mcp_servers") {
            let conflict = servers.iter().any(|v| {
                let t = match v.as_table() {
                    Some(t) => t,
                    None => return false,
                };
                let matches_field = |k: &str| t.get(k).and_then(|n| n.as_str()) == Some(name);
                matches_field("name") || matches_field("template_id")
            });
            if conflict {
                ui::error(&format!(
                    "MCP server '{name}' is already configured. Run \
                     `librefang mcp remove {name}` first if you want to re-install."
                ));
                std::process::exit(1);
            }
        }
    }

    // Set up credential resolver (vault + dotenv + interactive prompt fallback)
    let dotenv_path = home.join(".env");
    let vault_path = home.join("vault.enc");
    let vault = if vault_path.exists() {
        let mut v = librefang_extensions::vault::CredentialVault::new(vault_path);
        if v.unlock().is_ok() {
            Some(v)
        } else {
            None
        }
    } else {
        None
    };
    let mut resolver =
        librefang_extensions::credentials::CredentialResolver::new(vault, Some(&dotenv_path))
            .with_interactive(true);

    // Build provided keys map
    let mut provided_keys = std::collections::HashMap::new();
    if let Some(key_value) = key {
        // Auto-detect which env var to use (first required_env that's a secret)
        if let Some(env_var) = template.required_env.iter().find(|e| e.is_secret) {
            provided_keys.insert(env_var.name.clone(), key_value.to_string());
        }
    }

    let result = match librefang_extensions::installer::install_integration(
        &catalog,
        &mut resolver,
        name,
        &provided_keys,
    ) {
        Ok(r) => r,
        Err(e) => {
            ui::error(&e.to_string());
            std::process::exit(1);
        }
    };

    // Persist the new [[mcp_servers]] entry directly into config.toml.
    let config_path = home.join("config.toml");
    if let Err(e) = upsert_mcp_server_local(&config_path, &result.server) {
        ui::error(&format!("Failed to write config.toml: {e}"));
        std::process::exit(1);
    }

    match &result.status {
        librefang_extensions::McpStatus::Ready => ui::success(&result.message),
        librefang_extensions::McpStatus::Setup => {
            println!("{}", result.message.yellow());
            println!("\nTo add credentials:");
            for env in &template.required_env {
                if env.is_secret {
                    println!("  librefang vault set {}  # {}", env.name, env.help);
                    if let Some(ref url) = env.get_url {
                        println!("  Get it here: {url}");
                    }
                }
            }
        }
        _ => println!("{}", result.message),
    }

    // If daemon is running, trigger hot-reload.
    if let Some(base_url) = find_daemon() {
        let client = daemon_client();
        let _ = client.post(format!("{base_url}/api/mcp/reload")).send();
    }
}

fn cmd_mcp_remove(name: &str) {
    let home = librefang_home();
    let config_path = home.join("config.toml");

    // Resolve by template_id first, fall back to server name.
    let target_name: Option<String> = {
        let raw = std::fs::read_to_string(&config_path).unwrap_or_default();
        let doc: toml::Value =
            toml::from_str(&raw).unwrap_or(toml::Value::Table(Default::default()));
        doc.as_table()
            .and_then(|t| t.get("mcp_servers"))
            .and_then(|v| v.as_array())
            .and_then(|arr| {
                arr.iter().find_map(|entry| {
                    let tbl = entry.as_table()?;
                    let tid = tbl.get("template_id").and_then(|v| v.as_str());
                    let nm = tbl.get("name").and_then(|v| v.as_str())?;
                    if tid == Some(name) || nm == name {
                        Some(nm.to_string())
                    } else {
                        None
                    }
                })
            })
    };

    let target_name = match target_name {
        Some(n) => n,
        None => {
            ui::error(&format!("MCP server '{name}' is not configured"));
            std::process::exit(1);
        }
    };

    if let Err(e) = remove_mcp_server_local(&config_path, &target_name) {
        ui::error(&format!("Failed to update config.toml: {e}"));
        std::process::exit(1);
    }

    ui::success(&format!("{target_name} removed."));

    // Hot-reload daemon
    if let Some(base_url) = find_daemon() {
        let client = daemon_client();
        let _ = client.post(format!("{base_url}/api/mcp/reload")).send();
    }
}

fn cmd_mcp_catalog(query: Option<&str>) {
    let home = librefang_home();
    let mut catalog = librefang_extensions::catalog::McpCatalog::new(&home);
    catalog.load(&home);

    // Installed state comes from config.mcp_servers' template_id field.
    let installed_template_ids: std::collections::HashSet<String> = {
        let raw = std::fs::read_to_string(home.join("config.toml")).unwrap_or_default();
        toml::from_str::<toml::Value>(&raw)
            .ok()
            .and_then(|v| v.as_table().cloned())
            .and_then(|t| t.get("mcp_servers").cloned())
            .and_then(|v| v.as_array().cloned())
            .map(|arr| {
                arr.into_iter()
                    .filter_map(|v| {
                        v.as_table()
                            .and_then(|t| t.get("template_id"))
                            .and_then(|t| t.as_str())
                            .map(|s| s.to_string())
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    let entries: Vec<_> = if let Some(q) = query {
        catalog.search(q).into_iter().cloned().collect()
    } else {
        catalog.list().into_iter().cloned().collect()
    };

    if entries.is_empty() {
        if let Some(q) = query {
            println!("No MCP catalog entries matching '{q}'.");
        } else {
            println!("No MCP catalog entries available.");
        }
        return;
    }

    // Group by category
    let mut by_category: std::collections::BTreeMap<
        String,
        Vec<&librefang_extensions::McpCatalogEntry>,
    > = std::collections::BTreeMap::new();
    for entry in &entries {
        by_category
            .entry(entry.category.to_string())
            .or_default()
            .push(entry);
    }

    for (category, items) in &by_category {
        println!("\n{}", format!("  {category}").bold());
        for item in items {
            let status_badge = if installed_template_ids.contains(&item.id) {
                "[Installed]".green().to_string()
            } else {
                "[Available]".dimmed().to_string()
            };
            println!(
                "    {} {:<20} {:<13} {}",
                item.icon, item.id, status_badge, item.description
            );
        }
    }
    println!();
    println!(
        "  {} catalog entries ({} installed)",
        entries.len(),
        entries
            .iter()
            .filter(|e| installed_template_ids.contains(&e.id))
            .count()
    );
    println!("  Use `librefang mcp add <id>` to install an MCP server.");
}

fn cmd_mcp_list() {
    let home = librefang_home();
    let raw = std::fs::read_to_string(home.join("config.toml")).unwrap_or_default();
    let doc: toml::Value = toml::from_str(&raw).unwrap_or(toml::Value::Table(Default::default()));
    let servers = doc
        .as_table()
        .and_then(|t| t.get("mcp_servers"))
        .and_then(|v| v.as_array());
    let Some(servers) = servers else {
        println!("No MCP servers configured.");
        return;
    };
    if servers.is_empty() {
        println!("No MCP servers configured.");
        return;
    }
    println!();
    println!(
        "  {:<28} {:<14} {:<18} details",
        "name", "template_id", "transport"
    );
    for entry in servers {
        let Some(tbl) = entry.as_table() else {
            continue;
        };
        let name = tbl.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let tid = tbl
            .get("template_id")
            .and_then(|v| v.as_str())
            .unwrap_or("-");
        let (transport, detail) = match tbl.get("transport").and_then(|v| v.as_table()) {
            Some(t) => {
                let ttype = t.get("type").and_then(|v| v.as_str()).unwrap_or("?");
                let detail = match ttype {
                    "stdio" => t
                        .get("command")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    "sse" | "http" => t
                        .get("url")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    _ => String::new(),
                };
                (ttype.to_string(), detail)
            }
            None => ("-".to_string(), String::new()),
        };
        println!("  {name:<28} {tid:<14} {transport:<18} {detail}");
    }
    println!();
    println!("  Use `librefang mcp catalog` to list installable entries.");
}

/// Local upsert helper — mirrors the API's `upsert_mcp_server_config`.
fn upsert_mcp_server_local(
    config_path: &std::path::Path,
    entry: &librefang_types::config::McpServerConfigEntry,
) -> Result<(), String> {
    let mut table: toml::value::Table = if config_path.exists() {
        let content = std::fs::read_to_string(config_path).map_err(|e| e.to_string())?;
        // Propagate parse errors instead of silently defaulting. A
        // malformed config.toml would otherwise be overwritten as a new
        // near-empty file, wiping unrelated sections the user may want
        // to fix by hand.
        toml::from_str(&content).map_err(|e| format!("config.toml is not valid TOML: {e}"))?
    } else {
        toml::value::Table::new()
    };

    let entry_json = serde_json::to_value(entry).map_err(|e| e.to_string())?;
    let entry_toml = json_to_toml_value_cli(&entry_json);

    let servers = table
        .entry("mcp_servers".to_string())
        .or_insert_with(|| toml::Value::Array(Vec::new()));

    if let toml::Value::Array(ref mut arr) = servers {
        arr.retain(|v| {
            v.as_table()
                .and_then(|t| t.get("name"))
                .and_then(|n| n.as_str())
                .map(|n| n != entry.name)
                .unwrap_or(true)
        });
        arr.push(entry_toml);
    }

    let toml_string = toml::to_string_pretty(&table).map_err(|e| e.to_string())?;
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(config_path, toml_string).map_err(|e| e.to_string())?;
    Ok(())
}

fn remove_mcp_server_local(config_path: &std::path::Path, name: &str) -> Result<(), String> {
    let mut table: toml::value::Table = if config_path.exists() {
        let content = std::fs::read_to_string(config_path).map_err(|e| e.to_string())?;
        toml::from_str(&content).map_err(|e| format!("config.toml is not valid TOML: {e}"))?
    } else {
        return Ok(());
    };
    if let Some(toml::Value::Array(ref mut arr)) = table.get_mut("mcp_servers") {
        arr.retain(|v| {
            v.as_table()
                .and_then(|t| t.get("name"))
                .and_then(|n| n.as_str())
                .map(|n| n != name)
                .unwrap_or(true)
        });
    }
    let toml_string = toml::to_string_pretty(&table).map_err(|e| e.to_string())?;
    std::fs::write(config_path, toml_string).map_err(|e| e.to_string())?;
    Ok(())
}

/// JSON → TOML converter. Duplicates the `json_to_toml_value` helper from
/// the API crate to avoid a cross-crate dependency.
fn json_to_toml_value_cli(value: &serde_json::Value) -> toml::Value {
    match value {
        serde_json::Value::Null => toml::Value::String(String::new()),
        serde_json::Value::Bool(b) => toml::Value::Boolean(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                toml::Value::Integer(i)
            } else if let Some(f) = n.as_f64() {
                toml::Value::Float(f)
            } else {
                toml::Value::String(n.to_string())
            }
        }
        serde_json::Value::String(s) => toml::Value::String(s.clone()),
        serde_json::Value::Array(arr) => {
            toml::Value::Array(arr.iter().map(json_to_toml_value_cli).collect())
        }
        serde_json::Value::Object(map) => {
            let mut t = toml::value::Table::new();
            for (k, v) in map {
                t.insert(k.clone(), json_to_toml_value_cli(v));
            }
            toml::Value::Table(t)
        }
    }
}

// ---------------------------------------------------------------------------
// Auth commands (librefang auth chatgpt)
// ---------------------------------------------------------------------------

enum DeviceAuthNextStep {
    ContinueDevice(librefang_runtime::chatgpt_oauth::DeviceAuthPrompt),
    FallbackToBrowser(String),
}

fn resolve_device_auth_start(
    result: Result<
        librefang_runtime::chatgpt_oauth::DeviceAuthPrompt,
        librefang_runtime::chatgpt_oauth::DeviceAuthFlowError,
    >,
) -> Result<DeviceAuthNextStep, String> {
    match result {
        Ok(prompt) => Ok(DeviceAuthNextStep::ContinueDevice(prompt)),
        Err(librefang_runtime::chatgpt_oauth::DeviceAuthFlowError::BrowserFallback { message }) => {
            Ok(DeviceAuthNextStep::FallbackToBrowser(message))
        }
        Err(err) => Err(err.to_string()),
    }
}

async fn authenticate_chatgpt(
    device_auth: bool,
) -> Result<librefang_runtime::chatgpt_oauth::ChatGptAuthResult, String> {
    use librefang_runtime::chatgpt_oauth;

    if device_auth {
        match resolve_device_auth_start(chatgpt_oauth::start_device_auth_flow().await)? {
            DeviceAuthNextStep::ContinueDevice(prompt) => {
                println!("Device authentication requested.");
                println!(
                    "Open this URL in any browser:\n  {}\n",
                    chatgpt_oauth::DEVICE_AUTH_URL
                );
                println!("Enter this one-time code:\n  {}\n", prompt.user_code);
                println!("Do not share this code.");
                println!("Waiting for authorization...");
                return chatgpt_oauth::poll_device_auth_flow(&prompt).await;
            }
            DeviceAuthNextStep::FallbackToBrowser(message) => {
                println!("{message}");
                println!("\nSwitching to the standard browser login flow...\n");
            }
        }
    }

    let (auth_url, port, code_verifier, state) = chatgpt_oauth::start_oauth_flow().await?;

    println!("Opening browser for OpenAI authentication...");
    println!("If the browser does not open, visit:\n  {auth_url}\n");

    if let Err(e) = open::that(&auth_url) {
        eprintln!("Could not open browser automatically: {e}");
        eprintln!("Please open manually: {auth_url}");
    }

    let code = chatgpt_oauth::run_oauth_callback_server(port, &state).await?;
    chatgpt_oauth::exchange_code_for_tokens(&code, &code_verifier, port).await
}

async fn persist_chatgpt_auth(
    auth_result: librefang_runtime::chatgpt_oauth::ChatGptAuthResult,
) -> Result<(), String> {
    use librefang_runtime::chatgpt_oauth;

    let home = librefang_home();
    std::fs::create_dir_all(&home)
        .map_err(|e| format!("Failed to create LibreFang home directory: {e}"))?;

    let access_token = auth_result.access_token;
    let refresh_token = auth_result.refresh_token;
    let secrets_path = write_chatgpt_secrets(
        &home,
        access_token.as_str(),
        refresh_token.as_ref().map(|rt| rt.as_str()),
    )?;

    println!("\nChatGPT tokens saved to {}", secrets_path.display());

    println!("Detecting best available model...");
    let best_model = chatgpt_oauth::fetch_best_codex_model(&access_token).await;
    println!("Selected model: {best_model}");

    update_chatgpt_config(&home, &best_model)?;

    println!("config.toml updated: provider = \"chatgpt\", model = \"{best_model}\"");
    Ok(())
}

fn write_chatgpt_secrets(
    home: &std::path::Path,
    access_token: &str,
    refresh_token: Option<&str>,
) -> Result<std::path::PathBuf, String> {
    let secrets_path = home.join("secrets.env");
    let mut env_vars: Vec<(String, String)> = vec![(
        "CHATGPT_SESSION_TOKEN".to_string(),
        access_token.to_string(),
    )];
    if let Some(rt) = refresh_token {
        env_vars.push(("CHATGPT_REFRESH_TOKEN".to_string(), rt.to_string()));
    }

    let existing = std::fs::read_to_string(&secrets_path).unwrap_or_default();
    let mut lines: Vec<String> = existing
        .lines()
        .filter(|l| {
            !l.starts_with("CHATGPT_SESSION_TOKEN=") && !l.starts_with("CHATGPT_REFRESH_TOKEN=")
        })
        .map(|l| l.to_string())
        .collect();

    for (key, val) in &env_vars {
        lines.push(format!("{key}={val}"));
    }

    let mut updated = lines.join("\n");
    if !updated.ends_with('\n') {
        updated.push('\n');
    }

    std::fs::write(&secrets_path, updated)
        .map_err(|e| format!("Failed to write secrets.env: {e}"))?;

    Ok(secrets_path)
}

fn update_chatgpt_config(home: &std::path::Path, best_model: &str) -> Result<(), String> {
    let config_path = home.join("config.toml");
    let config_str = std::fs::read_to_string(&config_path).unwrap_or_default();
    let mut doc = if config_str.trim().is_empty() {
        toml_edit::DocumentMut::new()
    } else {
        config_str
            .parse::<toml_edit::DocumentMut>()
            .map_err(|e| format!("Failed to parse config.toml: {e}"))?
    };

    let dm = doc
        .entry("default_model")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .ok_or("default_model is not a table")?;
    dm.insert("provider", toml_edit::value("chatgpt"));
    dm.insert("api_key_env", toml_edit::value("CHATGPT_SESSION_TOKEN"));
    dm.insert("model", toml_edit::value(best_model));
    dm.insert(
        "base_url",
        toml_edit::value(librefang_runtime::chatgpt_oauth::CHATGPT_BASE_URL),
    );

    std::fs::write(&config_path, doc.to_string())
        .map_err(|e| format!("Failed to write config.toml: {e}"))?;

    Ok(())
}

fn cmd_auth_chatgpt(device_auth: bool) {
    println!("Starting ChatGPT authentication flow...\n");

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");

    let result: Result<(), String> = rt.block_on(async {
        let auth_result = authenticate_chatgpt(device_auth).await?;
        persist_chatgpt_auth(auth_result).await
    });

    match result {
        Ok(()) => ui::success("ChatGPT authentication complete."),
        Err(e) => {
            ui::error(&format!("ChatGPT authentication failed: {e}"));
            std::process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Vault commands (librefang vault init/set/list/remove)
// ---------------------------------------------------------------------------

fn cmd_vault_init() {
    let home = librefang_home();
    let vault_path = home.join("vault.enc");
    let mut vault = librefang_extensions::vault::CredentialVault::new(vault_path);

    match vault.init() {
        Ok(()) => ui::success(&i18n::t("vault-initialized")),
        Err(e) => {
            ui::error(&e.to_string());
            std::process::exit(1);
        }
    }
}

fn cmd_vault_set(key: &str) {
    use zeroize::Zeroizing;

    let home = librefang_home();
    let vault_path = home.join("vault.enc");
    let mut vault = librefang_extensions::vault::CredentialVault::new(vault_path);

    if !vault.exists() {
        ui::error(&i18n::t("vault-not-init-run"));
        std::process::exit(1);
    }

    if let Err(e) = vault.unlock() {
        ui::error(&i18n::t_args(
            "vault-unlock-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    }

    let value = prompt_input(&format!("Enter value for {key}: "));
    if value.is_empty() {
        ui::error(&i18n::t("vault-empty-value"));
        std::process::exit(1);
    }

    match vault.set(key.to_string(), Zeroizing::new(value)) {
        Ok(()) => ui::success(&i18n::t_args("vault-stored", &[("key", key)])),
        Err(e) => {
            ui::error(&i18n::t_args(
                "vault-store-failed",
                &[("error", &e.to_string())],
            ));
            std::process::exit(1);
        }
    }
}

fn cmd_vault_list() {
    let home = librefang_home();
    let vault_path = home.join("vault.enc");
    let mut vault = librefang_extensions::vault::CredentialVault::new(vault_path);

    if !vault.exists() {
        println!("{}", i18n::t("vault-not-init-run"));
        return;
    }

    if let Err(e) = vault.unlock() {
        ui::error(&i18n::t_args(
            "vault-unlock-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    }

    let keys = vault.list_keys();
    if keys.is_empty() {
        println!("Vault is empty.");
    } else {
        println!("Stored credentials ({}):", keys.len());
        for key in keys {
            println!("  {key}");
        }
    }
}

fn cmd_vault_remove(key: &str) {
    let home = librefang_home();
    let vault_path = home.join("vault.enc");
    let mut vault = librefang_extensions::vault::CredentialVault::new(vault_path);

    if !vault.exists() {
        ui::error(&i18n::t("vault-not-initialized"));
        std::process::exit(1);
    }
    if let Err(e) = vault.unlock() {
        ui::error(&i18n::t_args(
            "vault-unlock-failed",
            &[("error", &e.to_string())],
        ));
        std::process::exit(1);
    }

    match vault.remove(key) {
        Ok(true) => ui::success(&i18n::t_args("vault-removed", &[("key", key)])),
        Ok(false) => println!("{}", i18n::t_args("vault-key-not-found", &[("key", key)])),
        Err(e) => {
            ui::error(&i18n::t_args(
                "vault-remove-failed",
                &[("error", &e.to_string())],
            ));
            std::process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// hash-password command
// ---------------------------------------------------------------------------

fn cmd_hash_password(password: Option<String>) {
    let pass = match password {
        Some(p) => p,
        None => {
            let p1 = prompt_input("Enter password: ");
            if p1.is_empty() {
                ui::error("Password cannot be empty.");
                std::process::exit(1);
            }
            let p2 = prompt_input("Confirm password: ");
            if p1 != p2 {
                ui::error("Passwords do not match.");
                std::process::exit(1);
            }
            p1
        }
    };

    match librefang_api::password_hash::hash_password(&pass) {
        Ok(hash) => {
            println!("\n{hash}\n");
            println!("Add to config.toml:");
            println!("  dashboard_pass_hash = \"{hash}\"");
        }
        Err(e) => {
            ui::error(&format!("Failed to hash password: {e}"));
            std::process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Scaffold commands (librefang new skill/integration)
// ---------------------------------------------------------------------------

fn cmd_scaffold(kind: ScaffoldKind) {
    let cwd = std::env::current_dir().unwrap_or_default();
    let result = match kind {
        ScaffoldKind::Skill => {
            librefang_extensions::installer::scaffold_skill(&cwd.join("my-skill"))
        }
        ScaffoldKind::Mcp => {
            librefang_extensions::installer::scaffold_integration(&cwd.join("my-mcp"))
        }
    };
    match result {
        Ok(msg) => ui::success(&msg),
        Err(e) => {
            ui::error(&e.to_string());
            std::process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// New command handlers
// ---------------------------------------------------------------------------

fn cmd_models_list(provider_filter: Option<&str>, json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let url = match provider_filter {
            Some(p) => format!("{base}/api/models?provider={p}"),
            None => format!("{base}/api/models"),
        };
        let body = daemon_json(client.get(&url).send());
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
            return;
        }
        if let Some(arr) = body
            .get("models")
            .and_then(|v| v.as_array())
            .or_else(|| body.as_array())
        {
            if arr.is_empty() {
                println!("No models found.");
                return;
            }
            println!("{:<40} {:<16} {:<8} CONTEXT", "MODEL", "PROVIDER", "TIER");
            println!("{}", "-".repeat(80));
            for m in arr {
                println!(
                    "{:<40} {:<16} {:<8} {}",
                    m["id"].as_str().unwrap_or("?"),
                    m["provider"].as_str().unwrap_or("?"),
                    m["tier"].as_str().unwrap_or("?"),
                    m["context_window"].as_u64().unwrap_or(0),
                );
            }
        } else {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
        }
    } else {
        // Standalone: use ModelCatalog directly
        let catalog = librefang_runtime::model_catalog::ModelCatalog::default();
        let models = catalog.list_models();
        if json {
            let arr: Vec<serde_json::Value> = models
                .iter()
                .filter(|m| provider_filter.is_none_or(|p| m.provider == p))
                .map(|m| {
                    serde_json::json!({
                        "id": m.id,
                        "provider": m.provider,
                        "tier": format!("{:?}", m.tier),
                        "context_window": m.context_window,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&arr).unwrap_or_default());
            return;
        }
        if models.is_empty() {
            println!("No models in catalog.");
            return;
        }
        println!("{:<40} {:<16} {:<8} CONTEXT", "MODEL", "PROVIDER", "TIER");
        println!("{}", "-".repeat(80));
        for m in models {
            if let Some(p) = provider_filter {
                if m.provider != p {
                    continue;
                }
            }
            println!(
                "{:<40} {:<16} {:<8} {}",
                m.id,
                m.provider,
                format!("{:?}", m.tier),
                m.context_window,
            );
        }
    }
}

fn cmd_models_aliases(json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(client.get(format!("{base}/api/models/aliases")).send());
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
            return;
        }
        if let Some(arr) = body.get("aliases").and_then(|v| v.as_array()) {
            println!("{:<30} RESOLVES TO", "ALIAS");
            println!("{}", "-".repeat(60));
            for entry in arr {
                println!(
                    "{:<30} {}",
                    entry["alias"].as_str().unwrap_or("?"),
                    entry["model_id"].as_str().unwrap_or("?"),
                );
            }
        } else if let Some(obj) = body.as_object() {
            // Fallback for plain {alias: model_id} format
            println!("{:<30} RESOLVES TO", "ALIAS");
            println!("{}", "-".repeat(60));
            for (alias, target) in obj {
                println!("{:<30} {}", alias, target.as_str().unwrap_or("?"));
            }
        } else {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
        }
    } else {
        let catalog = librefang_runtime::model_catalog::ModelCatalog::default();
        let aliases = catalog.list_aliases();
        if json {
            let obj: serde_json::Map<String, serde_json::Value> = aliases
                .iter()
                .map(|(a, t)| (a.to_string(), serde_json::Value::String(t.to_string())))
                .collect();
            println!("{}", serde_json::to_string_pretty(&obj).unwrap_or_default());
            return;
        }
        println!("{:<30} RESOLVES TO", "ALIAS");
        println!("{}", "-".repeat(60));
        for (alias, target) in aliases {
            println!("{:<30} {}", alias, target);
        }
    }
}

fn cmd_models_providers(json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(client.get(format!("{base}/api/providers")).send());
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
            return;
        }
        if let Some(arr) = body
            .get("providers")
            .and_then(|v| v.as_array())
            .or_else(|| body.as_array())
        {
            println!(
                "{:<20} {:<12} {:<10} BASE URL",
                "PROVIDER", "AUTH", "MODELS"
            );
            println!("{}", "-".repeat(70));
            for p in arr {
                println!(
                    "{:<20} {:<12} {:<10} {}",
                    p["id"].as_str().unwrap_or("?"),
                    p["auth_status"].as_str().unwrap_or("?"),
                    p["model_count"].as_u64().unwrap_or(0),
                    p["base_url"].as_str().unwrap_or(""),
                );
            }
        } else {
            println!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_default()
            );
        }
    } else {
        let catalog = librefang_runtime::model_catalog::ModelCatalog::default();
        let providers = catalog.list_providers();
        if json {
            let arr: Vec<serde_json::Value> = providers
                .iter()
                .map(|p| {
                    serde_json::json!({
                        "id": p.id,
                        "auth_status": format!("{:?}", p.auth_status),
                        "model_count": p.model_count,
                        "base_url": p.base_url,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&arr).unwrap_or_default());
            return;
        }
        println!(
            "{:<20} {:<12} {:<10} BASE URL",
            "PROVIDER", "AUTH", "MODELS"
        );
        println!("{}", "-".repeat(70));
        for p in providers {
            println!(
                "{:<20} {:<12} {:<10} {}",
                p.id,
                format!("{:?}", p.auth_status),
                p.model_count,
                p.base_url,
            );
        }
    }
}

fn cmd_models_set(model: Option<String>) {
    let model = match model {
        Some(m) => m,
        None => pick_model(),
    };
    let base = require_daemon("models set");
    let client = daemon_client();
    // Use the config set approach through the API
    let body = daemon_json(
        client
            .post(format!("{base}/api/config/set"))
            .json(&serde_json::json!({"path": "default_model.model", "value": model}))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&i18n::t_args(
            "model-set-failed",
            &[("error", body["error"].as_str().unwrap_or("?"))],
        ));
    } else {
        ui::success(&i18n::t_args("model-set-success", &[("model", &model)]));
    }
}

/// Interactive model picker — shows numbered list, accepts number or model ID.
fn pick_model() -> String {
    let catalog = librefang_runtime::model_catalog::ModelCatalog::default();
    let models = catalog.list_models();

    if models.is_empty() {
        ui::error(&i18n::t("model-no-catalog"));
        std::process::exit(1);
    }

    // Group by provider for display
    let mut by_provider: std::collections::BTreeMap<
        String,
        Vec<&librefang_types::model_catalog::ModelCatalogEntry>,
    > = std::collections::BTreeMap::new();
    for m in models {
        by_provider.entry(m.provider.clone()).or_default().push(m);
    }

    ui::section(&i18n::t("section-select-model"));
    ui::blank();

    let mut numbered: Vec<&str> = Vec::new();
    let mut idx = 1;
    for (provider, provider_models) in &by_provider {
        println!("  {}:", provider.bold());
        for m in provider_models {
            println!("    {idx:>3}. {:<36} {:?}", m.id, m.tier);
            numbered.push(&m.id);
            idx += 1;
        }
    }
    ui::blank();

    loop {
        let input = prompt_input("  Enter number or model ID: ");
        if input.is_empty() {
            continue;
        }
        // Try as number first
        if let Ok(n) = input.parse::<usize>() {
            if n >= 1 && n <= numbered.len() {
                return numbered[n - 1].to_string();
            }
            ui::error(&i18n::t_args(
                "model-out-of-range",
                &[("max", &numbered.len().to_string())],
            ));
            continue;
        }
        // Accept direct model ID if it exists in catalog
        if models.iter().any(|m| m.id == input) {
            return input;
        }
        // Accept as alias
        if catalog.resolve_alias(&input).is_some() {
            return input;
        }
        // Accept any string (user might know a model not in catalog)
        return input;
    }
}

fn cmd_approvals_list(json: bool) {
    let base = require_daemon("approvals list");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/approvals")).send());
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = body
        .get("approvals")
        .and_then(|v| v.as_array())
        .or_else(|| body.as_array())
    {
        if arr.is_empty() {
            println!("No pending approvals.");
            return;
        }
        println!("{:<38} {:<16} {:<12} REQUEST", "ID", "AGENT", "TYPE");
        println!("{}", "-".repeat(80));
        for a in arr {
            println!(
                "{:<38} {:<16} {:<12} {}",
                a["id"].as_str().unwrap_or("?"),
                a["agent_name"].as_str().unwrap_or("?"),
                a["approval_type"].as_str().unwrap_or("?"),
                a["description"].as_str().unwrap_or(""),
            );
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_approvals_respond(id: &str, approve: bool) {
    let base = require_daemon("approvals");
    let client = daemon_client();
    let endpoint = if approve { "approve" } else { "reject" };
    let body = daemon_json(
        client
            .post(format!("{base}/api/approvals/{id}/{endpoint}"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&i18n::t_args(
            "approval-failed",
            &[
                ("action", endpoint),
                ("error", body["error"].as_str().unwrap_or("?")),
            ],
        ));
    } else {
        ui::success(&i18n::t_args(
            "approval-responded",
            &[("id", id), ("action", endpoint)],
        ));
    }
}

fn cmd_cron_list(json: bool) {
    let base = require_daemon("cron list");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/cron/jobs")).send());
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = body
        .get("jobs")
        .and_then(|v| v.as_array())
        .or_else(|| body.as_array())
    {
        if arr.is_empty() {
            println!("No scheduled jobs.");
            return;
        }
        println!(
            "{:<38} {:<16} {:<20} {:<8} PROMPT",
            "ID", "AGENT", "SCHEDULE", "ENABLED"
        );
        println!("{}", "-".repeat(100));
        for j in arr {
            println!(
                "{:<38} {:<16} {:<20} {:<8} {}",
                j["id"].as_str().unwrap_or("?"),
                j["agent_id"].as_str().unwrap_or("?"),
                j["schedule"]["expr"]
                    .as_str()
                    .or_else(|| j["cron_expr"].as_str())
                    .unwrap_or("?"),
                if j["enabled"].as_bool().unwrap_or(false) {
                    "yes"
                } else {
                    "no"
                },
                j["action"]["message"]
                    .as_str()
                    .or_else(|| j["prompt"].as_str())
                    .unwrap_or("")
                    .chars()
                    .take(40)
                    .collect::<String>(),
            );
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_cron_create(agent: &str, spec: &str, prompt: &str, explicit_name: Option<&str>) {
    let base = require_daemon("cron create");
    let agent = resolve_agent_id(&base, agent);
    let client = daemon_client();

    // Use explicit name if provided, otherwise derive from agent + prompt
    let name = if let Some(n) = explicit_name {
        n.to_string()
    } else {
        let short_prompt: String = prompt
            .split_whitespace()
            .take(4)
            .collect::<Vec<_>>()
            .join("-")
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
            .take(64)
            .collect();
        format!(
            "{}-{}",
            agent,
            if short_prompt.is_empty() {
                "job"
            } else {
                &short_prompt
            }
        )
    };

    let body = daemon_json(
        client
            .post(format!("{base}/api/cron/jobs"))
            .json(&serde_json::json!({
                "agent_id": agent,
                "name": name,
                "schedule": {
                    "kind": "cron",
                    "expr": spec
                },
                "action": {
                    "kind": "agent_turn",
                    "message": prompt
                }
            }))
            .send(),
    );
    if let Some(id) = body["job_id"].as_str().or_else(|| body["id"].as_str()) {
        ui::success(&i18n::t_args("cron-created", &[("id", id)]));
    } else {
        ui::error(&i18n::t_args(
            "cron-create-failed",
            &[("error", body["error"].as_str().unwrap_or("?"))],
        ));
    }
}

fn cmd_cron_delete(id: &str) {
    let base = require_daemon("cron delete");
    let client = daemon_client();
    let body = daemon_json(client.delete(format!("{base}/api/cron/jobs/{id}")).send());
    if body.get("error").is_some() {
        ui::error(&i18n::t_args(
            "cron-delete-failed",
            &[("error", body["error"].as_str().unwrap_or("?"))],
        ));
    } else {
        ui::success(&i18n::t_args("cron-deleted", &[("id", id)]));
    }
}

fn cmd_cron_toggle(id: &str, enable: bool) {
    let base = require_daemon("cron");
    let client = daemon_client();
    let endpoint = if enable { "enable" } else { "disable" };
    let body = daemon_json(
        client
            .post(format!("{base}/api/cron/jobs/{id}/{endpoint}"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&i18n::t_args(
            "cron-toggle-failed",
            &[
                ("action", endpoint),
                ("error", body["error"].as_str().unwrap_or("?")),
            ],
        ));
    } else {
        ui::success(&i18n::t_args(
            "cron-toggled",
            &[("id", id), ("action", endpoint)],
        ));
    }
}

fn cmd_sessions(agent: Option<&str>, json: bool) {
    let base = require_daemon("sessions");
    let client = daemon_client();
    let url = match agent {
        Some(a) => format!("{base}/api/sessions?agent={a}"),
        None => format!("{base}/api/sessions"),
    };
    let body = daemon_json(client.get(&url).send());
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = body
        .get("sessions")
        .and_then(|v| v.as_array())
        .or_else(|| body.as_array())
    {
        if arr.is_empty() {
            println!("No sessions found.");
            return;
        }
        println!("{:<38} {:<16} {:<8} LAST ACTIVE", "ID", "AGENT", "MSGS");
        println!("{}", "-".repeat(80));
        for s in arr {
            println!(
                "{:<38} {:<16} {:<8} {}",
                s["session_id"]
                    .as_str()
                    .or_else(|| s["id"].as_str())
                    .unwrap_or("?"),
                s["agent_id"]
                    .as_str()
                    .map(|id| if id.len() > 16 { &id[..16] } else { id })
                    .unwrap_or(s["agent_name"].as_str().unwrap_or("?")),
                s["message_count"].as_u64().unwrap_or(0),
                s["created_at"]
                    .as_str()
                    .or_else(|| s["last_active"].as_str())
                    .unwrap_or("?"),
            );
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn show_log_file(log_path: &std::path::Path, lines: usize, follow: bool) {
    if !log_path.exists() {
        ui::error_with_fix(
            "Log file not found",
            &format!("Expected at: {}", log_path.display()),
        );
        std::process::exit(1);
    }

    if follow {
        // Use tail -f equivalent
        #[cfg(unix)]
        {
            let _ = std::process::Command::new("tail")
                .args(["-f", "-n", &lines.to_string()])
                .arg(log_path)
                .status();
        }
        #[cfg(windows)]
        {
            // On Windows, read in a loop
            let content = std::fs::read_to_string(log_path).unwrap_or_default();
            let all_lines: Vec<&str> = content.lines().collect();
            let start = all_lines.len().saturating_sub(lines);
            for line in &all_lines[start..] {
                println!("{line}");
            }
            println!("--- Following {} (Ctrl+C to stop) ---", log_path.display());
            let mut last_len = content.len();
            loop {
                std::thread::sleep(std::time::Duration::from_millis(500));
                if let Ok(new_content) = std::fs::read_to_string(log_path) {
                    if new_content.len() > last_len {
                        print!("{}", &new_content[last_len..]);
                        last_len = new_content.len();
                    }
                }
            }
        }
    } else {
        let content = std::fs::read_to_string(log_path).unwrap_or_default();
        let all_lines: Vec<&str> = content.lines().collect();
        let start = all_lines.len().saturating_sub(lines);
        for line in &all_lines[start..] {
            println!("{line}");
        }
    }
}

fn cmd_logs(config: Option<PathBuf>, lines: usize, follow: bool) {
    let daemon = daemon_config_context(config.as_deref());
    let daemon_log = daemon_log_path_for_config(config.as_deref());
    if daemon_log.exists() {
        show_log_file(&daemon_log, lines, follow);
        return;
    }

    let tui_log = match daemon.log_dir.as_deref() {
        Some(dir) => dir.join("tui.log"),
        None => daemon.home_dir.join("logs").join("tui.log"),
    };
    if tui_log.exists() {
        ui::hint(&format!(
            "Daemon log not found; showing TUI log at {}",
            tui_log.display()
        ));
        show_log_file(&tui_log, lines, follow);
        return;
    }

    show_log_file(&daemon_log, lines, follow);
}

fn cmd_health(json: bool) {
    match find_daemon() {
        Some(base) => {
            let client = daemon_client();
            let body = daemon_json(client.get(format!("{base}/api/health")).send());
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&body).unwrap_or_default()
                );
                return;
            }
            ui::success(&i18n::t("health-ok"));
            if let Some(status) = body["status"].as_str() {
                ui::kv(&i18n::t("label-status"), status);
            }
            if let Some(uptime) = body.get("uptime_secs").and_then(|v| v.as_u64()) {
                let hours = uptime / 3600;
                let mins = (uptime % 3600) / 60;
                ui::kv(&i18n::t("label-uptime"), &format!("{hours}h {mins}m"));
            }
        }
        None => {
            if json {
                println!("{}", serde_json::json!({"error": "daemon not running"}));
                std::process::exit(1);
            }
            ui::error(&i18n::t("health-not-running"));
            ui::hint(&i18n::t("hint-start-daemon"));
            std::process::exit(1);
        }
    }
}

fn cmd_security_status(json: bool) {
    let base = require_daemon("security status");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/health/detail")).send());
    if json {
        let data = serde_json::json!({
            "audit_trail": "merkle_hash_chain_sha256",
            "taint_tracking": "information_flow_labels",
            "wasm_sandbox": "dual_metering_fuel_epoch",
            "wire_protocol": "ofp_hmac_sha256_mutual_auth",
            "api_keys": "zeroizing_auto_wipe",
            "manifests": "ed25519_signed",
            "agent_count": body.get("agent_count").and_then(|v| v.as_u64()),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&data).unwrap_or_default()
        );
        return;
    }
    ui::section(&i18n::t("section-security-status"));
    ui::blank();
    ui::kv(&i18n::t("label-audit-trail"), &i18n::t("value-audit-trail"));
    ui::kv(
        &i18n::t("label-taint-tracking"),
        &i18n::t("value-taint-tracking"),
    );
    ui::kv(
        &i18n::t("label-wasm-sandbox"),
        &i18n::t("value-wasm-sandbox"),
    );
    ui::kv(
        &i18n::t("label-wire-protocol"),
        &i18n::t("value-wire-protocol"),
    );
    ui::kv(&i18n::t("label-api-keys"), &i18n::t("value-api-keys"));
    ui::kv(&i18n::t("label-manifests"), &i18n::t("value-manifests"));
    if let Some(agents) = body.get("agent_count").and_then(|v| v.as_u64()) {
        ui::kv(&i18n::t("label-active-agents"), &agents.to_string());
    }
}

fn cmd_security_audit(limit: usize, json: bool) {
    let base = require_daemon("security audit");
    let client = daemon_client();
    let body = daemon_json(
        client
            .get(format!("{base}/api/audit/recent?limit={limit}"))
            .send(),
    );
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = body
        .get("entries")
        .and_then(|v| v.as_array())
        .or_else(|| body.as_array())
    {
        if arr.is_empty() {
            println!("No audit entries.");
            return;
        }
        println!("{:<24} {:<16} {:<12} EVENT", "TIMESTAMP", "AGENT", "TYPE");
        println!("{}", "-".repeat(80));
        for entry in arr {
            println!(
                "{:<24} {:<16} {:<12} {}",
                entry["timestamp"].as_str().unwrap_or("?"),
                entry["agent_id"]
                    .as_str()
                    .map(|id| if id.len() > 16 { &id[..16] } else { id })
                    .unwrap_or(entry["agent_name"].as_str().unwrap_or("?")),
                entry["action"]
                    .as_str()
                    .or_else(|| entry["event_type"].as_str())
                    .unwrap_or("?"),
                entry["detail"]
                    .as_str()
                    .or_else(|| entry["description"].as_str())
                    .unwrap_or(""),
            );
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_security_verify() {
    let base = require_daemon("security verify");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/audit/verify")).send());
    if body["valid"].as_bool().unwrap_or(false) {
        ui::success(&i18n::t("audit-verified"));
    } else {
        ui::error(&i18n::t("audit-failed"));
        if let Some(msg) = body["error"].as_str() {
            ui::hint(msg);
        }
        std::process::exit(1);
    }
}

/// Destructively reset the local audit trail.
///
/// Truncates `audit_entries` in SQLite and removes the anchor file so the
/// next daemon boot seeds a fresh Merkle chain. Refuses to run while the
/// daemon holds the DB (SQLite WAL mode + writer lock) and without
/// `--confirm`.
///
/// Phase 7 of `surrealdb-storage-swap`: gated behind `sqlite-backend`
/// since it talks to `rusqlite` directly. The Surreal-backed counterpart
/// lives in [`cmd_audit_reset_surreal`].
#[cfg(feature = "sqlite-backend")]
fn cmd_audit_reset(config: Option<PathBuf>, confirm: bool) {
    let daemon = daemon_config_context(config.as_deref());
    let kernel_config = load_config(config.as_deref());

    let db_path = kernel_config
        .memory
        .sqlite_path
        .clone()
        .unwrap_or_else(|| kernel_config.data_dir.join("librefang.db"));

    let anchor_path = match kernel_config.audit.anchor_path.as_ref() {
        Some(p) if p.is_absolute() => p.clone(),
        Some(p) => kernel_config.data_dir.join(p),
        None => kernel_config.data_dir.join("audit.anchor"),
    };

    if !confirm {
        ui::error("audit reset is destructive — re-run with `--confirm` to proceed");
        ui::blank();
        println!("  Would:");
        println!(
            "    1. DELETE all rows from `audit_entries` in {}",
            db_path.display()
        );
        println!("    2. Remove anchor file {}", anchor_path.display());
        println!("  The Merkle chain will restart from the next audit event.");
        std::process::exit(1);
    }

    // Refuse if daemon is running — SQLite writer lock would block or corrupt.
    if let Some(base) = find_daemon_in_home(&daemon.home_dir) {
        ui::error_with_fix(
            &format!("daemon is running at {base}; refusing to touch the audit database"),
            "stop the daemon first: `librefang stop`",
        );
        std::process::exit(1);
    }

    if !db_path.exists() {
        ui::error(&format!("database not found at {}", db_path.display()));
        std::process::exit(1);
    }

    let conn = match rusqlite::Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            ui::error(&format!("failed to open {}: {e}", db_path.display()));
            std::process::exit(1);
        }
    };

    let rows_before: i64 = conn
        .query_row("SELECT COUNT(*) FROM audit_entries", [], |r| r.get(0))
        .unwrap_or(0);

    // Remove the anchor FIRST. If the subsequent DB truncation then fails,
    // the next daemon boot sees `read_anchor = None` and re-seeds from the
    // current DB tip — a consistent (if still broken) state the user can
    // retry. The reverse order (DB first, anchor second) would instead
    // leave an empty table alongside a stale anchor, which produces a
    // fresh MISMATCH error the user didn't have before calling reset.
    let anchor_removed = if anchor_path.exists() {
        match std::fs::remove_file(&anchor_path) {
            Ok(()) => true,
            Err(e) => {
                ui::error(&format!(
                    "failed to remove anchor {}: {e}",
                    anchor_path.display()
                ));
                std::process::exit(1);
            }
        }
    } else {
        false
    };

    if let Err(e) = conn.execute("DELETE FROM audit_entries", []) {
        ui::error(&format!("failed to truncate audit_entries: {e}"));
        std::process::exit(1);
    }
    drop(conn);
    // `seq` is `INTEGER PRIMARY KEY` without AUTOINCREMENT, so the next
    // insert after an empty table naturally gets seq = 1. No sqlite_sequence
    // fiddling needed.

    ui::success(&format!(
        "Audit trail reset: removed {rows_before} row(s) from audit_entries{}.",
        if anchor_removed {
            format!(", deleted anchor at {}", anchor_path.display())
        } else {
            " (no anchor file to remove)".to_string()
        }
    ));
    ui::hint("The next daemon boot will seed a fresh Merkle chain from the current tip.");
}

/// Read-only inspection of the SurrealDB-backed audit trail.
///
/// This is the Phase 7 sibling to [`cmd_audit_reset`]: rather than
/// touching SQLite, it opens the configured embedded SurrealDB
/// (`storage.backend = embedded`) read-only, runs `SELECT * FROM
/// audit_entries ORDER BY seq ASC LIMIT $limit`, and prints the rows.
///
/// We deliberately do NOT expose a destructive Surreal "reset" path
/// from the CLI — the Surreal-backed audit log is rebuilt automatically
/// from the migration runner on next daemon boot, and a manual wipe is
/// not part of the documented operator workflow.
#[cfg(feature = "surreal-backend")]
fn cmd_audit_explore_surreal(config: Option<PathBuf>, limit: u32, json: bool) {
    use librefang_storage::{StorageConfig, SurrealConnectionPool};

    let kernel_config = load_config(config.as_deref());
    let storage_cfg: StorageConfig = kernel_config.storage.clone();

    // Mirror the SQLite reset's daemon-running guard so we never race
    // the writer (RocksDB will refuse the lock anyway, but a clear
    // message is friendlier than the lock error).
    let daemon = daemon_config_context(config.as_deref());
    if let Some(base) = find_daemon_in_home(&daemon.home_dir) {
        ui::error_with_fix(
            &format!("daemon is running at {base}; refusing to open the embedded SurrealDB store"),
            "stop the daemon first: `librefang stop`",
        );
        std::process::exit(1);
    }

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            ui::error(&format!("failed to start tokio runtime: {e}"));
            std::process::exit(1);
        }
    };

    let result: Result<Vec<serde_json::Value>, String> = rt.block_on(async move {
        let pool = SurrealConnectionPool::new();
        let session = pool
            .open(&storage_cfg)
            .await
            .map_err(|e| format!("open surreal: {e}"))?;
        // Apply schema migrations so a freshly-created store has the
        // `audit_entries` table even if the daemon never booted there.
        librefang_storage::migrations::apply_pending(
            session.client(),
            librefang_storage::migrations::OPERATIONAL_MIGRATIONS,
        )
        .await
        .map_err(|e| format!("apply schema migrations: {e}"))?;
        session
            .client()
            .query(
                "SELECT seq, timestamp, agent_id, action, detail, outcome, prev_hash, hash \
                    FROM audit_entries ORDER BY seq ASC LIMIT $limit",
            )
            .bind(("limit", i64::from(limit)))
            .await
            .map_err(|e| format!("query audit_entries: {e}"))?
            .take::<Vec<serde_json::Value>>(0)
            .map_err(|e| format!("decode audit_entries: {e}"))
    });

    let rows = match result {
        Ok(r) => r,
        Err(e) => {
            ui::error(&e);
            std::process::exit(1);
        }
    };

    if json {
        match serde_json::to_string_pretty(&rows) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                ui::error(&format!("serialise rows: {e}"));
                std::process::exit(1);
            }
        }
        return;
    }

    if rows.is_empty() {
        ui::hint("No audit entries in the embedded SurrealDB store.");
        return;
    }
    println!("{:<6} {:<25} {:<24} ACTION", "SEQ", "TIMESTAMP", "AGENT");
    println!("{}", "-".repeat(80));
    for row in &rows {
        let seq = row.get("seq").and_then(|v| v.as_i64()).unwrap_or(-1);
        let ts = row.get("timestamp").and_then(|v| v.as_str()).unwrap_or("");
        let agent = row.get("agent_id").and_then(|v| v.as_str()).unwrap_or("");
        let action = row.get("action").and_then(|v| v.as_str()).unwrap_or("");
        println!("{seq:<6} {ts:<25} {agent:<24} {action}");
    }
}

/// `librefang storage migrate --from sqlite --to surreal [--dry-run]`.
///
/// Phase 7 deliverable. Streams the legacy SQLite tables (audit, hook
/// traces, circuit-breaker states, TOTP lockout, agent registry) into
/// the configured embedded or remote SurrealDB instance. Writes a
/// receipt JSON to `<data_dir>/migrations/` on success.
#[cfg(all(feature = "sqlite-backend", feature = "surreal-backend"))]
fn cmd_storage_migrate(config: Option<PathBuf>, dry_run: bool) {
    use librefang_storage::migrate::{migrate_sqlite_to_surreal, plan_sqlite, MigrationOptions};
    use librefang_storage::SurrealConnectionPool;

    let kernel_config = load_config(config.as_deref());
    let sqlite_path = kernel_config
        .memory
        .sqlite_path
        .clone()
        .unwrap_or_else(|| kernel_config.data_dir.join("librefang.db"));

    let daemon = daemon_config_context(config.as_deref());
    if let Some(base) = find_daemon_in_home(&daemon.home_dir) {
        ui::error_with_fix(
            &format!("daemon is running at {base}; refusing to migrate while the writer holds the database"),
            "stop the daemon first: `librefang stop`",
        );
        std::process::exit(1);
    }

    if dry_run {
        match plan_sqlite(&sqlite_path) {
            Ok(plan) => {
                println!("Dry run — sqlite source: {}", sqlite_path.display());
                println!("{:<28} ROWS", "TABLE");
                println!("{}", "-".repeat(40));
                for (table, rows) in &plan.source_rows {
                    println!("{table:<28} {rows}");
                }
                println!("{}", "-".repeat(40));
                println!("{:<28} {}", "TOTAL", plan.total_rows());
            }
            Err(e) => {
                ui::error(&format!("plan failed: {e}"));
                std::process::exit(1);
            }
        }
        return;
    }

    let receipts_dir = kernel_config.data_dir.join("migrations");
    let storage_cfg = kernel_config.storage.clone();

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            ui::error(&format!("failed to start tokio runtime: {e}"));
            std::process::exit(1);
        }
    };

    // Open the Surreal session inside the runtime; the migrator itself
    // is sync (it bridges via `block_in_place`) and must run on a
    // multi-thread runtime.
    let outcome = rt.block_on(async {
        let pool = SurrealConnectionPool::new();
        let session = pool
            .open(&storage_cfg)
            .await
            .map_err(|e| format!("open surreal target: {e}"))?;
        librefang_storage::migrations::apply_pending(
            session.client(),
            librefang_storage::migrations::OPERATIONAL_MIGRATIONS,
        )
        .await
        .map_err(|e| format!("apply schema migrations: {e}"))?;
        let opts = MigrationOptions {
            dry_run: false,
            receipt_dir: Some(receipts_dir.clone()),
        };
        // The migrator drives sync rusqlite calls through
        // `block_in_place`, so `spawn_blocking` keeps the runtime
        // healthy.
        let session_for_blocking = session.clone();
        let sqlite_path_for_blocking = sqlite_path.clone();
        tokio::task::spawn_blocking(move || {
            migrate_sqlite_to_surreal(&sqlite_path_for_blocking, &session_for_blocking, &opts)
        })
        .await
        .map_err(|e| format!("join migration task: {e}"))?
        .map_err(|e| format!("migrate: {e}"))
    });

    match outcome {
        Ok(receipt) => {
            ui::success(&format!(
                "Migrated {} row(s) from {} to {}.",
                receipt.copied.values().sum::<u64>(),
                receipt.source,
                receipt.target,
            ));
            for (table, rows) in &receipt.copied {
                println!("  {table:<28} {rows}");
            }
            if !receipt.errors.is_empty() {
                ui::hint("Some tables reported errors:");
                for (table, err) in &receipt.errors {
                    println!("  {table:<28} {err}");
                }
            }
            ui::hint(&format!(
                "Receipt written under {}/",
                receipts_dir.display()
            ));
        }
        Err(e) => {
            ui::error(&e);
            std::process::exit(1);
        }
    }
}

/// Provision a UAR namespace + least-privilege user on a shared remote SurrealDB,
/// then write `[uar.remote]` (and optionally `share_librefang_storage`) to config.toml.
#[cfg(feature = "surreal-backend")]
#[allow(clippy::too_many_arguments)]
fn cmd_storage_link_uar(
    remote_url: String,
    root_user: String,
    root_pass_env: String,
    namespace: String,
    database: String,
    app_user: String,
    app_pass_env: String,
    also_link_memory: bool,
) {
    use librefang_storage::{RemoteSurrealConfig, StorageBackendKind, StorageConfig};

    // --- resolve credentials from env vars -----------------------------------
    let root_pass = std::env::var(&root_pass_env).unwrap_or_else(|_| {
        ui::error(&format!(
            "env var `{root_pass_env}` not set (required for root credentials)"
        ));
        std::process::exit(1);
    });
    let app_pass = std::env::var(&app_pass_env).unwrap_or_else(|_| {
        ui::error(&format!(
            "env var `{app_pass_env}` not set (required for app user password)"
        ));
        std::process::exit(1);
    });

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap_or_else(|e| {
            ui::error(&format!("tokio runtime: {e}"));
            std::process::exit(1);
        });

    let outcome: Result<(), String> = rt.block_on(async {
        // Open with root credentials using a temporary StorageConfig
        let root_cfg = StorageConfig {
            backend: StorageBackendKind::Remote(RemoteSurrealConfig {
                url: remote_url.clone(),
                namespace: namespace.clone(),
                database: database.clone(),
                username: root_user.clone(),
                password_env: root_pass_env.clone(),
                tls_skip_verify: false,
            }),
            namespace: namespace.clone(),
            database: database.clone(),
            legacy_sqlite_path: None,
        };

        // Set the env var so the pool's credential resolver can find the password.
        // SAFETY: single-threaded current_thread runtime; no concurrent env readers.
        #[allow(unused_unsafe)]
        unsafe { std::env::set_var(&root_pass_env, &root_pass) };

        let pool = librefang_storage::SurrealConnectionPool::new();
        let session = pool
            .open(&root_cfg)
            .await
            .map_err(|e| format!("connect to {remote_url}: {e}"))?;
        let db = session.client();

        ui::step(&format!(
            "Provisioning UAR namespace `{namespace}` on {remote_url} ..."
        ));

        // 1. Namespace
        db.query(format!("DEFINE NAMESPACE IF NOT EXISTS `{namespace}`;"))
            .await
            .map_err(|e| format!("DEFINE NAMESPACE: {e}"))?;

        // 2. Database
        db.query(format!(
            "USE NAMESPACE `{namespace}`; DEFINE DATABASE IF NOT EXISTS `{database}` ON NAMESPACE `{namespace}`;"
        ))
        .await
        .map_err(|e| format!("DEFINE DATABASE: {e}"))?;

        // 3. Application user scoped to the namespace
        let escaped_pass = app_pass.replace('\'', "\\'");
        db.query(format!(
            "USE NAMESPACE `{namespace}`; \
             DEFINE USER IF NOT EXISTS `{app_user}` ON NAMESPACE `{namespace}` \
             PASSWORD '{escaped_pass}' ROLES EDITOR;"
        ))
        .await
        .map_err(|e| format!("DEFINE USER: {e}"))?;

        Ok(())
    });

    if let Err(e) = outcome {
        ui::error(&e);
        std::process::exit(1);
    }

    ui::check_ok(&format!(
        "Provisioned namespace `{namespace}`, database `{database}`, user `{app_user}`"
    ));

    // --- write [uar.remote] to config.toml ----------------------------------
    let home = librefang_home();
    let config_path = home.join("config.toml");

    if !config_path.exists() {
        ui::error_with_fix(
            "config.toml not found",
            "run `librefang init` to initialise",
        );
        std::process::exit(1);
    }

    let raw = std::fs::read_to_string(&config_path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = raw
        .parse()
        .unwrap_or_else(|_| toml_edit::DocumentMut::new());

    // Ensure [uar] table exists
    if !doc.contains_table("uar") {
        doc.insert("uar", toml_edit::Item::Table(toml_edit::Table::new()));
    }
    let uar_tbl = doc["uar"].as_table_mut().expect("uar is a table");

    if also_link_memory {
        uar_tbl.insert("share_librefang_storage", toml_edit::value(true));
    }

    // Build [uar.remote] inline table
    let mut remote_tbl = toml_edit::Table::new();
    remote_tbl.insert("url", toml_edit::value(&remote_url));
    remote_tbl.insert("namespace", toml_edit::value(&namespace));
    remote_tbl.insert("database", toml_edit::value(&database));
    remote_tbl.insert("username", toml_edit::value(&app_user));
    remote_tbl.insert("password_env", toml_edit::value(&app_pass_env));

    uar_tbl.insert("remote", toml_edit::Item::Table(remote_tbl));

    if let Err(e) = std::fs::write(&config_path, doc.to_string()) {
        ui::error(&format!("write config.toml: {e}"));
        std::process::exit(1);
    }

    ui::check_ok("Updated config.toml with [uar.remote]");

    if find_daemon().is_some() {
        ui::check_warn("Daemon is running — restart to activate: `librefang restart`");
    } else {
        ui::hint("Start the daemon to activate: `librefang start`");
    }
}

/// Remove `[uar.remote]` and `share_librefang_storage` from config.toml.
/// Optionally drops the application user from the remote instance.
#[cfg(feature = "surreal-backend")]
fn cmd_storage_unlink_uar(purge_user: bool, root_user: String, root_pass_env: Option<String>) {
    use librefang_storage::{RemoteSurrealConfig, StorageBackendKind, StorageConfig};

    // Read current config to find remote URL + app user so we can REMOVE USER if requested
    let home = librefang_home();
    let config_path = home.join("config.toml");

    if !config_path.exists() {
        ui::error_with_fix(
            "config.toml not found",
            "run `librefang init` to initialise",
        );
        std::process::exit(1);
    }

    let raw = std::fs::read_to_string(&config_path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = raw
        .parse()
        .unwrap_or_else(|_| toml_edit::DocumentMut::new());

    // Extract current remote config before stripping it
    let maybe_remote: Option<(String, String, String, String)> =
        doc.get("uar").and_then(|u| u.get("remote")).and_then(|r| {
            let url = r.get("url")?.as_str()?.to_string();
            let ns = r.get("namespace")?.as_str()?.to_string();
            let user = r.get("username")?.as_str()?.to_string();
            let pass_env = r.get("password_env")?.as_str()?.to_string();
            Some((url, ns, user, pass_env))
        });

    if purge_user {
        let Some((url, ns, app_user, _pass_env)) = maybe_remote.as_ref() else {
            ui::error("no [uar.remote] found in config.toml; cannot purge user");
            std::process::exit(1);
        };
        let Some(ref root_pass_env_name) = root_pass_env else {
            ui::error("--root-pass-env is required when --purge-user is set");
            std::process::exit(1);
        };
        let root_pass = std::env::var(root_pass_env_name).unwrap_or_else(|_| {
            ui::error(&format!("env var `{root_pass_env_name}` not set"));
            std::process::exit(1);
        });

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap_or_else(|e| {
                ui::error(&format!("tokio runtime: {e}"));
                std::process::exit(1);
            });

        let (url, ns, app_user, root_pass_env_name) = (
            url.clone(),
            ns.clone(),
            app_user.clone(),
            root_pass_env_name.clone(),
        );
        // SAFETY: single-threaded current_thread runtime; no concurrent env readers.
        #[allow(unused_unsafe)]
        unsafe {
            std::env::set_var(&root_pass_env_name, &root_pass)
        };

        let outcome: Result<(), String> = rt.block_on(async {
            let root_cfg = StorageConfig {
                backend: StorageBackendKind::Remote(RemoteSurrealConfig {
                    url: url.clone(),
                    namespace: ns.clone(),
                    database: "main".to_string(),
                    username: root_user.clone(),
                    password_env: root_pass_env_name.clone(),
                    tls_skip_verify: false,
                }),
                namespace: ns.clone(),
                database: "main".to_string(),
                legacy_sqlite_path: None,
            };
            let pool = librefang_storage::SurrealConnectionPool::new();
            let session = pool
                .open(&root_cfg)
                .await
                .map_err(|e| format!("connect: {e}"))?;

            session
                .client()
                .query(format!(
                    "USE NAMESPACE `{ns}`; REMOVE USER IF EXISTS `{app_user}` ON NAMESPACE `{ns}`;"
                ))
                .await
                .map_err(|e| format!("REMOVE USER: {e}"))?;
            Ok(())
        });

        match outcome {
            Ok(()) => ui::check_ok(&format!("Removed user `{app_user}` from namespace `{ns}`")),
            Err(e) => {
                ui::check_warn(&format!(
                    "Could not drop user (config will still be cleared): {e}"
                ));
            }
        }
    }

    // Strip [uar.remote] and share_librefang_storage
    if let Some(uar_tbl) = doc.get_mut("uar").and_then(|u| u.as_table_mut()) {
        uar_tbl.remove("remote");
        uar_tbl.remove("share_librefang_storage");
        // If the [uar] table is now empty except for well-known scalar keys, leave it alone.
    }

    if let Err(e) = std::fs::write(&config_path, doc.to_string()) {
        ui::error(&format!("write config.toml: {e}"));
        std::process::exit(1);
    }

    ui::check_ok("Removed [uar.remote] from config.toml");

    if find_daemon().is_some() {
        ui::check_warn("Daemon is running — restart to activate: `librefang restart`");
    } else {
        ui::hint("Changes take effect on next daemon start.");
    }
}

fn cmd_memory_list(agent: &str, json: bool) {
    let base = require_daemon("memory list");
    let agent = resolve_agent_id(&base, agent);
    let client = daemon_client();
    let body = daemon_json(
        client
            .get(format!("{base}/api/memory/agents/{agent}/kv"))
            .send(),
    );
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = body
        .get("kv_pairs")
        .and_then(|v| v.as_array())
        .or_else(|| body.as_array())
    {
        if arr.is_empty() {
            println!("No memory entries for agent '{agent}'.");
            return;
        }
        println!("{:<30} VALUE", "KEY");
        println!("{}", "-".repeat(60));
        for kv in arr {
            println!(
                "{:<30} {}",
                kv["key"].as_str().unwrap_or("?"),
                kv["value"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(50)
                    .collect::<String>(),
            );
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_memory_get(agent: &str, key: &str, json: bool) {
    let base = require_daemon("memory get");
    let agent = resolve_agent_id(&base, agent);
    let client = daemon_client();
    let body = daemon_json(
        client
            .get(format!("{base}/api/memory/agents/{agent}/kv/{key}"))
            .send(),
    );
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(val) = body["value"].as_str() {
        println!("{val}");
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_memory_set(agent: &str, key: &str, value: &str) {
    let base = require_daemon("memory set");
    let agent = resolve_agent_id(&base, agent);
    let client = daemon_client();
    let body = daemon_json(
        client
            .put(format!("{base}/api/memory/agents/{agent}/kv/{key}"))
            .json(&serde_json::json!({"value": value}))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&i18n::t_args(
            "memory-set-failed",
            &[("error", body["error"].as_str().unwrap_or("?"))],
        ));
    } else {
        ui::success(&i18n::t_args(
            "memory-set",
            &[("key", key), ("agent", &agent)],
        ));
    }
}

fn cmd_memory_delete(agent: &str, key: &str) {
    let base = require_daemon("memory delete");
    let agent = resolve_agent_id(&base, agent);
    let client = daemon_client();
    let body = daemon_json(
        client
            .delete(format!("{base}/api/memory/agents/{agent}/kv/{key}"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&i18n::t_args(
            "memory-delete-failed",
            &[("error", body["error"].as_str().unwrap_or("?"))],
        ));
    } else {
        ui::success(&i18n::t_args(
            "memory-deleted",
            &[("key", key), ("agent", &agent)],
        ));
    }
}

fn cmd_devices_list(json: bool) {
    let base = require_daemon("devices list");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/pairing/devices")).send());
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = body.as_array() {
        if arr.is_empty() {
            println!("No paired devices.");
            return;
        }
        println!("{:<38} {:<20} LAST SEEN", "ID", "NAME");
        println!("{}", "-".repeat(70));
        for d in arr {
            println!(
                "{:<38} {:<20} {}",
                d["id"].as_str().unwrap_or("?"),
                d["name"].as_str().unwrap_or("?"),
                d["last_seen"].as_str().unwrap_or("?"),
            );
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_devices_pair() {
    let base = require_daemon("qr");
    let client = daemon_client();
    let body = daemon_json(client.post(format!("{base}/api/pairing/request")).send());
    if let Some(qr) = body["qr_data"].as_str() {
        ui::section(&i18n::t("section-device-pairing"));
        ui::blank();
        // Render a simple text-based QR representation
        println!("  {}", i18n::t("device-scan-qr"));
        ui::blank();
        println!("  {qr}");
        ui::blank();
        if let Some(code) = body["pairing_code"].as_str() {
            ui::kv(&i18n::t("label-pairing-code"), code);
        }
        if let Some(expires) = body["expires_at"].as_str() {
            ui::kv(&i18n::t("label-expires"), expires);
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_devices_remove(id: &str) {
    let base = require_daemon("devices remove");
    let client = daemon_client();
    let body = daemon_json(
        client
            .delete(format!("{base}/api/pairing/devices/{id}"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&i18n::t_args(
            "device-remove-failed",
            &[("error", body["error"].as_str().unwrap_or("?"))],
        ));
    } else {
        ui::success(&i18n::t_args("device-removed", &[("id", id)]));
    }
}

fn cmd_webhooks_list(json: bool) {
    let base = require_daemon("webhooks list");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/webhooks")).send());
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = body
        .get("webhooks")
        .and_then(|v| v.as_array())
        .or_else(|| body.as_array())
    {
        if arr.is_empty() {
            println!("No webhooks configured.");
            return;
        }
        println!("{:<38} {:<20} {:<10} URL", "ID", "NAME", "ENABLED");
        println!("{}", "-".repeat(90));
        for w in arr {
            println!(
                "{:<38} {:<20} {:<10} {}",
                w["id"].as_str().unwrap_or("?"),
                w["name"].as_str().unwrap_or("?"),
                if w["enabled"].as_bool().unwrap_or(false) {
                    "yes"
                } else {
                    "no"
                },
                w["url"].as_str().unwrap_or(""),
            );
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_webhooks_create(agent: &str, url: &str) {
    let base = require_daemon("webhooks create");
    let agent = resolve_agent_id(&base, agent);
    let client = daemon_client();

    // Derive a name from the URL hostname
    let name = reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_else(|| "webhook".to_string());

    let body = daemon_json(
        client
            .post(format!("{base}/api/webhooks"))
            .json(&serde_json::json!({
                "name": format!("{agent}-{name}"),
                "url": url,
                "events": ["all"],
            }))
            .send(),
    );
    if let Some(id) = body["id"].as_str() {
        ui::success(&i18n::t_args("webhook-created", &[("id", id)]));
    } else {
        ui::error(&i18n::t_args(
            "webhook-create-failed",
            &[("error", body["error"].as_str().unwrap_or("?"))],
        ));
    }
}

fn cmd_webhooks_delete(id: &str) {
    let base = require_daemon("webhooks delete");
    let client = daemon_client();
    let body = daemon_json(client.delete(format!("{base}/api/webhooks/{id}")).send());
    if body.get("error").is_some() {
        ui::error(&i18n::t_args(
            "webhook-delete-failed",
            &[("error", body["error"].as_str().unwrap_or("?"))],
        ));
    } else {
        ui::success(&i18n::t_args("webhook-deleted", &[("id", id)]));
    }
}

fn cmd_webhooks_test(id: &str) {
    let base = require_daemon("webhooks test");
    let client = daemon_client();
    let body = daemon_json(client.post(format!("{base}/api/webhooks/{id}/test")).send());
    if body["success"].as_bool().unwrap_or(false) {
        ui::success(&i18n::t_args("webhook-test-ok", &[("id", id)]));
    } else {
        ui::error(&i18n::t_args(
            "webhook-test-failed",
            &[("error", body["error"].as_str().unwrap_or("?"))],
        ));
    }
}

/// Resolve an agent name-or-id to a UUID by querying the daemon.
fn resolve_agent_id(base: &str, name_or_id: &str) -> String {
    if uuid::Uuid::try_parse(name_or_id).is_ok() {
        return name_or_id.to_string();
    }
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/agents")).send());
    let agents = body
        .get("items")
        .and_then(|v| v.as_array())
        .or_else(|| body.as_array());
    if let Some(arr) = agents {
        if let Some(agent) = arr.iter().find(|a| a["name"].as_str() == Some(name_or_id)) {
            if let Some(id) = agent["id"].as_str() {
                return id.to_string();
            }
        }
    }
    name_or_id.to_string()
}

fn cmd_message(agent: &str, text: &str, json: bool) {
    let base = require_daemon("message");
    let agent_id = resolve_agent_id(&base, agent);
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/agents/{agent_id}/message"))
            .json(&serde_json::json!({"message": text}))
            .send(),
    );
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    } else if let Some(reply) = body["reply"].as_str() {
        println!("{reply}");
    } else if let Some(reply) = body["response"].as_str() {
        println!("{reply}");
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn cmd_system_info(json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(client.get(format!("{base}/api/status")).send());
        if json {
            let mut data = body.clone();
            if let Some(obj) = data.as_object_mut() {
                obj.insert(
                    "version".to_string(),
                    serde_json::json!(env!("CARGO_PKG_VERSION")),
                );
                obj.insert("api_url".to_string(), serde_json::json!(base));
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&data).unwrap_or_default()
            );
            return;
        }
        ui::section(&i18n::t("section-system-info"));
        ui::blank();
        ui::kv(&i18n::t("label-version"), env!("CARGO_PKG_VERSION"));
        ui::kv(
            &i18n::t("label-status"),
            body["status"].as_str().unwrap_or("?"),
        );
        ui::kv(
            &i18n::t("label-agents"),
            &body["agent_count"].as_u64().unwrap_or(0).to_string(),
        );
        ui::kv(
            &i18n::t("label-provider"),
            body["default_provider"].as_str().unwrap_or("?"),
        );
        ui::kv(
            &i18n::t("label-model"),
            body["default_model"].as_str().unwrap_or("?"),
        );
        ui::kv(&i18n::t("label-api"), &base);
        ui::kv(
            &i18n::t("label-data-dir"),
            body["data_dir"].as_str().unwrap_or("?"),
        );
        ui::kv(
            &i18n::t("label-uptime"),
            &format!("{}s", body["uptime_seconds"].as_u64().unwrap_or(0)),
        );
    } else {
        if json {
            println!(
                "{}",
                serde_json::json!({
                    "version": env!("CARGO_PKG_VERSION"),
                    "daemon": "not_running",
                })
            );
            return;
        }
        ui::section(&i18n::t("section-system-info"));
        ui::blank();
        ui::kv(&i18n::t("label-version"), env!("CARGO_PKG_VERSION"));
        ui::kv_warn(
            &i18n::t("label-daemon"),
            &i18n::t("label-daemon-not-running"),
        );
        ui::hint(&i18n::t("hint-start-daemon"));
    }
}

fn cmd_system_version(json: bool) {
    if json {
        println!(
            "{}",
            serde_json::json!({"version": env!("CARGO_PKG_VERSION")})
        );
        return;
    }
    println!("librefang {}", env!("CARGO_PKG_VERSION"));
}

// ---------------------------------------------------------------------------
// Service management (boot auto-start)
// ---------------------------------------------------------------------------

/// Resolve the absolute path to the current librefang binary.
fn resolve_binary_path() -> std::path::PathBuf {
    std::env::current_exe()
        .unwrap_or_else(|_| std::path::PathBuf::from("librefang"))
        .canonicalize()
        .unwrap_or_else(|_| std::env::current_exe().unwrap_or_else(|_| "librefang".into()))
}

fn cmd_service_install() {
    // Warn if running as root — the service would be installed for root, not
    // the actual user. This catches `sudo librefang service install` mistakes.
    #[cfg(unix)]
    {
        // SAFETY: geteuid() is always safe to call.
        if unsafe { libc::geteuid() } == 0 {
            ui::error(
                "Running as root — the service will be installed for the root account, \
                 not your user. Run without sudo instead.",
            );
            std::process::exit(1);
        }
    }

    let binary = resolve_binary_path();

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    let librefang_home = cli_librefang_home();

    #[cfg(target_os = "linux")]
    {
        service_install_linux(&binary, &librefang_home);
    }
    #[cfg(target_os = "macos")]
    {
        service_install_macos(&binary, &librefang_home);
    }
    #[cfg(windows)]
    {
        service_install_windows(&binary);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = &binary;
        ui::error("Auto-start service is not supported on this platform.");
    }
}

#[cfg(target_os = "linux")]
fn service_install_linux(binary: &std::path::Path, librefang_home: &std::path::Path) {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => {
            ui::error("Cannot determine home directory.");
            return;
        }
    };
    let service_dir = home.join(".config/systemd/user");
    if let Err(e) = std::fs::create_dir_all(&service_dir) {
        ui::error(&format!("Failed to create {}: {e}", service_dir.display()));
        return;
    }

    let unit = format!(
        "[Unit]\n\
         Description=LibreFang Agent OS Daemon\n\
         Documentation=https://librefang.ai\n\
         After=network-online.target\n\
         Wants=network-online.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={binary} start --foreground\n\
         Restart=on-failure\n\
         RestartSec=5\n\
         WorkingDirectory={home}\n\
         EnvironmentFile=-{home}/env\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        binary = binary.display(),
        home = librefang_home.display(),
    );

    let service_path = service_dir.join("librefang.service");
    if let Err(e) = std::fs::write(&service_path, &unit) {
        ui::error(&format!("Failed to write {}: {e}", service_path.display()));
        return;
    }
    ui::success(&format!("Wrote {}", service_path.display()));

    // Reload and enable
    let reload = std::process::Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .output();
    if let Ok(o) = &reload {
        if !o.status.success() {
            ui::error("systemctl --user daemon-reload failed");
            return;
        }
    }
    let enable = std::process::Command::new("systemctl")
        .args(["--user", "enable", "librefang.service"])
        .output();
    match enable {
        Ok(o) if o.status.success() => {
            ui::success("Service enabled (will start on next login)");
            ui::hint("Start now with: systemctl --user start librefang.service");
            // Enable lingering so the user service runs without an active login session
            ui::hint("For headless servers, also run: loginctl enable-linger");
        }
        _ => ui::error("systemctl --user enable librefang.service failed"),
    }
}

#[cfg(target_os = "macos")]
fn service_install_macos(binary: &std::path::Path, librefang_home: &std::path::Path) {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => {
            ui::error("Cannot determine home directory.");
            return;
        }
    };
    let agents_dir = home.join("Library/LaunchAgents");
    if let Err(e) = std::fs::create_dir_all(&agents_dir) {
        ui::error(&format!("Failed to create {}: {e}", agents_dir.display()));
        return;
    }

    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>ai.librefang.daemon</string>
    <key>ProgramArguments</key>
    <array>
        <string>{binary}</string>
        <string>start</string>
        <string>--foreground</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>WorkingDirectory</key>
    <string>{home}</string>
    <key>StandardOutPath</key>
    <string>{home}/daemon.log</string>
    <key>StandardErrorPath</key>
    <string>{home}/daemon.log</string>
</dict>
</plist>
"#,
        binary = binary.display(),
        home = librefang_home.display(),
    );

    let plist_path = agents_dir.join("ai.librefang.daemon.plist");

    // Unload existing service first (if any) to avoid launchctl errors
    if plist_path.exists() {
        let _ = std::process::Command::new("launchctl")
            .args(["unload", &plist_path.to_string_lossy()])
            .output();
    }

    if let Err(e) = std::fs::write(&plist_path, &plist) {
        ui::error(&format!("Failed to write {}: {e}", plist_path.display()));
        return;
    }
    ui::success(&format!("Wrote {}", plist_path.display()));

    let load = std::process::Command::new("launchctl")
        .args(["load", &plist_path.to_string_lossy()])
        .output();
    match load {
        Ok(o) if o.status.success() => {
            ui::success("LaunchAgent loaded (will start on login and now)");
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            ui::error(&format!("launchctl load failed: {stderr}"));
        }
        Err(e) => ui::error(&format!("Failed to run launchctl: {e}")),
    }
}

#[cfg(windows)]
fn service_install_windows(binary: &std::path::Path) {
    let value = format!("\"{}\" start", binary.display());
    let output = std::process::Command::new("reg")
        .args([
            "add",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
            "/v",
            "LibreFang",
            "/t",
            "REG_SZ",
            "/d",
            &value,
            "/f",
        ])
        .output();
    match output {
        Ok(o) if o.status.success() => {
            ui::success("Added to Windows startup (HKCU\\...\\Run)");
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            ui::error(&format!("Failed to write registry: {stderr}"));
        }
        Err(e) => ui::error(&format!("Failed to run reg.exe: {e}")),
    }
}

fn cmd_service_uninstall() {
    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir().unwrap_or_default();
        let service_path = home.join(".config/systemd/user/librefang.service");
        if service_path.exists() {
            let _ = std::process::Command::new("systemctl")
                .args(["--user", "disable", "--now", "librefang.service"])
                .output();
            match std::fs::remove_file(&service_path) {
                Ok(()) => {
                    let _ = std::process::Command::new("systemctl")
                        .args(["--user", "daemon-reload"])
                        .output();
                    ui::success("Removed systemd user service");
                }
                Err(e) => ui::error(&format!("Failed to remove service file: {e}")),
            }
        } else {
            ui::hint("No systemd user service found — nothing to remove.");
        }
    }
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().unwrap_or_default();
        let plist_path = home.join("Library/LaunchAgents/ai.librefang.daemon.plist");
        if plist_path.exists() {
            let _ = std::process::Command::new("launchctl")
                .args(["unload", &plist_path.to_string_lossy()])
                .output();
            match std::fs::remove_file(&plist_path) {
                Ok(()) => ui::success("Removed LaunchAgent"),
                Err(e) => ui::error(&format!("Failed to remove plist: {e}")),
            }
        } else {
            ui::hint("No LaunchAgent found — nothing to remove.");
        }
    }
    #[cfg(windows)]
    {
        let output = std::process::Command::new("reg")
            .args([
                "delete",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                "/v",
                "LibreFang",
                "/f",
            ])
            .output();
        match output {
            Ok(o) if o.status.success() => {
                ui::success("Removed from Windows startup");
            }
            _ => ui::hint("No startup entry found — nothing to remove."),
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        ui::error("Auto-start service is not supported on this platform.");
    }
}

fn cmd_service_status() {
    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir().unwrap_or_default();
        let service_path = home.join(".config/systemd/user/librefang.service");
        if service_path.exists() {
            ui::success("Systemd user service is registered");
            // Show enabled/active status
            if let Ok(output) = std::process::Command::new("systemctl")
                .args(["--user", "is-enabled", "librefang.service"])
                .output()
            {
                let status = String::from_utf8_lossy(&output.stdout).trim().to_string();
                ui::kv("  Enabled", &status);
            }
            if let Ok(output) = std::process::Command::new("systemctl")
                .args(["--user", "is-active", "librefang.service"])
                .output()
            {
                let status = String::from_utf8_lossy(&output.stdout).trim().to_string();
                ui::kv("  Active", &status);
            }
        } else {
            ui::hint("No systemd user service registered.");
            ui::hint("Run `librefang service install` to set it up.");
        }
    }
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().unwrap_or_default();
        let plist_path = home.join("Library/LaunchAgents/ai.librefang.daemon.plist");
        if plist_path.exists() {
            ui::success("LaunchAgent is registered");
            if let Ok(output) = std::process::Command::new("launchctl")
                .args(["list"])
                .output()
            {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let running = stdout.lines().any(|l| l.contains("ai.librefang.daemon"));
                ui::kv("  Loaded", if running { "yes" } else { "not loaded" });
            }
        } else {
            ui::hint("No LaunchAgent registered.");
            ui::hint("Run `librefang service install` to set it up.");
        }
    }
    #[cfg(windows)]
    {
        let output = std::process::Command::new("reg")
            .args([
                "query",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                "/v",
                "LibreFang",
            ])
            .output();
        match output {
            Ok(o) if o.status.success() => {
                ui::success("Windows startup entry is registered");
            }
            _ => {
                ui::hint("No startup entry registered.");
                ui::hint("Run `librefang service install` to set it up.");
            }
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        ui::error("Auto-start service is not supported on this platform.");
    }
}

fn cmd_reset(confirm: bool) {
    let librefang_dir = cli_librefang_home();

    if !librefang_dir.exists() {
        println!(
            "Nothing to reset — {} does not exist.",
            librefang_dir.display()
        );
        return;
    }

    if !confirm {
        println!("  This will delete all data in {}", librefang_dir.display());
        println!("  Including: config, database, agent manifests, credentials.");
        println!();
        let answer = prompt_input("  Are you sure? Type 'yes' to confirm: ");
        if answer.trim() != "yes" {
            println!("  Cancelled.");
            return;
        }
    }

    match std::fs::remove_dir_all(&librefang_dir) {
        Ok(()) => ui::success(&i18n::t_args(
            "reset-success",
            &[("path", &librefang_dir.display().to_string())],
        )),
        Err(e) => {
            ui::error(&i18n::t_args(
                "reset-fail",
                &[
                    ("path", &librefang_dir.display().to_string()),
                    ("error", &e.to_string()),
                ],
            ));
            std::process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

const RELEASE_REPO: &str = "librefang/librefang";
const RELEASES_LATEST_API: &str =
    "https://api.github.com/repos/librefang/librefang/releases/latest";
const RELEASES_API: &str = "https://api.github.com/repos/librefang/librefang/releases";
const SHELL_INSTALLER_URL: &str = "https://librefang.ai/install.sh";
const POWERSHELL_INSTALLER_URL: &str = "https://librefang.ai/install.ps1";

enum UpdateLaunch {
    #[cfg(not(windows))]
    Completed,
    #[cfg(windows)]
    Detached,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReleaseComparison {
    Newer,
    SameCore,
    Older,
    Unknown,
}

fn cmd_update(check: bool, version: Option<String>, channel_override: Option<String>) {
    use librefang_types::config::UpdateChannel;

    let current_exe = std::env::current_exe().unwrap_or_else(|e| {
        ui::error(&format!("Cannot determine current executable path: {e}"));
        std::process::exit(1);
    });

    let current_version = env!("CARGO_PKG_VERSION");
    let current_exe_display = current_exe.display().to_string();
    let requested_version = version.as_deref();

    // Resolve update channel: CLI arg > config.toml > default (stable)
    let channel = if let Some(ref ch) = channel_override {
        match ch.parse::<UpdateChannel>() {
            Ok(c) => c,
            Err(e) => {
                ui::error(&e);
                std::process::exit(1);
            }
        }
    } else {
        load_update_channel_from_config().unwrap_or_default()
    };

    ui::section("Update");
    ui::kv("Current", current_version);
    ui::kv("Channel", &channel.to_string());
    ui::kv("Binary", &current_exe_display);

    let latest_tag = if requested_version.is_none() {
        match fetch_latest_release_tag(channel) {
            Ok(tag) => {
                ui::kv("Latest", &tag);
                Some(tag)
            }
            Err(err) => {
                if check {
                    ui::error(&format!("Failed to check latest release: {err}"));
                    std::process::exit(1);
                }
                ui::warn_with_fix(
                    &format!("Could not resolve the latest published release: {err}"),
                    "Retry later, or pass `--version <tag>` to target a specific release.",
                );
                None
            }
        }
    } else {
        if let Some(target) = requested_version {
            ui::kv("Target", target);
        }
        None
    };
    let target_tag = requested_version
        .map(str::to_owned)
        .or_else(|| latest_tag.clone());
    let target_comparison = target_tag
        .as_deref()
        .map(|tag| compare_release_tag(tag, current_version));

    if check {
        match (target_tag.as_deref(), target_comparison) {
            (Some(tag), Some(ReleaseComparison::Newer)) => {
                ui::warn_with_fix(
                    &format!("A newer published release is available: {tag}"),
                    "Run `librefang update` to install it.",
                );
            }
            (Some(tag), Some(ReleaseComparison::SameCore)) => {
                ui::warn_with_fix(
                    &format!(
                        "The published release {tag} uses the same CLI version core as the current binary ({current_version})."
                    ),
                    "Run `librefang update` if you want the latest published build for this version line.",
                );
            }
            (Some(tag), Some(ReleaseComparison::Older)) => {
                ui::success(&format!(
                    "Current binary version {current_version} is ahead of the published release {tag}."
                ));
            }
            (Some(tag), Some(ReleaseComparison::Unknown)) => {
                ui::warn_with_fix(
                    &format!("Could not compare the current binary with release tag {tag}."),
                    "If you want that exact release, run `librefang update --version <tag>`.",
                );
            }
            _ => {
                ui::warn_with_fix(
                    "Unable to determine whether an update is available.",
                    "Retry later when GitHub Releases is reachable.",
                );
            }
        }
        return;
    }

    if requested_version.is_none() {
        match (latest_tag.as_deref(), target_comparison) {
            (Some(tag), Some(ReleaseComparison::Older)) => {
                ui::success(&format!(
                    "Current binary version {current_version} is ahead of the latest published release {tag}."
                ));
                return;
            }
            (Some(tag), Some(ReleaseComparison::Unknown)) => {
                ui::warn_with_fix(
                    &format!(
                        "Could not safely compare the current binary against release tag {tag}."
                    ),
                    &format!(
                        "Re-run with `librefang update --version {tag}` to install it explicitly."
                    ),
                );
                return;
            }
            _ => {}
        }
    }

    let default_install = default_install_executable();
    let cargo_install = cargo_install_executable();
    let target_version = target_tag.as_deref();

    #[cfg(windows)]
    if same_path(&current_exe, &default_install) && find_daemon().is_some() {
        ui::error_with_fix(
            "Stop the running daemon before updating on Windows.",
            "Run `librefang stop`, then `librefang update`, then `librefang start`.",
        );
        std::process::exit(1);
    }

    if same_path(&current_exe, &default_install) {
        match run_official_update(target_version) {
            #[cfg(not(windows))]
            Ok(UpdateLaunch::Completed) => {
                ui::success("LibreFang CLI updated.");
                if let Some(installed) = installed_binary_version(&default_install) {
                    ui::kv("Installed", &installed);
                }
                // Merge any new config defaults added in the updated binary.
                // Spawn the new binary rather than calling cmd_init_upgrade() here,
                // because the current process still holds the old binary's template.
                ui::blank();
                ui::hint("Merging new config defaults...");
                let _ = std::process::Command::new(&default_install)
                    .args(["init", "--upgrade"])
                    .status();
                ui::hint("If the daemon is running, restart it with `librefang restart`.");
            }
            #[cfg(windows)]
            Ok(UpdateLaunch::Detached) => {
                ui::success("Update launched in the background.");
                ui::hint("Open a new terminal after it finishes and run `librefang --version`.");
                ui::hint("If the daemon is running, restart it after the update completes.");
            }
            Err(err) => {
                ui::error(&format!("Update failed: {err}"));
                std::process::exit(1);
            }
        }
        return;
    }

    if same_path(&current_exe, &cargo_install) {
        let cargo_cmd = cargo_update_command(target_version);
        ui::warn_with_fix(
            "This binary was installed with cargo. Running `cargo install` from inside the active executable is intentionally blocked.",
            &cargo_cmd,
        );
        return;
    }

    let official_path = default_install.display().to_string();
    ui::warn_with_fix(
        &format!(
            "Automatic update only supports the official install path ({official_path}). This binary is running from a different location."
        ),
        &manual_installer_command(target_version),
    );
    ui::hint("If this binary came from another package manager, update it with that package manager instead.");
}

fn fetch_latest_release_tag(
    channel: librefang_types::config::UpdateChannel,
) -> Result<String, String> {
    use librefang_types::config::UpdateChannel;

    let client = update_http_client()?;

    match channel {
        UpdateChannel::Stable => {
            // /releases/latest returns the latest non-draft, non-prerelease
            let response = client
                .get(RELEASES_LATEST_API)
                .send()
                .map_err(|e| format!("GitHub request failed: {e}"))?;
            let status = response.status();
            if !status.is_success() {
                return Err(format!("GitHub API returned {status}"));
            }
            let body = response
                .json::<serde_json::Value>()
                .map_err(|e| format!("Failed to decode release metadata: {e}"))?;
            body["tag_name"]
                .as_str()
                .filter(|tag| !tag.is_empty())
                .map(str::to_string)
                .ok_or_else(|| "Release metadata is missing `tag_name`".to_string())
        }
        UpdateChannel::Beta | UpdateChannel::Rc => {
            // /releases lists all releases, newest first — filter by channel
            let response = client
                .get(RELEASES_API)
                .send()
                .map_err(|e| format!("GitHub request failed: {e}"))?;
            let status = response.status();
            if !status.is_success() {
                return Err(format!("GitHub API returned {status}"));
            }
            let releases = response
                .json::<Vec<serde_json::Value>>()
                .map_err(|e| format!("Failed to decode releases list: {e}"))?;

            for release in &releases {
                let draft = release["draft"].as_bool().unwrap_or(false);
                if draft {
                    continue;
                }
                let Some(tag) = release["tag_name"].as_str().filter(|t| !t.is_empty()) else {
                    continue;
                };
                match channel {
                    UpdateChannel::Rc => return Ok(tag.to_string()),
                    UpdateChannel::Beta => {
                        if !tag.contains("-rc") {
                            return Ok(tag.to_string());
                        }
                    }
                    _ => unreachable!(),
                }
            }
            Err(format!(
                "No matching release found for the '{channel}' channel"
            ))
        }
    }
}

fn update_http_client() -> Result<reqwest::blocking::Client, String> {
    crate::http_client::client_builder()
        .user_agent(format!("librefang-cli/{}", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))
}

fn compare_release_tag(tag: &str, current_version: &str) -> ReleaseComparison {
    let Some(release_core) = parse_version_core(normalize_release_tag(tag)) else {
        return ReleaseComparison::Unknown;
    };
    let Some(current_core) = parse_version_core(current_version) else {
        return ReleaseComparison::Unknown;
    };

    match release_core.cmp(&current_core) {
        std::cmp::Ordering::Greater => ReleaseComparison::Newer,
        std::cmp::Ordering::Equal => ReleaseComparison::SameCore,
        std::cmp::Ordering::Less => ReleaseComparison::Older,
    }
}

fn parse_version_core(version: &str) -> Option<Vec<u64>> {
    let core = version.split('-').next()?;
    if core.is_empty() {
        return None;
    }
    core.split('.')
        .map(|part| part.parse::<u64>().ok())
        .collect()
}

fn run_official_update(version: Option<&str>) -> Result<UpdateLaunch, String> {
    let script_url = if cfg!(windows) {
        POWERSHELL_INSTALLER_URL
    } else {
        SHELL_INSTALLER_URL
    };
    let script = download_text(script_url)?;

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const DETACHED_PROCESS: u32 = 0x0000_0008;

        let wrapped = format!(
            "Start-Sleep -Seconds 1\r\n{script}\r\nRemove-Item $MyInvocation.MyCommand.Path -ErrorAction SilentlyContinue\r\n"
        );
        let script_path = write_update_script(&wrapped, "ps1")?;
        let script_arg = script_path.to_string_lossy().to_string();

        let mut command = std::process::Command::new("powershell");
        command
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                &script_arg,
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
        if let Some(tag) = version {
            command.env("LIBREFANG_VERSION", tag);
        }

        command
            .spawn()
            .map_err(|e| format!("Failed to launch PowerShell updater: {e}"))?;
        Ok(UpdateLaunch::Detached)
    }

    #[cfg(not(windows))]
    {
        let script_path = write_update_script(&script, "sh")?;
        let mut command = std::process::Command::new("sh");
        command.arg(&script_path);
        if let Some(tag) = version {
            command.env("LIBREFANG_VERSION", tag);
        }

        let status = command
            .status()
            .map_err(|e| format!("Failed to run installer: {e}"))?;
        let _ = std::fs::remove_file(&script_path);
        if !status.success() {
            return Err(format!("Installer exited with status {status}"));
        }
        Ok(UpdateLaunch::Completed)
    }
}

fn download_text(url: &str) -> Result<String, String> {
    let client = update_http_client()?;
    let response = client
        .get(url)
        .send()
        .map_err(|e| format!("Download failed: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("Download returned {status}"));
    }
    response
        .text()
        .map_err(|e| format!("Failed to read response body: {e}"))
}

#[cfg(not(windows))]
fn installed_binary_version(path: &std::path::Path) -> Option<String> {
    let output = std::process::Command::new(path)
        .arg("--version")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if version.is_empty() {
        None
    } else {
        Some(version)
    }
}

fn write_update_script(contents: &str, extension: &str) -> Result<PathBuf, String> {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!(
        "librefang-update-{}-{unique}.{extension}",
        std::process::id()
    ));
    std::fs::write(&path, contents).map_err(|e| format!("Failed to write updater script: {e}"))?;
    restrict_file_permissions(&path);
    Ok(path)
}

fn default_install_executable() -> PathBuf {
    cli_librefang_home().join("bin").join(binary_name())
}

fn cargo_install_executable() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".cargo")
        .join("bin")
        .join(binary_name())
}

fn binary_name() -> &'static str {
    if cfg!(windows) {
        "librefang.exe"
    } else {
        "librefang"
    }
}

fn same_path(left: &std::path::Path, right: &std::path::Path) -> bool {
    let left = std::fs::canonicalize(left).unwrap_or_else(|_| left.to_path_buf());
    let right = std::fs::canonicalize(right).unwrap_or_else(|_| right.to_path_buf());
    left == right
}

fn normalize_release_tag(tag: &str) -> &str {
    tag.strip_prefix('v').unwrap_or(tag)
}

fn cargo_update_command(version: Option<&str>) -> String {
    match version {
        Some(tag) => format!(
            "cargo install --git https://github.com/{RELEASE_REPO} --tag {tag} librefang-cli --force"
        ),
        None => format!(
            "cargo install --git https://github.com/{RELEASE_REPO} librefang-cli --force"
        ),
    }
}

fn manual_installer_command(version: Option<&str>) -> String {
    #[cfg(windows)]
    {
        match version {
            Some(tag) => {
                format!("$env:LIBREFANG_VERSION='{tag}'; irm {POWERSHELL_INSTALLER_URL} | iex")
            }
            None => format!("irm {POWERSHELL_INSTALLER_URL} | iex"),
        }
    }

    #[cfg(not(windows))]
    {
        match version {
            Some(tag) => format!("curl -fsSL {SHELL_INSTALLER_URL} | LIBREFANG_VERSION={tag} sh"),
            None => format!("curl -fsSL {SHELL_INSTALLER_URL} | sh"),
        }
    }
}

// ---------------------------------------------------------------------------
// Uninstall
// ---------------------------------------------------------------------------

fn cmd_uninstall(confirm: bool, keep_config: bool) {
    let librefang_dir = cli_librefang_home();
    let exe_path = std::env::current_exe().ok();

    // Step 1: Show what will be removed
    println!();
    println!(
        "  {}",
        "This will completely uninstall LibreFang from your system."
            .bold()
            .red()
    );
    println!();
    if librefang_dir.exists() {
        if keep_config {
            println!(
                "  • Remove data in {} (keeping config files)",
                librefang_dir.display()
            );
        } else {
            println!("  • Remove {}", librefang_dir.display());
        }
    }
    if let Some(ref exe) = exe_path {
        println!("  • Remove binary: {}", exe.display());
    }
    // Check cargo bin path
    let cargo_bin = dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".cargo")
        .join("bin")
        .join(if cfg!(windows) {
            "librefang.exe"
        } else {
            "librefang"
        });
    if cargo_bin.exists() && exe_path.as_ref().is_none_or(|e| *e != cargo_bin) {
        println!("  • Remove cargo binary: {}", cargo_bin.display());
    }
    println!("  • Remove auto-start entries (if any)");
    println!("  • Clean PATH from shell configs (if any)");
    println!();

    // Step 2: Confirm
    if !confirm {
        let answer = prompt_input("  Type 'uninstall' to confirm: ");
        if answer.trim() != "uninstall" {
            println!("  Cancelled.");
            return;
        }
        println!();
    }

    // Step 3: Stop running daemon
    if find_daemon().is_some() {
        println!("  {}", i18n::t("uninstall-stopping-daemon"));
        cmd_stop(None);
        // Give it a moment
        std::thread::sleep(std::time::Duration::from_secs(1));
        // Force kill if still alive
        if find_daemon().is_some() {
            if let Some(info) = read_daemon_info(&librefang_dir) {
                force_kill_pid(info.pid);
                let _ = std::fs::remove_file(librefang_dir.join("daemon.json"));
            }
        }
    }

    // Step 4: Remove auto-start entries
    let user_home = dirs::home_dir().unwrap_or_else(std::env::temp_dir);
    remove_autostart_entries(&user_home);

    // Step 5: Clean PATH from shell configs
    if let Some(ref exe) = exe_path {
        if let Some(bin_dir) = exe.parent() {
            clean_path_entries(&user_home, &bin_dir.to_string_lossy());
        }
    }

    // Step 6: Remove ~/.librefang/ data
    if librefang_dir.exists() {
        if keep_config {
            remove_dir_except_config(&librefang_dir);
            ui::success(&i18n::t("uninstall-removed-data-kept"));
        } else {
            match std::fs::remove_dir_all(&librefang_dir) {
                Ok(()) => ui::success(&i18n::t_args(
                    "uninstall-removed",
                    &[("path", &librefang_dir.display().to_string())],
                )),
                Err(e) => ui::error(&i18n::t_args(
                    "uninstall-remove-failed",
                    &[
                        ("path", &librefang_dir.display().to_string()),
                        ("error", &e.to_string()),
                    ],
                )),
            }
        }
    }

    // Step 7: Remove cargo bin copy if it exists and is separate from current exe
    if cargo_bin.exists() && exe_path.as_ref().is_none_or(|e| *e != cargo_bin) {
        match std::fs::remove_file(&cargo_bin) {
            Ok(()) => ui::success(&i18n::t_args(
                "uninstall-removed",
                &[("path", &cargo_bin.display().to_string())],
            )),
            Err(e) => ui::error(&i18n::t_args(
                "uninstall-remove-failed",
                &[
                    ("path", &cargo_bin.display().to_string()),
                    ("error", &e.to_string()),
                ],
            )),
        }
    }

    // Step 8: Remove the binary itself (skip if already removed with ~/.librefang/)
    if let Some(exe) = exe_path {
        if exe.exists() {
            remove_self_binary(&exe);
        }
    }

    println!();
    ui::success(&i18n::t("uninstall-goodbye"));
}

/// Remove auto-start / launch-agent / systemd entries.
#[allow(unused_variables)]
fn remove_autostart_entries(home: &std::path::Path) {
    #[cfg(windows)]
    {
        // Windows: remove from HKCU\Software\Microsoft\Windows\CurrentVersion\Run
        let output = std::process::Command::new("reg")
            .args([
                "delete",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                "/v",
                "LibreFang",
                "/f",
            ])
            .output();
        match output {
            Ok(o) if o.status.success() => {
                ui::success(&i18n::t("uninstall-removed-autostart-win"));
            }
            _ => {} // Entry didn't exist — that's fine
        }
    }

    #[cfg(target_os = "macos")]
    {
        let plist = home.join("Library/LaunchAgents/ai.librefang.desktop.plist");
        if plist.exists() {
            // Unload first
            let _ = std::process::Command::new("launchctl")
                .args(["unload", &plist.to_string_lossy()])
                .output();
            match std::fs::remove_file(&plist) {
                Ok(()) => ui::success(&i18n::t("uninstall-removed-launch-agent")),
                Err(e) => ui::error(&i18n::t_args(
                    "uninstall-remove-launch-fail",
                    &[("error", &e.to_string())],
                )),
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let desktop_file = home.join(".config/autostart/LibreFang.desktop");
        if desktop_file.exists() {
            match std::fs::remove_file(&desktop_file) {
                Ok(()) => ui::success(&i18n::t("uninstall-removed-autostart-linux")),
                Err(e) => ui::error(&i18n::t_args(
                    "uninstall-remove-autostart-fail",
                    &[("error", &e.to_string())],
                )),
            }
        }

        // Also check for systemd user service
        let service_file = home.join(".config/systemd/user/librefang.service");
        if service_file.exists() {
            let _ = std::process::Command::new("systemctl")
                .args(["--user", "disable", "--now", "librefang.service"])
                .output();
            match std::fs::remove_file(&service_file) {
                Ok(()) => {
                    let _ = std::process::Command::new("systemctl")
                        .args(["--user", "daemon-reload"])
                        .output();
                    ui::success(&i18n::t("uninstall-removed-systemd"));
                }
                Err(e) => ui::error(&i18n::t_args(
                    "uninstall-remove-systemd-fail",
                    &[("error", &e.to_string())],
                )),
            }
        }
    }
}

/// Remove lines from shell config files that add librefang to PATH.
#[allow(unused_variables)]
fn clean_path_entries(home: &std::path::Path, librefang_dir: &str) {
    #[cfg(not(windows))]
    {
        let shell_files = [
            home.join(".bashrc"),
            home.join(".bash_profile"),
            home.join(".profile"),
            home.join(".zshrc"),
            home.join(".config/fish/config.fish"),
        ];

        for path in &shell_files {
            if !path.exists() {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(path) else {
                continue;
            };
            let filtered: Vec<&str> = content
                .lines()
                .filter(|line| !is_librefang_path_line(line, librefang_dir))
                .collect();
            if filtered.len() < content.lines().count() {
                let new_content = filtered.join("\n");
                // Preserve trailing newline if original had one
                let new_content = if content.ends_with('\n') {
                    format!("{new_content}\n")
                } else {
                    new_content
                };
                if std::fs::write(path, &new_content).is_ok() {
                    ui::success(&i18n::t_args(
                        "uninstall-cleaned-path",
                        &[("path", &path.display().to_string())],
                    ));
                }
            }
        }
    }

    #[cfg(windows)]
    {
        // Read User PATH via PowerShell, filter out librefang entries, write back
        let output = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "[Environment]::GetEnvironmentVariable('PATH', 'User')",
            ])
            .output();
        if let Ok(out) = output {
            if out.status.success() {
                let current = String::from_utf8_lossy(&out.stdout);
                let current = current.trim();
                if !current.is_empty() {
                    let dir_lower = librefang_dir.to_lowercase();
                    let filtered: Vec<&str> = current
                        .split(';')
                        .filter(|entry| {
                            let e = entry.trim().to_lowercase();
                            !e.is_empty() && !e.contains("librefang") && !e.contains(&dir_lower)
                        })
                        .collect();
                    if filtered.len() < current.split(';').count() {
                        let new_path = filtered.join(";");
                        let ps_cmd = format!(
                            "[Environment]::SetEnvironmentVariable('PATH', '{}', 'User')",
                            new_path.replace('\'', "''")
                        );
                        let result = std::process::Command::new("powershell")
                            .args(["-NoProfile", "-Command", &ps_cmd])
                            .output();
                        if result.is_ok_and(|o| o.status.success()) {
                            ui::success(&i18n::t("uninstall-cleaned-path-win"));
                        }
                    }
                }
            }
        }
    }
}

/// Returns true if a shell config line is an librefang PATH export.
/// Must match BOTH an librefang reference AND a PATH-setting pattern.
#[cfg(any(not(windows), test))]
fn is_librefang_path_line(line: &str, librefang_dir: &str) -> bool {
    let lower = line.to_lowercase();
    let has_librefang =
        lower.contains("librefang") || lower.contains(&librefang_dir.to_lowercase());
    if !has_librefang {
        return false;
    }
    // Match common PATH-setting patterns
    lower.contains("export path=")
        || lower.contains("export path =")
        || lower.starts_with("path=")
        || lower.contains("set -gx path")
        || lower.contains("fish_add_path")
}

/// Remove everything in ~/.librefang/ except config files.
fn remove_dir_except_config(librefang_dir: &std::path::Path) {
    let keep = ["config.toml", ".env", "secrets.env"];
    let Ok(entries) = std::fs::read_dir(librefang_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if keep.contains(&name_str.as_ref()) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            let _ = std::fs::remove_dir_all(&path);
        } else {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Remove the currently-running binary.
fn remove_self_binary(exe_path: &std::path::Path) {
    #[cfg(unix)]
    {
        // On Unix, running binaries can be unlinked — the OS keeps the inode
        // alive until the process exits.
        match std::fs::remove_file(exe_path) {
            Ok(()) => ui::success(&i18n::t_args(
                "uninstall-removed",
                &[("path", &exe_path.display().to_string())],
            )),
            Err(e) => ui::error(&i18n::t_args(
                "uninstall-remove-failed",
                &[
                    ("path", &exe_path.display().to_string()),
                    ("error", &e.to_string()),
                ],
            )),
        }
    }

    #[cfg(windows)]
    {
        // Windows locks running executables. Rename first, then spawn a
        // detached process that waits briefly and deletes the renamed file.
        let old_path = exe_path.with_extension("exe.old");
        if std::fs::rename(exe_path, &old_path).is_err() {
            ui::error(&format!(
                "Could not rename binary for deferred deletion: {}",
                exe_path.display()
            ));
            return;
        }

        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const DETACHED_PROCESS: u32 = 0x0000_0008;

        let del_cmd = format!(
            "ping -n 3 127.0.0.1 >nul & del /f /q \"{}\"",
            old_path.display()
        );
        let _ = std::process::Command::new("cmd.exe")
            .args(["/C", &del_cmd])
            .creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS)
            .spawn();

        ui::success(&i18n::t_args(
            "uninstall-removed",
            &[("path", &exe_path.display().to_string())],
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::{
        channel_test_request_body, compare_release_tag, daemon_log_path_for_config,
        daemon_log_path_for_home, detached_daemon_args, normalize_release_tag, parse_version_core,
        resolve_device_auth_start, resolve_hand_instance, AuthCommands, ChannelCommands, Cli,
        Commands, DeviceAuthNextStep, GatewayCommands, ReleaseComparison,
    };
    use clap::Parser;
    use serde_json::json;
    use std::ffi::OsString;
    use std::fs;
    use std::path::Path;

    // --- Doctor command unit tests ---

    #[test]
    fn test_start_accepts_tail_flag() {
        let cli = Cli::parse_from(["librefang", "start", "--tail"]);
        match cli.command {
            Some(Commands::Start {
                tail,
                foreground,
                spawned,
            }) => {
                assert!(tail);
                assert!(!foreground);
                assert!(!spawned);
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn test_restart_accepts_tail_flag() {
        let cli = Cli::parse_from(["librefang", "restart", "--tail"]);
        match cli.command {
            Some(Commands::Restart { tail, foreground }) => {
                assert!(tail);
                assert!(!foreground);
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn test_gateway_start_accepts_tail_flag() {
        let cli = Cli::parse_from(["librefang", "gateway", "start", "--tail"]);
        match cli.command {
            Some(Commands::Gateway(GatewayCommands::Start { tail, foreground })) => {
                assert!(tail);
                assert!(!foreground);
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn test_gateway_restart_accepts_tail_flag() {
        let cli = Cli::parse_from(["librefang", "gateway", "restart", "--tail"]);
        match cli.command {
            Some(Commands::Gateway(GatewayCommands::Restart { tail, foreground })) => {
                assert!(tail);
                assert!(!foreground);
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn test_start_accepts_foreground_flag() {
        let cli = Cli::parse_from(["librefang", "start", "--foreground"]);
        match cli.command {
            Some(Commands::Start {
                tail,
                foreground,
                spawned,
            }) => {
                assert!(!tail);
                assert!(foreground);
                assert!(!spawned);
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn test_restart_accepts_foreground_flag() {
        let cli = Cli::parse_from(["librefang", "restart", "--foreground"]);
        match cli.command {
            Some(Commands::Restart { tail, foreground }) => {
                assert!(!tail);
                assert!(foreground);
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn test_gateway_start_accepts_foreground_flag() {
        let cli = Cli::parse_from(["librefang", "gateway", "start", "--foreground"]);
        match cli.command {
            Some(Commands::Gateway(GatewayCommands::Start { tail, foreground })) => {
                assert!(!tail);
                assert!(foreground);
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn test_gateway_restart_accepts_foreground_flag() {
        let cli = Cli::parse_from(["librefang", "gateway", "restart", "--foreground"]);
        match cli.command {
            Some(Commands::Gateway(GatewayCommands::Restart { tail, foreground })) => {
                assert!(!tail);
                assert!(foreground);
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn test_channel_test_accepts_target_channel_flag() {
        let cli = Cli::parse_from([
            "librefang",
            "channel",
            "test",
            "discord",
            "--channel",
            "123456789",
        ]);
        match cli.command {
            Some(Commands::Channel(ChannelCommands::Test {
                name,
                channel_id,
                chat_id,
            })) => {
                assert_eq!(name, "discord");
                assert_eq!(channel_id.as_deref(), Some("123456789"));
                assert!(chat_id.is_none());
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn test_channel_test_accepts_chat_id_flag() {
        let cli = Cli::parse_from([
            "librefang",
            "channel",
            "test",
            "telegram",
            "--chat-id",
            "999",
        ]);
        match cli.command {
            Some(Commands::Channel(ChannelCommands::Test {
                name,
                channel_id,
                chat_id,
            })) => {
                assert_eq!(name, "telegram");
                assert!(channel_id.is_none());
                assert_eq!(chat_id.as_deref(), Some("999"));
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn test_channel_test_rejects_both_target_flags() {
        let cli = Cli::try_parse_from([
            "librefang",
            "channel",
            "test",
            "discord",
            "--channel",
            "123",
            "--chat-id",
            "456",
        ]);
        assert!(cli.is_err());
    }

    #[test]
    fn test_channel_test_request_body_prefers_channel_id() {
        assert_eq!(
            channel_test_request_body(Some("C123"), None),
            Some(json!({ "channel_id": "C123" }))
        );
    }

    #[test]
    fn test_channel_test_request_body_supports_chat_id() {
        assert_eq!(
            channel_test_request_body(None, Some("42")),
            Some(json!({ "chat_id": "42" }))
        );
    }

    #[test]
    fn test_channel_test_request_body_empty_when_no_target() {
        assert_eq!(channel_test_request_body(None, None), None);
    }

    #[test]
    fn test_auth_chatgpt_accepts_device_auth_flag() {
        let cli = Cli::parse_from(["librefang", "auth", "chatgpt", "--device-auth"]);
        match cli.command {
            Some(Commands::Auth(AuthCommands::Chatgpt { device_auth })) => {
                assert!(device_auth);
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn test_resolve_device_auth_start_continues_device_path() {
        let prompt = librefang_runtime::chatgpt_oauth::DeviceAuthPrompt {
            device_auth_id: "device-1".to_string(),
            user_code: "ABCD-EFGH".to_string(),
            interval_secs: 9,
        };

        match resolve_device_auth_start(Ok(prompt.clone())).unwrap() {
            DeviceAuthNextStep::ContinueDevice(actual) => assert_eq!(actual, prompt),
            DeviceAuthNextStep::FallbackToBrowser(_) => panic!("unexpected fallback"),
        }
    }

    #[test]
    fn test_resolve_device_auth_start_requests_browser_fallback_on_unsupported_error() {
        let err = librefang_runtime::chatgpt_oauth::DeviceAuthFlowError::BrowserFallback {
            message: "fallback".to_string(),
        };

        match resolve_device_auth_start(Err(err)).unwrap() {
            DeviceAuthNextStep::FallbackToBrowser(message) => assert_eq!(message, "fallback"),
            DeviceAuthNextStep::ContinueDevice(_) => panic!("unexpected device continuation"),
        }
    }

    #[test]
    fn test_start_rejects_tail_with_foreground() {
        let cli = Cli::try_parse_from(["librefang", "start", "--tail", "--foreground"]);
        assert!(cli.is_err());
    }

    #[test]
    fn test_detached_daemon_args_include_config_and_spawned_flag() {
        let args = detached_daemon_args(Some(Path::new("/tmp/librefang.toml")));
        assert_eq!(
            args,
            vec![
                OsString::from("--config"),
                OsString::from("/tmp/librefang.toml"),
                OsString::from("start"),
                OsString::from("--spawned"),
            ]
        );
    }

    #[test]
    fn test_daemon_log_path_uses_logs_directory() {
        let home = Path::new("/tmp/librefang-home");
        assert_eq!(
            daemon_log_path_for_home(home),
            home.join("logs").join("daemon.log")
        );
    }

    #[test]
    fn test_daemon_log_path_respects_custom_config_home_dir() {
        let temp_root = std::env::temp_dir().join(format!(
            "librefang-cli-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&temp_root).unwrap();
        let config_path = temp_root.join("config.toml");
        let custom_home = temp_root.join("custom-home");
        fs::write(
            &config_path,
            format!("home_dir = {:?}\n", custom_home.display().to_string()),
        )
        .unwrap();

        assert_eq!(
            daemon_log_path_for_config(Some(&config_path)),
            custom_home.join("logs").join("daemon.log")
        );

        let _ = fs::remove_dir_all(&temp_root);
    }

    #[test]
    fn test_doctor_skill_registry_loads() {
        let skills_dir = std::env::temp_dir().join("librefang-doctor-test-skills");
        let mut skill_reg = librefang_skills::registry::SkillRegistry::new(skills_dir);
        let count = skill_reg.load_all().unwrap_or(0);
        assert_eq!(skill_reg.count(), count);
    }

    #[test]
    fn test_doctor_extension_registry_loads_templates() {
        let tmp = std::env::temp_dir().join("librefang-doctor-test-ext");
        let _ = std::fs::create_dir_all(&tmp);
        let mut catalog = librefang_extensions::catalog::McpCatalog::new(&tmp);
        let count = catalog.load(&librefang_runtime::registry_sync::resolve_home_dir_for_tests());
        assert_eq!(catalog.len(), count);
    }

    #[test]
    fn test_doctor_config_deser_default() {
        // Default KernelConfig should serialize/deserialize round-trip
        let config = librefang_types::config::KernelConfig::default();
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: librefang_types::config::KernelConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.api_listen, config.api_listen);
    }

    #[test]
    fn test_doctor_config_include_field() {
        let config_toml = r#"
api_listen = "127.0.0.1:4545"
include = ["providers.toml", "agents.toml"]

[default_model]
provider = "groq"
model = "llama-3.3-70b-versatile"
api_key_env = "GROQ_API_KEY"
"#;
        let config: librefang_types::config::KernelConfig = toml::from_str(config_toml).unwrap();
        assert_eq!(config.include.len(), 2);
        assert_eq!(config.include[0], "providers.toml");
        assert_eq!(config.include[1], "agents.toml");
    }

    #[test]
    fn test_doctor_exec_policy_field() {
        let config_toml = r#"
api_listen = "127.0.0.1:4545"

[exec_policy]
mode = "allowlist"
safe_bins = ["ls", "cat", "echo"]
timeout_secs = 30

[default_model]
provider = "groq"
model = "llama-3.3-70b-versatile"
api_key_env = "GROQ_API_KEY"
"#;
        let config: librefang_types::config::KernelConfig = toml::from_str(config_toml).unwrap();
        assert_eq!(
            config.exec_policy.mode,
            librefang_types::config::ExecSecurityMode::Allowlist
        );
        assert_eq!(config.exec_policy.safe_bins.len(), 3);
        assert_eq!(config.exec_policy.timeout_secs, 30);
    }

    #[test]
    fn test_doctor_mcp_transport_validation() {
        let config_toml = r#"
api_listen = "127.0.0.1:4545"

[default_model]
provider = "groq"
model = "llama-3.3-70b-versatile"
api_key_env = "GROQ_API_KEY"

[[mcp_servers]]
name = "github"
timeout_secs = 30

[mcp_servers.transport]
type = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
"#;
        let config: librefang_types::config::KernelConfig = toml::from_str(config_toml).unwrap();
        assert_eq!(config.mcp_servers.len(), 1);
        assert_eq!(config.mcp_servers[0].name, "github");
        match config.mcp_servers[0].transport.as_ref().unwrap() {
            librefang_types::config::McpTransportEntry::Stdio { command, args } => {
                assert_eq!(command, "npx");
                assert_eq!(args.len(), 2);
            }
            _ => panic!("Expected Stdio transport"),
        }
    }

    #[test]
    fn test_doctor_http_compat_transport_validation() {
        let config_toml = r#"
api_listen = "127.0.0.1:4545"

[default_model]
provider = "groq"
model = "llama-3.3-70b-versatile"
api_key_env = "GROQ_API_KEY"

[[mcp_servers]]
name = "http-tools"
timeout_secs = 30

[mcp_servers.transport]
type = "http_compat"
base_url = "http://127.0.0.1:11235"

[[mcp_servers.transport.headers]]
name = "Authorization"
value_env = "HTTP_TOOLS_TOKEN"

[[mcp_servers.transport.tools]]
name = "search"
description = "Search HTTP backend"
path = "/search"
method = "get"
request_mode = "query"
response_mode = "json"
input_schema = { type = "object" }
"#;
        let config: librefang_types::config::KernelConfig = toml::from_str(config_toml).unwrap();
        assert_eq!(config.mcp_servers.len(), 1);
        assert_eq!(config.mcp_servers[0].name, "http-tools");
        match config.mcp_servers[0].transport.as_ref().unwrap() {
            librefang_types::config::McpTransportEntry::HttpCompat {
                base_url,
                headers,
                tools,
            } => {
                assert_eq!(base_url, "http://127.0.0.1:11235");
                assert_eq!(headers.len(), 1);
                assert_eq!(tools.len(), 1);
                assert_eq!(tools[0].name, "search");
            }
            _ => panic!("Expected HttpCompat transport"),
        }
    }

    #[test]
    fn test_doctor_skill_injection_scan_clean() {
        let clean_content = "This is a normal skill prompt with helpful instructions.";
        let warnings = librefang_skills::verify::SkillVerifier::scan_prompt_content(clean_content);
        assert!(warnings.is_empty(), "Clean content should have no warnings");
    }

    #[test]
    fn test_doctor_hook_event_variants() {
        // Verify all 4 hook event types are constructable
        use librefang_types::agent::HookEvent;
        let events = [
            HookEvent::BeforeToolCall,
            HookEvent::AfterToolCall,
            HookEvent::BeforePromptBuild,
            HookEvent::AgentLoopEnd,
        ];
        assert_eq!(events.len(), 4);
    }

    // --- Uninstall command unit tests ---

    #[test]
    fn test_uninstall_path_line_filter() {
        use super::is_librefang_path_line;
        let dir = "/home/user/.librefang/bin";

        // Should match: librefang PATH exports
        assert!(is_librefang_path_line(
            r#"export PATH="$HOME/.librefang/bin:$PATH""#,
            dir
        ));
        assert!(is_librefang_path_line(
            r#"export PATH="/home/user/.librefang/bin:$PATH""#,
            dir
        ));
        assert!(is_librefang_path_line(
            "set -gx PATH $HOME/.librefang/bin $PATH",
            dir
        ));
        assert!(is_librefang_path_line(
            "fish_add_path $HOME/.librefang/bin",
            dir
        ));

        // Should NOT match: unrelated PATH exports
        assert!(!is_librefang_path_line(
            r#"export PATH="$HOME/.cargo/bin:$PATH""#,
            dir
        ));
        assert!(!is_librefang_path_line(
            r#"export PATH="/usr/local/bin:$PATH""#,
            dir
        ));

        // Should NOT match: librefang lines that aren't PATH-related
        assert!(!is_librefang_path_line("# librefang config", dir));
        assert!(!is_librefang_path_line("alias of=librefang", dir));
    }

    #[test]
    fn test_update_command_parses() {
        let cli = Cli::parse_from(["librefang", "update"]);
        assert!(matches!(
            cli.command,
            Some(Commands::Update {
                check: false,
                version: None,
                channel: None,
            })
        ));
    }

    #[test]
    fn test_update_check_command_parses() {
        let cli = Cli::parse_from(["librefang", "update", "--check"]);
        assert!(matches!(
            cli.command,
            Some(Commands::Update {
                check: true,
                version: None,
                channel: None,
            })
        ));
    }

    #[test]
    fn test_update_channel_command_parses() {
        let cli = Cli::parse_from(["librefang", "update", "--channel", "rc"]);
        match cli.command {
            Some(Commands::Update { channel, .. }) => {
                assert_eq!(channel.as_deref(), Some("rc"));
            }
            _ => panic!("Expected Update command"),
        }
    }

    #[test]
    fn test_spawn_alias_parses() {
        let cli = Cli::parse_from(["librefang", "spawn", "coder", "--name", "backend-coder"]);
        assert!(matches!(cli.command, Some(Commands::Spawn(_))));
    }

    #[test]
    fn test_agents_alias_parses() {
        let cli = Cli::parse_from(["librefang", "agents", "--json"]);
        assert!(matches!(cli.command, Some(Commands::Agents { json: true })));
    }

    #[test]
    fn test_kill_alias_parses() {
        let cli = Cli::parse_from(["librefang", "kill", "agent-123"]);
        assert!(matches!(
            cli.command,
            Some(Commands::Kill { agent_id }) if agent_id == "agent-123"
        ));
    }

    #[test]
    fn test_agent_spawn_dry_run_parses() {
        let cli = Cli::parse_from(["librefang", "agent", "spawn", "--dry-run", "agent.toml"]);
        assert!(matches!(cli.command, Some(Commands::Agent(_))));
    }

    #[test]
    fn test_hand_status_parses() {
        let cli = Cli::parse_from(["librefang", "hand", "status", "researcher"]);
        assert!(matches!(cli.command, Some(Commands::Hand(_))));
    }

    #[test]
    fn test_skill_test_parses() {
        let cli = Cli::parse_from(["librefang", "skill", "test", ".", "--tool", "summarize"]);
        assert!(matches!(cli.command, Some(Commands::Skill(_))));
    }

    #[test]
    fn test_skill_publish_parses() {
        let cli = Cli::parse_from([
            "librefang",
            "skill",
            "publish",
            ".",
            "--repo",
            "librefang-skills/demo",
            "--dry-run",
        ]);
        assert!(matches!(cli.command, Some(Commands::Skill(_))));
    }

    #[test]
    fn test_normalize_release_tag_strips_v_prefix() {
        assert_eq!(normalize_release_tag("v0.3.56"), "0.3.56");
        assert_eq!(normalize_release_tag("0.3.56"), "0.3.56");
    }

    #[test]
    fn test_parse_version_core_strips_release_suffix() {
        assert_eq!(parse_version_core("0.3.56-20260312"), Some(vec![0, 3, 56]));
        assert_eq!(parse_version_core("0.3.56"), Some(vec![0, 3, 56]));
    }

    #[test]
    fn test_compare_release_tag_detects_newer_release() {
        assert_eq!(
            compare_release_tag("v0.3.57-20260312", "0.3.56"),
            ReleaseComparison::Newer
        );
    }

    #[test]
    fn test_compare_release_tag_detects_same_core_release() {
        assert_eq!(
            compare_release_tag("v0.3.56-20260312", "0.3.56"),
            ReleaseComparison::SameCore
        );
    }

    #[test]
    fn test_compare_release_tag_detects_older_release() {
        assert_eq!(
            compare_release_tag("v0.3.55-20260312", "0.3.56"),
            ReleaseComparison::Older
        );
    }

    #[test]
    fn test_resolve_hand_instance_matches_hand_id() {
        let instances = vec![serde_json::json!({
            "instance_id": "inst-1",
            "hand_id": "researcher",
            "status": "running",
            "agent_name": "researcher-agent"
        })];
        let resolved =
            resolve_hand_instance(&instances, "researcher").expect("hand should resolve");
        assert_eq!(resolved["instance_id"].as_str(), Some("inst-1"));
    }

    #[test]
    fn test_resolve_hand_instance_matches_instance_id() {
        let instances = vec![serde_json::json!({
            "instance_id": "inst-1",
            "hand_id": "researcher"
        })];
        let resolved =
            resolve_hand_instance(&instances, "inst-1").expect("instance should resolve");
        assert_eq!(resolved["hand_id"].as_str(), Some("researcher"));
    }
}
