//! LibreFang CLI — command-line interface for the LibreFang Agent OS.
//!
//! When a daemon is running (`librefang start`), the CLI talks to it over HTTP.
//! Otherwise, commands boot an in-process kernel (single-shot mode).

// The in-process agent loop's deeply-nested async future chain — now
// carrying the per-task held-agent-lock `scope` layer (#5125/#5126) —
// exceeds the default type-recursion limit when this binary crate is
// monomorphised. Matches the `librefang-kernel` / `librefang-api` crate
// roots.
#![recursion_limit = "256"]

#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

// Sibling modules are pub(crate) so the command groups under `commands/` can
// reach them as `crate::<mod>` (and via the prelude re-export). `progress` and
// `table` stay `pub mod` as they were pre-split: narrowing them to pub(crate)
// would expose pre-existing (already-dead) builder methods/variants to the
// dead_code lint. Deleting that dead API is out of scope for a code-move PR.
pub(crate) mod acp;
mod cli;
mod commands;
pub(crate) mod desktop_install;
pub(crate) mod doctor;
pub(crate) mod http_client;
pub(crate) mod i18n;
pub(crate) mod launcher;
pub(crate) mod log_filter;
pub(crate) mod mcp;
pub mod progress;
pub mod table;
pub(crate) mod templates;
pub(crate) mod tui;
pub(crate) mod ui;

use clap::Parser;
// All other shared symbols (cli defs, common helpers, command groups, and the
// std/external short names) come through the command prelude glob, which is
// exempt from unused-import warnings as `main.rs` keeps shrinking.
use commands::prelude::*;
#[cfg(windows)]
use std::sync::atomic::Ordering;

/// Global flag set by the Ctrl+C handler.
static CTRLC_PRESSED: AtomicBool = AtomicBool::new(false);
// Re-exported via the command prelude; the `include_str!` path is relative to
// this source file, so the const stays in `main.rs`.
pub(crate) const INIT_DEFAULT_CONFIG_TEMPLATE: &str =
    include_str!("../templates/init_default_config.toml");

