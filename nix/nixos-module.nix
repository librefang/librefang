# NixOS module for the LibreFang daemon — `services.librefang`.
#
# Two supported ways in, and `services.librefang.package` resolves for both:
#
#   * The flake's `nixosModules.default` / `nixosModules.librefang` wrapper imports this file and sets `services.librefang.package` to the flake's own `librefang-cli` with `lib.mkDefault`, so importing the module is enough.
#   * Importing this file directly (vendored, or from a non-flake config) works as long as `overlays.default` is applied to the package set, which is what puts `pkgs.librefang-cli` in scope for the option default below.
#
# Deliberately not a `_module.args` argument: the module system resolves module arguments through `config._module.args` and ignores a Nix-level default on the parameter, so a `librefangPackages ? { }` parameter fails outright on the direct-import path rather than falling back.
{ config, lib, pkgs, ... }:

let
  cfg = config.services.librefang;

  defaultUser = "librefang";
  defaultStateDir = "/var/lib/librefang";

  # `StateDirectory=` takes a path relative to `/var/lib`, so it can express the default state dir (and any other location under `/var/lib`) but not an arbitrary absolute override.
  # Relocated state dirs get a tmpfiles rule plus `ReadWritePaths=` instead, since `ProtectSystem=strict` mounts everything outside the declared write set read-only.
  stateDirUnderVarLib = lib.hasPrefix "/var/lib/" cfg.stateDir;
  stateDirectoryName = lib.removePrefix "/var/lib/" cfg.stateDir;

  # The trees `ProtectHome = true` covers, per systemd's documented behaviour for that directive: `/home`, `/root` and `/run/user`.
  # The bare directory names themselves are matched separately, because a prefix match on `/home/` does not catch `/root` used verbatim as the state dir.
  protectHomePrefixes = [ "/home/" "/root/" "/run/user/" ];
  stateDirUnderProtectedHome =
    lib.any (prefix: lib.hasPrefix prefix (toString cfg.stateDir)) protectHomePrefixes
    || lib.elem (toString cfg.stateDir) [ "/root" "/run/user" ];

  # The bind address the daemon will actually listen on.
  # `KernelConfig::default()` starts from `DEFAULT_API_LISTEN = "127.0.0.1:4545"` (crates/librefang-types/src/config/mod.rs:25, applied at crates/librefang-types/src/config/types.rs:6362) and `Kernel::boot_with_config` overrides `config.api_listen` from `LIBREFANG_LISTEN` when that variable is set (crates/librefang-kernel/src/kernel/boot.rs:47-49).
  # `cmd_start` then hands the booted kernel's `api_listen` straight to `run_daemon` (crates/librefang-cli/src/commands/daemon.rs:481-482,518-519), so the env var — not a generated `config.toml` — is the supported way for a unit to pin the port.
  # Managing `config.toml` from the module is deliberately avoided, because the daemon writes that file itself: the boot-time MCP migrator rewrites it whenever it actually migrates something (crates/librefang-runtime/src/mcp_migrate.rs:383, reached from crates/librefang-kernel/src/kernel/boot.rs:985), and config edits made through the API or the dashboard are written back by several handlers through `crate::atomic_write` (crates/librefang-api/src/routes/config/manage.rs:1297, crates/librefang-api/src/routes/budget.rs:669, crates/librefang-api/src/routes/providers.rs:2579,2826,2871).
  # A read-only store path would break both.
  listenAddress = "127.0.0.1:${toString cfg.port}";

  # An operator can still point the daemon at a non-loopback bind through `extraEnvironment.LIBREFANG_LISTEN`.
  # That path needs its own safety check — see the assertion below.
  effectiveListen = cfg.extraEnvironment.LIBREFANG_LISTEN or listenAddress;

  # `LIBREFANG_LISTEN` is `host:port`, with an IPv6 host wrapped in brackets.
  listenHost =
    if lib.hasPrefix "[" effectiveListen
    then lib.removePrefix "[" (lib.head (lib.splitString "]" effectiveListen))
    else lib.head (lib.splitString ":" effectiveListen);

  # Mirrors the loopback set `evaluate_bind_auth_safety` treats as safe (crates/librefang-api/src/server.rs:312-330).
  listenIsLoopback =
    listenHost == "127.0.0.1" || listenHost == "::1" || listenHost == "localhost";

  environmentIsInStore = path: lib.hasPrefix builtins.storeDir (toString path);
