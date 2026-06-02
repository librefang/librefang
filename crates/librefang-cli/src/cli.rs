//! Command-line argument definitions for the LibreFang CLI.
//!
//! The clap derive types — the top-level [`Cli`] parser, the [`Commands`]
//! enum, and every subcommand enum / argument struct — live here, split out
//! of `main.rs` to keep that file focused on `main()` and dispatch. Command
//! handlers live under `commands/`.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

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
    pub(crate) config: Option<PathBuf>,

    #[command(subcommand)]
    pub(crate) command: Option<Commands>,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
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
        long_about = "Start the LibreFang kernel daemon, which runs the API server and agent runtime.\n\nBy default the daemon detaches into the background. Use --foreground to keep it\nattached to the current terminal, or --tail to detach but stream logs.\n\nExamples:\n  librefang start                # Start daemon in the background\n  librefang start --tail         # Start and follow log output\n  librefang start --foreground   # Run in the foreground (Ctrl+C to stop)"
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
    /// Manage messaging channels (list, setup, reload, rm) [*].
    #[command(
        subcommand,
        long_about = "Manage out-of-process messaging channel sidecars (Telegram, Discord, Slack, …).\n\nEvery channel runs as a sidecar adapter; this subcommand drives the\nsurviving daemon endpoints: `GET /api/channels` for the list, `GET\n/api/channels/registry` + `POST /api/channels/sidecar/{name}/configure`\nfor setup, `POST /api/channels/reload` to apply changes without a\ndaemon restart.\n\nThe pre-migration `librefang channel test / enable / disable` arms\nare not restored — sidecars surface their own health via stdout logs\n(no in-band /test endpoint), and presence of the `[[sidecar_channels]]`\nblock in `config.toml` is the only on/off signal (use `rm` to remove).\n\nExamples:\n  librefang channel list                 # Show configured channels\n  librefang channel setup                # Interactive picker over unconfigured rows\n  librefang channel setup telegram       # Schema-driven configure for one adapter\n  librefang channel reload               # Hot-reload after manual config.toml edits\n  librefang channel rm telegram          # Delete the [[sidecar_channels]] entry + reload"
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
    /// Run the Agent Client Protocol (ACP) server over stdio (#3313).
    ///
    /// Launches an in-process kernel and serves an ACP `Agent` on
    /// stdin/stdout. Editors like Zed, VS Code (Claude Code), and
    /// JetBrains spawn this as a child process per workspace and
    /// drive prompts / approvals / streaming through it.
    #[command(
        long_about = "Run the Agent Client Protocol (ACP) server over stdio (#3313).\n\nExposes a LibreFang agent to ACP-compatible editors (Zed, VS Code, JetBrains).\nThe editor spawns `librefang acp` as a child process per workspace; this\ncommand runs an in-process kernel and serves the JSON-RPC protocol on\nstdin/stdout until the editor disconnects.\n\nExamples:\n  librefang acp                    # Use the default agent (\"assistant\")\n  librefang acp --agent reviewer   # Use a named agent\n  librefang acp --agent <uuid>     # Use an agent by UUID"
    )]
    Acp {
        /// Agent name or UUID to embed. Defaults to "assistant".
        #[arg(long)]
        agent: Option<String>,
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
        long_about = "List conversation sessions stored by agents.\n\nOptionally filter by agent name or ID. The STATE column reflects whether the session has an in-flight loop (running) or is idle.\n\nExamples:\n  librefang sessions              # List all sessions\n  librefang sessions coder        # Filter by agent name\n  librefang sessions --active     # Only currently-executing sessions\n  librefang sessions --json       # JSON output for scripting"
    )]
    Sessions {
        /// Optional agent name or ID to filter by.
        agent: Option<String>,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
        /// Only show sessions that currently have an in-flight loop.
        #[arg(long)]
        active: bool,
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
        long_about = "Send a single message to an agent and print the response.\n\nUnlike `chat`, this does not start an interactive session. Useful for\nscripting and automation.\n\nExamples:\n  librefang message coder \"Fix the bug in main.rs\"\n  librefang message coder \"Summarize this file\" --json\n  librefang message coder \"Draft this email\" --incognito"
    )]
    Message {
        /// Agent name or ID.
        agent: String,
        /// Message text.
        text: String,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
        /// Run in incognito mode: session messages and memory writes are
        /// suppressed while memory reads remain fully operational.
        #[arg(long)]
        incognito: bool,
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
pub(crate) enum VaultCommands {
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
    /// Rotate the vault master key (re-encrypt every entry with a new key).
    ///
    /// Recovery / hygiene workflow shipped for #3651. By default reads the
    /// old key from `LIBREFANG_VAULT_KEY_OLD` and the new key from
    /// `LIBREFANG_VAULT_KEY_NEW`; pass `--from-stdin` to read the new key
    /// from stdin instead (useful when the new key cannot safely live in
    /// the shell history). Both keys must be valid base64 of exactly
    /// 32 bytes (`openssl rand -base64 32`).
    ///
    /// The vault is re-encrypted to a temp file, fsync'd, then atomically
    /// renamed over the original — no half-rotated state on disk if the
    /// process is killed mid-way. The startup sentinel is preserved so the
    /// daemon will boot cleanly under the new key.
    #[command(
        long_about = "Rotate the vault master key (re-encrypt every entry with a new key).\n\nReads the old key from LIBREFANG_VAULT_KEY_OLD and the new key from LIBREFANG_VAULT_KEY_NEW (or from stdin with --from-stdin). Both must be base64 of exactly 32 bytes (openssl rand -base64 32).\n\nAfter a successful rotation, restart the daemon with the new LIBREFANG_VAULT_KEY set to the new value. Until you do, the daemon will refuse to boot — the startup sentinel verifies the key matches the rotated vault.\n\nExamples:\n  LIBREFANG_VAULT_KEY_OLD=$(cat .key.old) \\\n  LIBREFANG_VAULT_KEY_NEW=$(cat .key.new) \\\n    librefang vault rotate-key\n\n  echo $NEW_KEY | LIBREFANG_VAULT_KEY_OLD=$OLD_KEY librefang vault rotate-key --from-stdin"
    )]
    RotateKey {
        /// Read the new key from stdin instead of `LIBREFANG_VAULT_KEY_NEW`.
        #[arg(long)]
        from_stdin: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum AuthCommands {
    /// Authenticate with ChatGPT using browser or device auth.
    #[command(
        long_about = "Authenticate with ChatGPT using the OpenAI Codex login flow.\n\nBy default this opens a browser and waits for the localhost callback.\nUse --device-auth for headless or remote environments. If device auth is\nnot enabled for the current OpenAI account or workspace, LibreFang falls\nback to the standard browser login flow.\n\nExamples:\n  librefang auth chatgpt\n  librefang auth chatgpt --device-auth"
    )]
    Chatgpt {
        /// Use the OpenAI device auth flow before falling back to browser auth.
        #[arg(long)]
        device_auth: bool,
    },
    /// Manage credential pools for multi-key per-provider rotation (#4965) [*].
    #[command(
        subcommand,
        long_about = "Inspect and manage credential pools — multi-key API key rotation per provider.\n\nPools are configured in config.toml as `[[credential_pools]]` blocks. The CLI\ntalks to the running daemon if one is up; otherwise it reads the config file\ndirectly. Mutating subcommands (`add`, `remove`, `strategy`) rewrite\nconfig.toml and require a daemon restart or hot-reload to take effect.\n\nExamples:\n  librefang auth pool list                              # Show all pools and key telemetry\n  librefang auth pool list --json                       # Machine-readable output\n  librefang auth pool add openai OPENAI_API_KEY_2 --label Backup --priority 5\n  librefang auth pool strategy openai round_robin\n  librefang auth pool remove openai OPENAI_API_KEY_2"
    )]
    Pool(AuthPoolCommands),
}