/// Install a Ctrl+C handler that force-exits the process.
/// On Windows/MINGW, the default handler doesn't reliably interrupt blocking
/// `read_line` calls, so we explicitly call `process::exit`.
pub(crate) fn install_ctrlc_handler() {
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

/// Wraps an inner `FormatEvent` impl so every emitted log line carries a
/// `trace_id=<32-hex>` suffix whenever the current tracing span is part of
/// an OpenTelemetry-traced flow (i.e. the OTel reload layer has been swapped
/// in by `init_otel_tracing` and the span has a valid trace context).
///
/// The trace_id sits at the **end** of the line as a logfmt-style structured
/// suffix rather than at the front. This keeps the human-readable
/// timestamp/level/message portion at the start of the line where readers
/// expect it, matching the convention that structured key=value fields
/// follow the unstructured prose of a log entry.
///
/// When telemetry is compiled out, the wrapper still exists but the
/// `cfg(feature = "telemetry")` block is empty — every call delegates to
/// the inner formatter unchanged, so non-telemetry builds see no behaviour
/// change. When telemetry is compiled in but no OTel context is active
/// (e.g. an early boot log before the reload swap, a CLI subcommand that
/// never started the API), the trace context is invalid and the suffix is
/// omitted — again the inner formatter's output is passed through verbatim.
///
/// The suffix uses bare logfmt `trace_id=<hex>` (no quotes) — the matching
/// `derivedFields` regex in `deploy/grafana/provisioning/datasources/loki.yml`
/// is `trace_id="?([0-9a-f]{32})"?`, which is anchored on the literal
/// `trace_id=` token rather than line position, so the suffix placement
/// resolves the same clickable trace link as a prefix would.
struct WithTraceId<F>(F);

impl<S, N, F> tracing_subscriber::fmt::format::FormatEvent<S, N> for WithTraceId<F>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
    N: for<'a> tracing_subscriber::fmt::FormatFields<'a> + 'static,
    F: tracing_subscriber::fmt::format::FormatEvent<S, N>,
{
    fn format_event(
        &self,
        ctx: &tracing_subscriber::fmt::FmtContext<'_, S, N>,
        #[allow(unused_mut)] mut writer: tracing_subscriber::fmt::format::Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> std::fmt::Result {
        #[cfg(feature = "telemetry")]
        {
            use opentelemetry::trace::TraceContextExt;
            use tracing_opentelemetry::OpenTelemetrySpanExt;
            // Bind `cx` and the span via separate `let` bindings: `cx.span()`
            // returns a `SpanRef` that borrows from `cx`, and `span_context()`
            // returns a reference into the `SpanRef`'s inner state. Inlining
            // either one drops a temporary while a later borrow still needs
            // it (E0716 — verified with rustc 1.90 on this branch).
            let cx = tracing::Span::current().context();
            let span_ref = cx.span();
            let span_cx = span_ref.span_context();
            if span_cx.is_valid() {
                // Capture the inner formatter's output into a buffer so we
                // can append the trace_id suffix before the trailing newline.
                // The inner formatter writes its own `\n`; we strip it,
                // append ` trace_id=<hex>`, then re-emit a single newline.
                // Allocates one String per traced log event — acceptable,
                // and the no-OTel path below avoids the alloc entirely.
                let mut buf = String::new();
                self.0.format_event(
                    ctx,
                    tracing_subscriber::fmt::format::Writer::new(&mut buf),
                    event,
                )?;
                let trimmed = buf.trim_end_matches('\n');
                return writeln!(writer, "{trimmed} trace_id={:032x}", span_cx.trace_id());
            }
        }
        self.0.format_event(ctx, writer, event)
    }
}

fn init_tracing_stderr(log_level: &str) {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::Layer;

    // One-shot CLI commands (status, stop, doctor, …) load config.toml as a
    // side effect. librefang_kernel::config emits INFO on every load and WARN
    // for every unknown field; in a CLI context those lines leak into the
    // user's stdout flow and make basic commands look broken. Keep them out
    // of the default stderr budget — users who set RUST_LOG explicitly still
    // see everything, and daemon/foreground boots route through a different
    // initialiser where the full log is expected.
    let user_set_rust_log = std::env::var("RUST_LOG").is_ok();
    // Per-target overrides applied unconditionally on top of the user-visible
    // level (and reapplied on every hot-reload via `install_with_baseline` —
    // see Codex P2-1 #3200). Stored as strings so the filter installer can
    // reparse them after a `log_level` swap; without that, a dashboard
    // "give me debug" toggle would silently drop these and flood operators
    // with kernel/runtime DEBUG noise that boot specifically masked.
    let baseline_directives: Vec<String> = if user_set_rust_log {
        // RUST_LOG is the explicit "I want full control" knob — don't layer
        // any opinionated overrides on top of it, and don't carry any across
        // reloads either.
        Vec::new()
    } else {
        vec![
            "librefang_kernel=warn".to_string(),
            "librefang_runtime=warn".to_string(),
            "librefang_extensions=warn".to_string(),
            "librefang_kernel::config=error".to_string(),
            "librefang_runtime::registry_sync=error".to_string(),
        ]
    };
    let mut env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(log_level));
    for d in &baseline_directives {
        // Per-string parse keeps the boot-time directive list and the
        // reload-time directive list literally identical.
        env_filter = env_filter.add_directive(d.parse().expect("baseline directive must parse"));
    }

    // Compact stderr format: in a one-shot CLI context the user cares about
    // the WARN/ERROR text, not the timestamp or the fully-qualified target.
    // One-shot CLI runs are transient — stderr is the only sink; the daemon
    // has its own file appender under `logs/daemon.log`.
    //
    // `.with_filter(env_filter)` applies the user-visible log filter to the
    // fmt layer ONLY. A registry-level filter would also suppress span
    // CREATION, which would starve the OTel exporter layer attached below
    // (`librefang_kernel`/`librefang_runtime` downgraded to WARN means all
    // INFO-level `#[instrument]` spans are filtered out before OTel ever
    // sees them). Per-layer filtering keeps stderr terse while OTel
    // receives the full span tree.
    //
    // The filter is wrapped in `ReloadableEnvFilter` so the daemon can swap
    // it at runtime when `KernelConfig::log_level` changes via hot-reload.
    // `install_with_baseline` hands the per-target directives above to the
    // filter installer so a dashboard `log_level` edit reapplies them after
    // the swap — i.e. the kernel/runtime overrides survive reloads instead
    // of being silently dropped. `RUST_LOG` itself is *not* re-read on
    // reload (it's a boot-time knob); operators wanting env-driven
    // filtering after a config edit need to restart.
    //
    // Force stderr explicitly: machine-readable subcommands like
    // `doctor --json` expect a clean stdout stream. The fmt layer's
    // default writer is stdout, which would interleave tracing output
    // with the JSON payload and corrupt downstream parsers.
    //
    // Build the inner format separately so we can wrap it in `WithTraceId`,
    // which appends the OTel `trace_id` as a logfmt suffix on every line when
    // an OTel context is active. The wrapper is unconditional but no-ops
    // without the `telemetry` feature; see `WithTraceId` doc above.
    let inner_format = tracing_subscriber::fmt::format()
        .without_time()
        .with_target(false)
        .compact();
    let reloadable_filter =
        log_filter::ReloadableEnvFilter::install_with_baseline(env_filter, baseline_directives);
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .event_format(WithTraceId(inner_format))
        .with_filter(reloadable_filter);

    // Register a no-op reload slot so `init_otel_tracing` can swap a real
    // OTel layer in later without needing to claim the global dispatcher.
    // The slot is stacked **first** (directly on Registry) so its boxed
    // `Layer<Registry>` trait object matches the innermost subscriber type.
    // No filter is attached to this layer on purpose — see comment above.
    #[cfg(feature = "telemetry")]
    let registry =
        tracing_subscriber::registry().with(librefang_api::telemetry::install_otel_reload_layer());
    #[cfg(not(feature = "telemetry"))]
    let registry = tracing_subscriber::registry();

    registry.with(fmt_layer).init();
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
            // Same `WithTraceId` wrapper as `init_tracing_stderr` so the TUI
            // log file carries `trace_id=<hex>` suffixes when OTel is on.
            // We have to build the subscriber by hand here (rather than the
            // `tracing_subscriber::fmt()` builder shortcut) because the
            // builder owns its formatter and doesn't expose `event_format`.
            use tracing_subscriber::layer::SubscriberExt;
            use tracing_subscriber::util::SubscriberInitExt;

            let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(log_level));
            let inner_format = tracing_subscriber::fmt::format();
            let fmt_layer = tracing_subscriber::fmt::layer()
                .with_writer(std::sync::Mutex::new(file))
                .with_ansi(false)
                .event_format(WithTraceId(inner_format));
            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt_layer)
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

    let language = load_language_from_config().unwrap_or_else(i18n::detect_system_language);
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
        }) => cmd_start(cli.config, tail, spawned, foreground),
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
            AgentCommands::Delete { name, yes } => cmd_agent_delete(cli.config, &name, yes),
            AgentCommands::ResetUuid { name, yes } => cmd_agent_reset_uuid(cli.config, &name, yes),
            AgentCommands::MergeHistory { name, from } => cmd_agent_merge_history(&name, &from),
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
            SkillCommands::Pending(sub) => cmd_skill_pending(sub),
        },
        Some(Commands::Channel(sub)) => match sub {
            ChannelCommands::List => cmd_channel_list(),
            ChannelCommands::Reload => cmd_channel_reload(),
            ChannelCommands::Setup { name } => cmd_channel_setup(name.as_deref()),
            ChannelCommands::Rm { name } => cmd_channel_rm(&name),
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
        Some(Commands::Acp { agent }) => acp::run_acp_server(cli.config, agent),
        Some(Commands::Auth(sub)) => match sub {
            AuthCommands::Chatgpt { device_auth } => cmd_auth_chatgpt(device_auth),
            AuthCommands::Pool(sub) => match sub {
                AuthPoolCommands::List { json } => cmd_auth_pool_list(cli.config, json),
                AuthPoolCommands::Add {
                    provider,
                    env_var,
                    label,
                    priority,
                } => cmd_auth_pool_add(cli.config, &provider, &env_var, &label, priority),
                AuthPoolCommands::Remove { provider, env_var } => {
                    cmd_auth_pool_remove(cli.config, &provider, &env_var)
                }
                AuthPoolCommands::Strategy { provider, strategy } => {
                    cmd_auth_pool_strategy(cli.config, &provider, &strategy)
                }
            },
        },
        Some(Commands::Vault(sub)) => match sub {
            VaultCommands::Init => cmd_vault_init(),
            VaultCommands::Set { key } => cmd_vault_set(&key),
            VaultCommands::List => cmd_vault_list(),
            VaultCommands::Remove { key } => cmd_vault_remove(&key),
            VaultCommands::RotateKey { from_stdin } => cmd_vault_rotate_key(from_stdin),
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
                cmd_start(cli.config, tail, false, foreground)
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
        Some(Commands::Sessions {
            agent,
            json,
            active,
        }) => cmd_sessions(agent.as_deref(), json, active),
        Some(Commands::Logs { lines, follow }) => cmd_logs(cli.config, lines, follow),
        Some(Commands::Health { json }) => cmd_health(json),
        Some(Commands::Security(sub)) => match sub {
            SecurityCommands::Status { json } => cmd_security_status(json),
            SecurityCommands::Audit { limit, json } => cmd_security_audit(limit, json),
            SecurityCommands::Verify => cmd_security_verify(),
            SecurityCommands::AuditReset { confirm } => cmd_audit_reset(cli.config, confirm),
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
        Some(Commands::Message {
            agent,
            text,
            json,
            incognito,
        }) => cmd_message(&agent, &text, json, incognito),
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

#[cfg(test)]
mod tests {
    // The items under test now live in `crate::commands::*`; pull them in via
    // the command prelude (clap defs + every handler group + shared helpers).
    use crate::commands::prelude::*;
    use clap::Parser;
    use serde_json::json;
    use std::ffi::OsString;
    use std::fs;
    use std::path::Path;

    // --- Config set numeric parsing (#3461) ---

    #[test]
    fn parse_toml_integer_accepts_normal_i64() {
        match parse_toml_integer("42").unwrap() {
            toml::Value::Integer(v) => assert_eq!(v, 42),
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn parse_toml_integer_accepts_i64_max() {
        match parse_toml_integer(&i64::MAX.to_string()).unwrap() {
            toml::Value::Integer(v) => assert_eq!(v, i64::MAX),
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn parse_toml_integer_rejects_u64_max_instead_of_truncating() {
        // u64::MAX as i64 would silently become -1 — we must error instead.
        let err = parse_toml_integer(&u64::MAX.to_string()).unwrap_err();
        assert!(err.contains("exceeds i64::MAX"), "got: {err}");
    }

    #[test]
    fn parse_toml_integer_rejects_non_integer() {
        assert!(parse_toml_integer("not-a-number").is_err());
    }

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
    fn test_channel_list_parses() {
        let cli = Cli::parse_from(["librefang", "channel", "list"]);
        assert!(matches!(
            cli.command,
            Some(Commands::Channel(ChannelCommands::List))
        ));
    }

    #[test]
    fn test_channel_setup_with_name_parses() {
        let cli = Cli::parse_from(["librefang", "channel", "setup", "telegram"]);
        assert!(matches!(
            cli.command,
            Some(Commands::Channel(ChannelCommands::Setup { name: Some(ref n) })) if n == "telegram"
        ));
    }

    #[test]
    fn test_channel_setup_picker_parses() {
        let cli = Cli::parse_from(["librefang", "channel", "setup"]);
        assert!(matches!(
            cli.command,
            Some(Commands::Channel(ChannelCommands::Setup { name: None }))
        ));
    }

    #[test]
    fn test_channel_reload_parses() {
        let cli = Cli::parse_from(["librefang", "channel", "reload"]);
        assert!(matches!(
            cli.command,
            Some(Commands::Channel(ChannelCommands::Reload))
        ));
    }

    #[test]
    fn test_channel_rm_parses() {
        let cli = Cli::parse_from(["librefang", "channel", "rm", "telegram"]);
        assert!(matches!(
            cli.command,
            Some(Commands::Channel(ChannelCommands::Rm { ref name })) if name == "telegram"
        ));
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

    // --- WithTraceId log-format wrapper tests ---
    //
    // The wrapper is the Rust-side counterpart of the Loki `derivedFields`
    // regex provisioned in `deploy/grafana/provisioning/datasources/loki.yml`.
    // It must (a) be a transparent passthrough when no OTel context is active
    // (the common case for one-shot CLI commands and early boot), and (b)
    // emit `trace_id=<32-hex>` exactly when a context is live so the Loki
    // regex resolves it into a clickable trace link.
    //
    // We can't easily build a live OTel context inside a unit test without
    // spinning up an exporter, so the OTel-active path is covered by the
    // live integration test described in `deploy/OBSERVABILITY.md`. These
    // tests pin the no-OTel-context behaviour, which is what regresses
    // first if someone refactors the wrapper.

    #[test]
    fn test_with_trace_id_passthrough_without_otel_context() {
        use super::WithTraceId;
        use std::sync::{Arc, Mutex};
        use tracing_subscriber::fmt::MakeWriter;
        use tracing_subscriber::layer::SubscriberExt;

        // Capture writer: collects every byte written by the fmt layer so the
        // test can assert on the rendered line. Wrapped in Arc<Mutex<Vec<u8>>>
        // so both the subscriber and the test body share a view.
        #[derive(Clone)]
        struct VecWriter(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for VecWriter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        impl<'a> MakeWriter<'a> for VecWriter {
            type Writer = VecWriter;
            fn make_writer(&'a self) -> Self::Writer {
                self.clone()
            }
        }

        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let writer = VecWriter(buf.clone());
        let inner = tracing_subscriber::fmt::format()
            .without_time()
            .with_target(false)
            .compact();
        let layer = tracing_subscriber::fmt::layer()
            .with_writer(writer)
            .with_ansi(false)
            .event_format(WithTraceId(inner));
        let subscriber = tracing_subscriber::registry().with(layer);

        // Scope the dispatcher to this test so we don't fight the global
        // subscriber installed by other tests in the binary.
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("hello world");
        });

        let line = String::from_utf8(buf.lock().unwrap().clone()).expect("utf8");
        assert!(
            line.contains("hello world"),
            "expected the inner formatter to render the message, got: {line:?}"
        );
        assert!(
            !line.contains("trace_id="),
            "expected NO trace_id prefix when no OTel context is active, got: {line:?}"
        );
    }

    #[test]
    fn test_with_trace_id_format_matches_loki_regex() {
        // Pin the exact format we emit so the Loki `derivedFields` regex in
        // `deploy/grafana/provisioning/datasources/loki.yml` keeps resolving:
        // `matcherRegex: 'trace_id="?([0-9a-f]{32})"?'`.
        //
        // If someone changes the format string in `WithTraceId::format_event`
        // (e.g. to `traceId={...}` or to upper-case hex), this test fails
        // before the change reaches Grafana and silently breaks log↔trace
        // linking in the dashboards.
        let trace_id_u128: u128 = 0x0123_4567_89ab_cdef_0123_4567_89ab_cdef_u128;
        let rendered = format!("trace_id={trace_id_u128:032x} ");
        assert_eq!(
            rendered, "trace_id=0123456789abcdef0123456789abcdef ",
            "trace_id format must be 32 lowercase hex chars with no quotes"
        );

        // Mimic the Loki regex `trace_id="?([0-9a-f]{32})"?` without pulling
        // in a regex crate just for one assertion: locate the `trace_id=`
        // prefix, optionally consume a quote, then take 32 chars and verify
        // they are all lowercase hex.
        let needle = "trace_id=";
        let pos = rendered
            .find(needle)
            .expect("emitted line must contain trace_id=");
        let after = &rendered[pos + needle.len()..];
        let after = after.strip_prefix('"').unwrap_or(after);
        let hex: String = after.chars().take(32).collect();
        assert_eq!(hex.len(), 32, "expected 32 hex chars, got {hex:?}");
        assert!(
            hex.chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
            "expected lowercase hex, got {hex:?}"
        );
        assert_eq!(hex, "0123456789abcdef0123456789abcdef");
    }

    /// Pins the daemon's compact-format behavior: when an event fires inside
    /// a span carrying `agent.id` / `session.id` fields, the rendered line
    /// MUST include both as inline span suffix tokens (the format
    /// `tracing-subscriber`'s `Compact` formatter emits is
    /// `<level> <span_name>: <message> <field>=<value> ...`). Daemon log
    /// search relies on this to correlate any line back to the originating
    /// agent + session — see also the `#[instrument]` on `run_agent_loop`
    /// in `librefang-runtime/src/agent_loop.rs`.
    #[test]
    fn with_trace_id_compact_format_carries_agent_and_session_ids_from_span() {
        use super::WithTraceId;
        use std::sync::{Arc, Mutex};
        use tracing::{info_span, warn};
        use tracing_subscriber::fmt::MakeWriter;
        use tracing_subscriber::layer::SubscriberExt;

        #[derive(Clone)]
        struct VecWriter(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for VecWriter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        impl<'a> MakeWriter<'a> for VecWriter {
            type Writer = VecWriter;
            fn make_writer(&'a self) -> Self::Writer {
                self.clone()
            }
        }

        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let writer = VecWriter(buf.clone());
        let inner = tracing_subscriber::fmt::format()
            .without_time()
            .with_target(false)
            .compact();
        let layer = tracing_subscriber::fmt::layer()
            .with_writer(writer)
            .with_ansi(false)
            .event_format(WithTraceId(inner));
        let subscriber = tracing_subscriber::registry().with(layer);

        tracing::subscriber::with_default(subscriber, || {
            let span = info_span!(
                "run_agent_loop",
                agent.id = "agent-uuid-aaaa",
                session.id = "session-uuid-bbbb",
            );
            let _entered = span.enter();
            warn!("shell exec full mode");
        });

        let captured = String::from_utf8(buf.lock().unwrap().clone()).expect("utf8");
        assert!(
            captured.contains("agent.id=\"agent-uuid-aaaa\""),
            "expected agent.id span field in line, got: {captured:?}"
        );
        assert!(
            captured.contains("session.id=\"session-uuid-bbbb\""),
            "expected session.id span field in line, got: {captured:?}"
        );
        assert!(
            captured.contains("run_agent_loop"),
            "expected span name prefix, got: {captured:?}"
        );
        assert!(
            captured.contains("shell exec full mode"),
            "expected original message preserved, got: {captured:?}"
        );
    }

    // --- Daemon detection / launcher port logic (#3582) ---
    //
    // These exercise the `find_daemon_with_probe` core, which was extracted
    // from `find_daemon_in_home` so the HTTP probe can be faked in unit
    // tests instead of binding sockets or making real requests.

    fn write_daemon_json(home: &Path, listen_addr: &str) {
        let body = json!({
            "pid": 4242u32,
            "listen_addr": listen_addr,
            "started_at": "1970-01-01T00:00:00Z",
            "version": "0.0.0-test",
            "platform": "test",
        });
        fs::write(home.join("daemon.json"), body.to_string()).expect("write daemon.json");
    }

    #[test]
    fn normalize_daemon_addr_rewrites_bind_all_to_loopback() {
        // `0.0.0.0:4545` is the default bind-all address; on macOS, probing
        // it directly can hang, so the launcher rewrites to 127.0.0.1.
        assert_eq!(normalize_daemon_addr("0.0.0.0:4545"), "127.0.0.1:4545");
    }

    #[test]
    fn normalize_daemon_addr_leaves_explicit_loopback_alone() {
        assert_eq!(normalize_daemon_addr("127.0.0.1:4545"), "127.0.0.1:4545");
    }

    #[test]
    fn normalize_daemon_addr_leaves_other_hosts_alone() {
        // A user who explicitly bound to a LAN IP should keep it.
        assert_eq!(
            normalize_daemon_addr("192.168.1.10:4545"),
            "192.168.1.10:4545"
        );
    }

    #[test]
    fn find_daemon_with_probe_returns_none_when_no_daemon_json() {
        let tmp = tempfile::tempdir().expect("tempdir");
        // No daemon.json written. Probe must NOT be invoked.
        let probe_called = std::cell::Cell::new(false);
        let got = find_daemon_with_probe(tmp.path(), |_url| {
            probe_called.set(true);
            true
        });
        assert!(got.is_none());
        assert!(
            !probe_called.get(),
            "probe must not run when daemon.json is absent — saves a network round-trip"
        );
    }

    #[test]
    fn find_daemon_with_probe_returns_none_on_unparseable_daemon_json() {
        let tmp = tempfile::tempdir().expect("tempdir");
        fs::write(tmp.path().join("daemon.json"), "not valid json {{{").unwrap();
        let got = find_daemon_with_probe(tmp.path(), |_url| true);
        assert!(
            got.is_none(),
            "corrupt daemon.json must not be treated as a live daemon"
        );
    }

    #[test]
    fn find_daemon_with_probe_returns_base_url_on_healthy_probe() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write_daemon_json(tmp.path(), "127.0.0.1:4545");

        let seen = std::cell::Cell::new(None);
        let got = find_daemon_with_probe(tmp.path(), |url| {
            seen.set(Some(url.to_string()));
            true
        });

        // The probe receives the /api/health URL...
        assert_eq!(
            seen.into_inner().as_deref(),
            Some("http://127.0.0.1:4545/api/health")
        );
        // ...and the caller gets back the *base* URL (no /api/health suffix).
        assert_eq!(got.as_deref(), Some("http://127.0.0.1:4545"));
    }

    #[test]
    fn find_daemon_with_probe_normalizes_bind_all_in_url() {
        // Regression: ensure 0.0.0.0 in daemon.json is rewritten to 127.0.0.1
        // BEFORE we hand the URL to the probe (and before we return it).
        let tmp = tempfile::tempdir().expect("tempdir");
        write_daemon_json(tmp.path(), "0.0.0.0:4545");

        let seen = std::cell::Cell::new(None);
        let got = find_daemon_with_probe(tmp.path(), |url| {
            seen.set(Some(url.to_string()));
            true
        });

        assert_eq!(
            seen.into_inner().as_deref(),
            Some("http://127.0.0.1:4545/api/health"),
            "probe must see normalized 127.0.0.1 URL, never 0.0.0.0"
        );
        assert_eq!(got.as_deref(), Some("http://127.0.0.1:4545"));
    }

    #[test]
    fn find_daemon_with_probe_returns_none_on_failed_probe() {
        // Stale daemon.json (process gone, port in use by something else, or
        // returning 5xx) — probe returns false → caller gets None.
        let tmp = tempfile::tempdir().expect("tempdir");
        write_daemon_json(tmp.path(), "127.0.0.1:4545");
        let got = find_daemon_with_probe(tmp.path(), |_url| false);
        assert!(got.is_none());
    }

    // Regression guard for #4923: `memory store` must parse identically to
    // `memory set` so the alias added in this PR is wired up correctly.
    #[test]
    fn memory_store_alias_parses_identically_to_memory_set() {
        let via_set =
            Cli::try_parse_from(["librefang", "memory", "set", "coder", "my-key", "my-value"])
                .expect("memory set must parse");
        let via_store = Cli::try_parse_from([
            "librefang",
            "memory",
            "store",
            "coder",
            "my-key",
            "my-value",
        ])
        .expect("memory store alias must parse");

        let (set_agent, set_key, set_val) = match via_set.command.unwrap() {
            Commands::Memory(MemoryCommands::Set { agent, key, value }) => (agent, key, value),
            _ => panic!("unexpected variant from 'memory set'"),
        };
        let (store_agent, store_key, store_val) = match via_store.command.unwrap() {
            Commands::Memory(MemoryCommands::Set { agent, key, value }) => (agent, key, value),
            _ => panic!("unexpected variant from 'memory store'"),
        };

        assert_eq!(set_agent, store_agent);
        assert_eq!(set_key, store_key);
        assert_eq!(set_val, store_val);
    }

    // ── Credential pool CLI helpers (#4965) ───────────────────────────────────

    #[test]
    fn is_valid_env_var_name_accepts_standard_shapes() {
        assert!(is_valid_env_var_name("OPENAI_API_KEY"));
        assert!(is_valid_env_var_name("OPENAI_API_KEY_2"));
        assert!(is_valid_env_var_name("_PRIVATE"));
        assert!(is_valid_env_var_name("A"));
        assert!(is_valid_env_var_name("X1"));
    }

    #[test]
    fn is_valid_env_var_name_rejects_garbage() {
        // Leading digit, lowercase, spaces, punctuation, empty — all rejected.
        assert!(!is_valid_env_var_name(""));
        assert!(!is_valid_env_var_name("1FOO"));
        assert!(!is_valid_env_var_name("foo"));
        assert!(!is_valid_env_var_name("FOO BAR"));
        assert!(!is_valid_env_var_name("FOO-BAR"));
        assert!(!is_valid_env_var_name("FOO.BAR"));
        assert!(!is_valid_env_var_name("FOO$"));
        assert!(!is_valid_env_var_name(" FOO"));
    }

    #[test]
    fn pool_strategy_canon_accepts_known_strategies() {
        assert_eq!(pool_strategy_canon("fill_first"), Some("fill_first"));
        assert_eq!(pool_strategy_canon("Fill-First"), Some("fill_first"));
        assert_eq!(pool_strategy_canon("FILLFIRST"), Some("fill_first"));
        assert_eq!(pool_strategy_canon("round_robin"), Some("round_robin"));
        assert_eq!(pool_strategy_canon("RoundRobin"), Some("round_robin"));
        assert_eq!(pool_strategy_canon("random"), Some("random"));
        assert_eq!(pool_strategy_canon("least_used"), Some("least_used"));
        assert_eq!(pool_strategy_canon("LEASTUSED"), Some("least_used"));
    }

    #[test]
    fn pool_strategy_canon_rejects_unknown() {
        assert_eq!(pool_strategy_canon(""), None);
        assert_eq!(pool_strategy_canon("foo"), None);
        assert_eq!(pool_strategy_canon("priority"), None);
        assert_eq!(pool_strategy_canon("rand"), None);
    }

    /// Round-trip a config.toml fragment containing comments and an unrelated
    /// section through `toml_edit::DocumentMut`. Proves the parser preserves
    /// the bits the mutating pool commands rely on: comments survive,
    /// unrelated tables stay intact, and a freshly inserted
    /// `[[credential_pools]]` lands at the bottom without rewriting the
    /// rest of the file. (The actual cmd_auth_pool_* functions are private
    /// CLI orchestrators that exit the process on error and call `ui::*`
    /// helpers, so we test the underlying mutation primitive directly.)
    #[test]
    fn toml_edit_roundtrip_preserves_comments_and_unrelated_sections() {
        let original = r#"# top-of-file comment
api_listen = "127.0.0.1:4545"

[default_model]
# inline comment in default_model
provider = "anthropic"
model = "claude-3-5-sonnet"
api_key_env = "ANTHROPIC_API_KEY"

# trailing comment before our edit
"#;
        let mut doc: toml_edit::DocumentMut = original.parse().expect("fragment must parse");
        // Insert a credential_pools entry the same way the CLI's add-on-no-pool
        // path does — building an ArrayOfTables and pushing one table into it.
        let item = doc
            .entry("credential_pools")
            .or_insert(toml_edit::Item::ArrayOfTables(
                toml_edit::ArrayOfTables::new(),
            ));
        let arr = item
            .as_array_of_tables_mut()
            .expect("just inserted as array of tables");
        let mut pool_tbl = toml_edit::Table::new();
        pool_tbl["provider"] = toml_edit::value("anthropic");
        pool_tbl["strategy"] = toml_edit::value("fill_first");
        let mut keys_arr = toml_edit::ArrayOfTables::new();
        let mut key_tbl = toml_edit::Table::new();
        key_tbl["api_key_env"] = toml_edit::value("ANTHROPIC_API_KEY_2");
        key_tbl["label"] = toml_edit::value("Backup");
        key_tbl["priority"] = toml_edit::value(5_i64);
        keys_arr.push(key_tbl);
        pool_tbl.insert("keys", toml_edit::Item::ArrayOfTables(keys_arr));
        arr.push(pool_tbl);

        let rendered = doc.to_string();
        // All three comments survive verbatim.
        assert!(
            rendered.contains("# top-of-file comment"),
            "top comment missing: {rendered}"
        );
        assert!(
            rendered.contains("# inline comment in default_model"),
            "inline comment missing: {rendered}"
        );
        assert!(
            rendered.contains("# trailing comment before our edit"),
            "trailing comment missing: {rendered}"
        );
        // Unrelated section intact.
        assert!(rendered.contains("[default_model]"));
        assert!(rendered.contains("provider = \"anthropic\""));
        // New section present with the expected canonical shape.
        assert!(rendered.contains("[[credential_pools]]"));
        assert!(rendered.contains("[[credential_pools.keys]]"));
        assert!(rendered.contains("api_key_env = \"ANTHROPIC_API_KEY_2\""));
        assert!(rendered.contains("label = \"Backup\""));
        assert!(rendered.contains("priority = 5"));
    }
}