in
{
  options.services.librefang = {
    enable = lib.mkEnableOption "the LibreFang agent operating system daemon";

    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.librefang-cli or (throw ''
        services.librefang.package has no default in this evaluation: `pkgs.librefang-cli` is not in scope.
        Import the LibreFang flake's `nixosModules.default` (which supplies the flake's own build), or apply its `overlays.default` to your nixpkgs, or set `services.librefang.package` explicitly.
      '');
      defaultText = lib.literalMD "`pkgs.librefang-cli`, or the flake's own `librefang-cli` when imported through `nixosModules.default`";
      description = ''
        Package providing the `librefang` binary.
        The binary name is `librefang`, not `librefang-cli` (crates/librefang-cli/Cargo.toml:35-37).
      '';
    };

    port = lib.mkOption {
      type = lib.types.port;
      default = 4545;
      description = ''
        TCP port the API server and dashboard listen on, exported to the daemon as `LIBREFANG_LISTEN=127.0.0.1:<port>`.
        The bind host stays on loopback; override `extraEnvironment.LIBREFANG_LISTEN` to change it, and read the `openFirewall` description first.
      '';
    };

    openFirewall = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = ''
        Open {option}`services.librefang.port` in the host firewall.
        This module binds the daemon to loopback only, so on its own the hole reaches nothing — it is useful in combination with an `extraEnvironment.LIBREFANG_LISTEN` override that binds a routable address.
        A non-loopback bind with no authentication configured makes the daemon refuse to start (crates/librefang-api/src/server.rs:1915 calling `check_bind_auth_safety`, crates/librefang-api/src/server.rs:312-330), so pair it with an API key in {option}`services.librefang.environmentFile`.
      '';
    };

    user = lib.mkOption {
      type = lib.types.str;
      default = defaultUser;
      description = ''
        User account the daemon runs as.
        The module declares this account only when it is left at the default `${defaultUser}`; any other value must be declared by the operator, with `home` set to {option}`services.librefang.stateDir`.
      '';
    };

    group = lib.mkOption {
      type = lib.types.str;
      default = defaultUser;
      description = ''
        Group the daemon runs as, declared by this module only when left at the default `${defaultUser}`.
      '';
    };

    stateDir = lib.mkOption {
      type = lib.types.path;
      default = defaultStateDir;
      description = ''
        Directory holding everything the daemon writes: `config.toml`, `data/`, `workspaces/`, `skills/`, `hooks/`, `registry/`, `logs/`, `vault.enc`, `daemon.json`, the `daemon.lock` flock, and a git repo at its root.
        Exported as `LIBREFANG_HOME`, which `librefang_home()` reads ahead of the user's home directory (crates/librefang-kernel/src/config.rs:535-542).
        Must be a directory dedicated to LibreFang, with no trailing slash — a path under `/var/lib` is created by `StateDirectory=`, anything else by a `systemd.tmpfiles` rule that chowns it to {option}`services.librefang.user`.
      '';
    };

    environmentFile = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      example = "/run/secrets/librefang.env";
      description = ''
        Path to a systemd `EnvironmentFile` holding provider API keys and other secrets, one `KEY=value` per line.
        systemd reads the file as root before dropping privileges, so it can be mode `0400` and owned by root — unlike a Nix store path, which is world-readable to every user on the machine.
        Point this at a secret produced out-of-band (`sops-nix`, `agenix`, a manually installed file under `/run` or `/etc`); an assertion rejects store paths.
      '';
    };

    extraEnvironment = lib.mkOption {
      type = lib.types.attrsOf lib.types.str;
      default = { };
      example = lib.literalExpression ''
        {
          LIBREFANG_LISTEN = "0.0.0.0:4545";
          RUST_LOG = "info";
        }
      '';
      description = ''
        Extra environment variables for the daemon unit, merged after the variables this module sets.
        Do not put secrets here — unit environment blocks land in the world-readable Nix store; use {option}`services.librefang.environmentFile` instead.
      '';
    };

    authConfiguredExternally = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = ''
        Assert that authentication is configured outside this module — an `api_key`, a `[[users]]` entry with `api_key_hash`, or dashboard credentials in `<stateDir>/config.toml`, which this module deliberately does not manage.
        No environment variable can set `api_key`, so an {option}`services.librefang.environmentFile` holding only provider keys does not satisfy the daemon's boot-time check; this option is the escape hatch for a `config.toml` maintained out-of-band.
        It relaxes the non-loopback bind assertion below and nothing else: the daemon still runs its own check at startup and refuses to start if the claim is false.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = cfg.environmentFile == null || !(environmentIsInStore cfg.environmentFile);
        message = ''
          services.librefang.environmentFile points into the Nix store (${toString cfg.environmentFile}), which is world-readable to every local user.
          Provider API keys must come from a file outside the store — generate it with sops-nix / agenix, or install it under /run or /etc with mode 0400.
        '';
      }
      {
        # The value check mirrors `allow_no_auth_env()` (crates/librefang-api/src/server.rs), which accepts only these five spellings.
        # A presence check would let `LIBREFANG_ALLOW_NO_AUTH = "0"` pass evaluation and then be rejected at boot.
        assertion = listenIsLoopback
          || cfg.environmentFile != null
          || cfg.authConfiguredExternally
          || (lib.elem (cfg.extraEnvironment.LIBREFANG_ALLOW_NO_AUTH or "") [ "1" "true" "TRUE" "yes" "on" ]);
        message = ''
          services.librefang binds the non-loopback address ${effectiveListen} with no authentication source configured, and the daemon refuses to start in that configuration (`check_bind_auth_safety` / `any_auth_configured` in crates/librefang-api/src/server.rs).
          No environment variable feeds `api_key` — it is read from <stateDir>/config.toml only — so the only authentication the daemon picks up from the unit environment is a dashboard credential pair: have services.librefang.environmentFile export LIBREFANG_DASHBOARD_USER and LIBREFANG_DASHBOARD_PASS.
          If authentication is already configured in <stateDir>/config.toml, which this module does not manage, set services.librefang.authConfiguredExternally = true; to run intentionally open, set extraEnvironment.LIBREFANG_ALLOW_NO_AUTH = "1".
        '';
      }
      {
        # Both consumers of `stateDir` need a dedicated directory with a real final component, and neither failure surfaces during evaluation.
        # A trailing slash leaves `StateDirectory=` empty and systemd rejects the unit at load time — long after `nixos-rebuild` reported success — while a shared FHS parent would have the tmpfiles rule below chown that parent to the service user.
        assertion = lib.last (lib.splitString "/" (toString cfg.stateDir)) != ""
          && !(lib.elem (toString cfg.stateDir) [ "/var" "/var/lib" "/srv" "/opt" "/etc" "/usr" "/home" ]);
        message = ''
          services.librefang.stateDir must be a dedicated directory with no trailing slash, and got ${toString cfg.stateDir}.
          It becomes StateDirectory= when it sits under /var/lib and a systemd-tmpfiles rule anywhere else; an empty final component makes systemd refuse to load the unit, and a shared parent would be chowned to ${cfg.user}.
        '';
      }
      {
        # The unit below sets `ProtectHome = true` unconditionally, and systemd documents that directive as making `/home`, `/root` and `/run/user` inaccessible and empty for the unit's processes — so none of those trees is a location this service can use for its state dir.
        # Evaluation is the only place to catch it: the tmpfiles rule and the `ReadWritePaths=` entry are both generated for a relocated state dir without complaint.
        assertion = !stateDirUnderProtectedHome;
        message = ''
          services.librefang.stateDir is ${toString cfg.stateDir}, which sits in a tree this unit's ProtectHome = true makes inaccessible and empty (/home, /root, /run/user), so the daemon cannot read or write there.
          Point stateDir outside those trees — the default /var/lib/librefang is managed by StateDirectory=, and any other location outside them gets a systemd-tmpfiles rule plus a ReadWritePaths= entry.
        '';
      }
      {
        assertion = !(cfg.extraEnvironment ? LIBREFANG_HOME);
        message = ''
          services.librefang.extraEnvironment sets LIBREFANG_HOME, which would override the value this module derives from services.librefang.stateDir and desynchronise it from StateDirectory / ReadWritePaths.
          Set services.librefang.stateDir instead.
        '';
      }
    ];

    warnings =
      lib.optional (cfg.openFirewall && listenIsLoopback) ''
        services.librefang.openFirewall opens port ${toString cfg.port} but the daemon binds ${effectiveListen}, so nothing off-host can reach it.
        Set extraEnvironment.LIBREFANG_LISTEN to a routable bind (together with an API key in environmentFile) or turn openFirewall off.
      ''
      ++ lib.optional (cfg.user != defaultUser) ''
        services.librefang.user is set to ${cfg.user}, so this module does not declare the account — define users.users.${cfg.user} yourself with home = "${cfg.stateDir}".
        First start auto-runs `librefang init` (crates/librefang-cli/src/commands/daemon.rs:296-318), and that path exits 1 when `dirs::home_dir()` resolves to nothing (crates/librefang-cli/src/commands/init.rs:13-19).
      ''
      ++ lib.optional (cfg.group != defaultUser) ''
        services.librefang.group is set to ${cfg.group}, so this module does not declare the group — define users.groups.${cfg.group} yourself.
      ''
      ++ lib.optional (listenIsLoopback && cfg.extraEnvironment ? LIBREFANG_ALLOW_NO_AUTH) ''
        services.librefang.extraEnvironment sets LIBREFANG_ALLOW_NO_AUTH while the daemon binds the loopback address ${effectiveListen}, where it has no effect.
        The opt-out only relaxes the non-loopback bind check (crates/librefang-api/src/server.rs:312-330); dropping it keeps a later bind change failing loudly instead of silently running open.
      '';

    users.users = lib.mkIf (cfg.user == defaultUser) {
      ${defaultUser} = {
        isSystemUser = true;
        group = cfg.group;
        # `librefang init` — auto-run on first start (crates/librefang-cli/src/commands/daemon.rs:296-318) — resolves `dirs::home_dir()` and exits 1 when it is `None` (crates/librefang-cli/src/commands/init.rs:13-19), so a system user left on the NixOS default `/var/empty` would abort the first boot even though `LIBREFANG_HOME` already points somewhere writable.
        home = cfg.stateDir;
        createHome = false;
        description = "LibreFang agent operating system daemon";
      };
    };

    users.groups = lib.mkIf (cfg.group == defaultUser) {
      ${defaultUser} = { };
    };

    networking.firewall = lib.mkIf cfg.openFirewall {
      allowedTCPPorts = [ cfg.port ];
    };

    # A relocated state dir cannot use `StateDirectory=`, so create it here.
    systemd.tmpfiles.rules = lib.optional (!stateDirUnderVarLib)
      "d '${cfg.stateDir}' 0700 ${cfg.user} ${cfg.group} - -";

    systemd.services.librefang = {
      description = "LibreFang Agent OS Daemon";
      documentation = [ "https://librefang.ai" ];
      wantedBy = [ "multi-user.target" ];
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];

      # `init_git_if_missing` spawns `git` by bare name on every boot (crates/librefang-kernel/src/kernel/workspace_setup.rs:203-207, called from crates/librefang-kernel/src/kernel/boot.rs:847) to version-control the state dir.
      # A systemd unit does not inherit the system profile's PATH, so git has to be declared here.
      path = [ pkgs.git ];

      environment = {
        # `librefang_home()` prefers `LIBREFANG_HOME` over the user's home directory (crates/librefang-kernel/src/config.rs:535-542); without it the daemon would resolve `<home>/.librefang`.
        LIBREFANG_HOME = cfg.stateDir;
        # `dirs::home_dir()` reads `$HOME` before consulting the passwd entry, and the first-start init path exits 1 when it resolves to nothing (crates/librefang-cli/src/commands/init.rs:13-19).
        HOME = cfg.stateDir;
        LIBREFANG_LISTEN = listenAddress;
      } // cfg.extraEnvironment;

      serviceConfig = {
        # `Type=exec` rather than `simple`: systemd then reports startup failure when the binary cannot be executed at all.
        # The daemon never calls `sd_notify` (no reference anywhere in `crates/`), so `notify` is not an option, and `forking` has no PIDFile to read.
        Type = "exec";

        # `librefang start` with no flags FORKS.
        # `cmd_start` takes the `!spawned && !foreground` branch at crates/librefang-cli/src/commands/daemon.rs:347 into `spawn_detached_daemon` (daemon.rs:56-134), which re-execs the binary with `start --spawned`, calls `libc::setsid()` in `pre_exec` (daemon.rs:110-118), then returns from the parent after a health poll (daemon.rs:354-418) — systemd would see the main process exit immediately and tear the setsid'd child down with it.
        # `--foreground` (crates/librefang-cli/src/cli.rs:79-81) skips that branch and falls through to `rt.block_on(run_daemon(...))` (daemon.rs:465-527), blocking for the process lifetime.
        # Same invocation as the two reference units already in the repo: deploy/librefang.service:11 and crates/librefang-cli/src/commands/maintenance.rs:107-108.
        # The internal `--spawned` flag also stays in the foreground but is `hide = true` and documented as a parent-to-child contract (crates/librefang-cli/src/cli.rs:82-84), so it is not a unit-level interface.
        ExecStart = "${lib.getExe' cfg.package "librefang"} start --foreground";

        User = cfg.user;
        Group = cfg.group;
        WorkingDirectory = cfg.stateDir;

        # Provider API keys and other secrets.
        # systemd reads the file as root before dropping privileges, so it never has to be store-readable.
        # No `EnvironmentFile=-${cfg.stateDir}/secrets.env` counterpart is needed: the foreground path loads `<home>/secrets.env` into its own process environment before building the tokio runtime (crates/librefang-cli/src/commands/daemon.rs:421-437, #4701), so a dashboard-saved key already survives a restart.
        EnvironmentFile = lib.optional (cfg.environmentFile != null) cfg.environmentFile;

        # `run_daemon` installs a graceful-shutdown future that listens for SIGTERM and SIGINT (crates/librefang-api/src/server.rs:2365 and :3081-3106), so systemd's default `KillSignal=SIGTERM` is already the right stop signal.
        Restart = "on-failure";
        RestartSec = 5;
        TimeoutStopSec = 30;

        # Security hardening.
        # Everything down to `RestrictRealtime` mirrors deploy/librefang.service:22-32, so the NixOS unit and the hand-written reference unit stay comparable; `RestrictAddressFamilies=` below has no counterpart in that file and carries its own justification.
        NoNewPrivileges = true;
        ProtectSystem = "strict";
        # Safe only because the state dir lives outside the trees this makes inaccessible, which the `stateDirUnderProtectedHome` assertion above enforces.
        # It does foreclose the BYO-CLI credential discovery that resolves `~/.claude`, `~/.codex`, `~/.gemini` and `~/.qwen` (crates/librefang-api/src/routes/providers.rs:91,122,144,150) — those providers need their keys supplied through `environmentFile` instead.
        ProtectHome = true;
        PrivateTmp = true;
        ProtectKernelTunables = true;
        ProtectKernelModules = true;
        ProtectControlGroups = true;
        RestrictSUIDSGID = true;
        RestrictRealtime = true;
        # Left off deliberately, as in deploy/librefang.service:31 — the WASM plugin sandbox needs writable-executable pages.
        MemoryDenyWriteExecute = false;
        # The API server binds TCP (crates/librefang-api/src/server.rs:1915 onwards) and the ACP bridge binds a unix socket at `<home>/acp.sock` (crates/librefang-api/src/server.rs:2055, crates/librefang-api/src/acp_uds.rs:119-141), so all three families are load-bearing.
        RestrictAddressFamilies = [ "AF_INET" "AF_INET6" "AF_UNIX" ];

        # Resource limits carried over from deploy/librefang.service:35-36.
        LimitNOFILE = 65536;
        LimitNPROC = 4096;
      }
      // lib.optionalAttrs stateDirUnderVarLib {
        StateDirectory = stateDirectoryName;
        StateDirectoryMode = "0700";
      }
      // lib.optionalAttrs (!stateDirUnderVarLib) {
        # `ProtectSystem=strict` mounts the whole filesystem read-only except for the paths declared here, so a relocated state dir has to be listed explicitly.
        ReadWritePaths = [ cfg.stateDir ];
      };
    };
  };
}