#[derive(Subcommand)]
pub(crate) enum AuthPoolCommands {
    /// List configured credential pools with per-key telemetry.
    List {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Add a new key entry to a provider's pool.
    ///
    /// Creates the pool if it does not yet exist. The pool's strategy defaults
    /// to `fill_first` (highest priority first) on first creation; change it
    /// later with `librefang auth pool strategy`.
    Add {
        /// Provider name (e.g. `openai`, `anthropic`, `groq`).
        provider: String,
        /// Name of the environment variable holding the API key.
        env_var: String,
        /// Human-readable label for the key (e.g. `Primary`, `Backup`).
        #[arg(long, default_value = "Key")]
        label: String,
        /// Priority — higher value picked first under `fill_first` / `round_robin`.
        #[arg(long, default_value_t = 0)]
        priority: u32,
    },
    /// Remove a key entry from a provider's pool.
    ///
    /// Removes the pool itself when the last key entry is removed. The
    /// `env_var` argument must match the entry's `api_key_env` field exactly.
    Remove {
        /// Provider name.
        provider: String,
        /// Env-var name of the key to remove.
        env_var: String,
    },
    /// Change a pool's selection strategy.
    Strategy {
        /// Provider name.
        provider: String,
        /// Strategy: `fill_first`, `round_robin`, `random`, `least_used`.
        strategy: String,
    },
}

#[derive(Clone, clap::ValueEnum)]
pub(crate) enum ScaffoldKind {
    Skill,
    Mcp,
}

#[derive(Subcommand)]
pub(crate) enum McpCommands {
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
pub(crate) struct MigrateArgs {
    /// Source framework to migrate from.
    #[arg(long, value_enum)]
    pub(crate) from: MigrateSourceArg,
    /// Path to the source workspace (auto-detected if not set).
    #[arg(long)]
    pub(crate) source_dir: Option<PathBuf>,
    /// Dry run — show what would be imported without making changes.
    #[arg(long)]
    pub(crate) dry_run: bool,
}

#[derive(clap::Args)]
pub(crate) struct SpawnAliasArgs {
    /// Template name (e.g. "coder") or manifest path. Interactive picker if omitted.
    pub(crate) target: Option<String>,
    /// Explicit manifest path (legacy alias for a template file path).
    #[arg(long)]
    pub(crate) template: Option<PathBuf>,
    /// Override the agent name before spawning.
    #[arg(long)]
    pub(crate) name: Option<String>,
    /// Parse and preview the manifest without spawning an agent.
    #[arg(long)]
    pub(crate) dry_run: bool,
}

#[derive(clap::Args)]
pub(crate) struct AgentSpawnArgs {
    /// Path to the agent manifest TOML file.
    pub(crate) manifest: PathBuf,
    /// Override the agent name before spawning.
    #[arg(long)]
    pub(crate) name: Option<String>,
    /// Parse and preview the manifest without spawning an agent.
    #[arg(long)]
    pub(crate) dry_run: bool,
}

#[derive(Clone, clap::ValueEnum)]
pub(crate) enum MigrateSourceArg {
    Openclaw,
    Langchain,
    Autogpt,
    Openfang,
}

#[derive(Subcommand)]
pub(crate) enum ChannelCommands {
    /// List configured + discoverable channels via `GET /api/channels`.
    #[command(
        long_about = "List configured + discoverable channels via `GET /api/channels`.\n\nColumns: NAME, KIND, CONFIGURED, TOKEN (does every required secret env\nvar have a value), 24H MSGS.\n\nRequires a running daemon — falls through with an error if the daemon\nis not reachable. To inspect raw config without a daemon, read\n`~/.librefang/config.toml` directly.\n\nExamples:\n  librefang channel list"
    )]
    List,
    /// Trigger `POST /api/channels/reload` so the daemon re-reads\n    /// `[[sidecar_channels]]` from disk without restarting.
    #[command(
        long_about = "Trigger `POST /api/channels/reload` so the daemon re-reads\n`[[sidecar_channels]]` from `~/.librefang/config.toml` (plus any\n`include`-d files) without restarting. Use after a manual edit, or\nafter `librefang channel rm`.\n\nExamples:\n  librefang channel reload"
    )]
    Reload,
    /// Interactive schema-driven sidecar configure.
    #[command(
        long_about = "Interactive schema-driven sidecar configure.\n\nWith no argument: shows a picker over the currently-unconfigured\nadapters (via `GET /api/channels`). With an argument: jumps straight\nto the configure prompts for that adapter.\n\nPrompts for each field the sidecar's `--describe` schema lists\n(secret fields are masked + flagged as `(set — leave blank to keep)`\nwhen they already have a value). On submit, POSTs to\n`/api/channels/sidecar/{name}/configure`, which splits values across\n`~/.librefang/secrets.env` (secret-typed fields) and `[[sidecar_channels]]`\nin `config.toml` (everything else), then hot-reloads.\n\nExamples:\n  librefang channel setup            # Interactive picker\n  librefang channel setup telegram   # Schema-driven configure for one adapter"
    )]
    Setup {
        /// Sidecar adapter name (`telegram`, `ntfy`, …). Picker if omitted.
        name: Option<String>,
    },
    /// Remove a `[[sidecar_channels]]` entry from config.toml + reload.
    #[command(
        long_about = "Remove the `[[sidecar_channels]]` entry whose `name` matches\n`<NAME>` from `~/.librefang/config.toml`, then hot-reload so the\nrunning sidecar shuts down.\n\nPresence of the `[[sidecar_channels]]` block is the only on/off\nsignal post-migration (`enable` / `disable` are retired), so `rm`\nis how you turn an adapter off.\n\nExamples:\n  librefang channel rm telegram"
    )]
    Rm {
        /// Sidecar entry `name` field to remove.
        name: String,
    },
}

#[derive(Subcommand)]
pub(crate) enum SkillCommands {
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
    /// Skill workshop (#3328) — review pending candidates captured from
    /// agent conversations.
    #[command(
        subcommand,
        long_about = "Review candidates produced by the skill workshop after-turn capture.\n\nA candidate is a draft skill the workshop extracted from a conversation\nturn (e.g. `from now on always run cargo fmt`). Candidates land in\n`~/.librefang/skills/pending/<agent_id>/<uuid>.toml` and are NOT loaded\ninto the active registry until you approve them. Approval routes\nthrough the same evolution::create_skill path used for marketplace\ninstalls, so name validation and prompt-injection scans run a second\ntime before anything reaches `~/.librefang/skills/`.\n\nExamples:\n  librefang skill pending list\n  librefang skill pending show <id>\n  librefang skill pending approve <id>\n  librefang skill pending reject <id>"
    )]
    Pending(PendingCommands),
}

#[derive(Subcommand)]
pub(crate) enum PendingCommands {
    /// List pending candidates (oldest captured first — same order the
    /// dashboard's pending review section renders).
    List {
        /// Show only candidates from the given agent UUID.
        #[arg(long)]
        agent: Option<String>,
    },
    /// Print a candidate's full TOML and provenance.
    Show {
        /// Candidate UUID (shown by `pending list`).
        id: String,
    },
    /// Promote a candidate into the active skill registry.
    Approve {
        /// Candidate UUID.
        id: String,
    },
    /// Drop a candidate without promoting.
    Reject {
        /// Candidate UUID.
        id: String,
    },
}

#[derive(Subcommand)]
pub(crate) enum EvolveCommands {
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
pub(crate) enum HandCommands {
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
pub(crate) enum ConfigCommands {
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
pub(crate) enum AgentCommands {
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
        long_about = "Terminate a running agent by its UUID.\n\nThis is a destructive operation: the agent's canonical UUID binding is\npurged, orphaning any prior sessions / memories under the old UUID. The\nnext spawn under the same name lands on a fresh UUID. Use\n`librefang agent delete <name> --yes` for the explicit-by-name variant\n(refs #4614).\n\nExamples:\n  librefang agent kill 550e8400-e29b-41d4-a716-446655440000"
    )]
    Kill {
        /// Agent ID (UUID).
        agent_id: String,
    },
    /// Delete an agent by name with a confirmation prompt (refs #4614).
    #[command(
        long_about = "Permanently delete an agent and purge its canonical UUID binding.\n\nResolves <name> to its canonical UUID via the agent_identities registry,\nprompts for confirmation (or `--yes` to bypass), and issues\n`DELETE /api/agents/{id}?confirm=true`. The next spawn under the same\nname will land on a fresh UUID; prior sessions / memories are orphaned.\n\nExamples:\n  librefang agent delete coder\n  librefang agent delete coder --yes"
    )]
    Delete {
        /// Agent name (looked up in agent_identities.toml).
        name: String,
        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
    /// Reset an agent's canonical UUID without killing it (refs #4614).
    #[command(
        long_about = "Drop the canonical UUID binding for <name> from the\nagent_identities registry without killing the agent. The next spawn\nwill re-derive a fresh UUID. Prior sessions / memories tied to the\nold UUID are orphaned. Prompts for confirmation; pass `--yes` to skip.\n\nExamples:\n  librefang agent reset-uuid coder\n  librefang agent reset-uuid coder --yes"
    )]
    ResetUuid {
        /// Agent name.
        name: String,
        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
    /// Reassign sessions / memories from an old UUID to the canonical one (refs #4614, deferred).
    #[command(
        long_about = "Migrate orphaned data from <old-uuid> to the canonical UUID for <name>.\n\nNOT YET IMPLEMENTED — this command is reserved by issue #4614 but the\nactual cross-table reassignment requires deep memory-substrate surgery\n(sessions, events, kv_store, memories, entities, relations,\nusage_events, canonical_sessions, prompt_experiments, audit_entries,\napproval_audit, plus the proactive memory store) under a single\ntransaction with rollback semantics. Tracked as a follow-up.\n\nFor now this command prints a friendly message pointing to the issue.\n\nExamples:\n  librefang agent merge-history coder --from 0123abcd-...-..."
    )]
    MergeHistory {
        /// Agent name (canonical UUID destination).
        name: String,
        /// Old UUID whose sessions / memories should be reassigned.
        #[arg(long)]
        from: String,
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
pub(crate) enum WorkflowCommands {
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
pub(crate) enum TriggerCommands {
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
pub(crate) enum ModelsCommands {
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
pub(crate) enum GatewayCommands {
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
pub(crate) enum ApprovalsCommands {
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
pub(crate) enum CronCommands {
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
pub(crate) enum SecurityCommands {
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
    AuditReset {
        /// Required. Without this flag the command prints what it would do and exits non-zero.
        #[arg(long)]
        confirm: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum MemoryCommands {
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
        alias = "store",
        long_about = "Store a key-value pair in an agent's memory.\n\nExamples:\n  librefang memory set coder my-key \"hello world\"\n  librefang memory store coder my-key \"hello world\"  # alias for set"
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
pub(crate) enum DevicesCommands {
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
pub(crate) enum WebhooksCommands {
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
pub(crate) enum SystemCommands {
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
pub(crate) enum ServiceCommands {
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
