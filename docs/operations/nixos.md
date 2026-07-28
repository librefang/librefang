# Running LibreFang on NixOS

LibreFang ships a flake with first-class NixOS support: two packages, an overlay, and a `services.librefang` NixOS module that generates a working, hardened systemd unit.
This page covers the four ways to consume it, in ascending order of commitment.

Everything below is produced by `flake.nix` at the repository root; the module itself lives in `nix/nixos-module.nix`.

## Try it without installing anything

```bash
nix run github:librefang/librefang -- --help
nix run github:librefang/librefang -- start --foreground
```

`apps.default` wraps the `librefang-cli` package, whose `meta.mainProgram` is `librefang` (`flake.nix:122`), so `nix run` lands on the daemon CLI.

The desktop UI is a separate package rather than an app:

```bash
nix build github:librefang/librefang#librefang-desktop
./result/bin/librefang-desktop
```

## Install into a profile

```bash
nix profile install github:librefang/librefang#librefang-cli
nix profile install github:librefang/librefang#librefang-desktop
```

`install` is used here, and in the README and docs-site sections, because it works on every Nix release that supports flakes.
Nix 2.20 introduced `nix profile add` as the preferred spelling and kept `install` working as an alias, so a recent Nix accepts either.

`librefang-desktop` installs its own `.desktop` entry and hicolor icons at 32, 128, 256 and 512 pixels (`flake.nix:193-200`), so it shows up in the application launcher without further wiring.

## Overlay the packages into your nixpkgs

`overlays.default` adds `librefang-cli` and `librefang-desktop` to a package set.
Use it when you want `pkgs.librefang-cli` available to the rest of your configuration, for example in `environment.systemPackages` or a devShell.

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    librefang.url = "github:librefang/librefang";
  };

  outputs = { nixpkgs, librefang, ... }: {
    nixosConfigurations.myhost = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        { nixpkgs.overlays = [ librefang.overlays.default ]; }
        ({ pkgs, ... }: { environment.systemPackages = [ pkgs.librefang-cli ]; })
      ];
    };
  };
}
```

The derivations the overlay injects are built from the flake's own pinned `nixpkgs`, `crane` and `rust-overlay` inputs, not from your `nixpkgs`.
That is deliberate: the flake maintains three separate crane deps-only artifact sets and a CLI/desktop `buildInputs` split so a CLI build never drags in the GTK stack (`flake.nix:102-133`), and re-instantiating crane against a foreign `nixpkgs` would fork that wiring.
The practical consequence is that overlaying LibreFang does not deduplicate its Rust toolchain against yours.

## Run the daemon as a system service

`nixosModules.default` (alias: `nixosModules.librefang`) declares `services.librefang`.
Importing it is enough — it injects the flake's own `librefang-cli` as the default `package`, so you do not also need the overlay.

```nix
# flake.nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    librefang.url = "github:librefang/librefang";
  };

  outputs = { nixpkgs, librefang, ... }: {
    nixosConfigurations.myhost = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        librefang.nixosModules.default
        ./configuration.nix
      ];
    };
  };
}
```

```nix
# configuration.nix
{ config, pkgs, ... }:

{
  services.librefang = {
    enable = true;

    # API server and dashboard. The module binds loopback only; see
    # "Exposing the API off-host" below before changing that.
    port = 4545;

    # Provider API keys. Never a path inside the Nix store — see
    # "Provider API keys" below.
    environmentFile = "/run/secrets/librefang.env";

    # Everything the daemon writes lives here.
    stateDir = "/var/lib/librefang";

    extraEnvironment = {
      RUST_LOG = "info";
    };
    # The dashboard is served by the daemon itself on the port above. Reach it
    # at http://127.0.0.1:4545/ over an SSH tunnel; leave openFirewall off and
    # read "Exposing the API off-host" before opening the port for real.
    openFirewall = false;
  };
}
```

`nixos-rebuild switch`, then:

```bash
systemctl status librefang
journalctl -u librefang -f
curl -s http://127.0.0.1:4545/api/health
```

### What the generated unit does

`ExecStart` is `${package}/bin/librefang start --foreground`.
The `--foreground` flag is not cosmetic: `librefang start` with no flags takes the `!spawned && !foreground` branch at `crates/librefang-cli/src/commands/daemon.rs:347`, which re-execs the binary through `spawn_detached_daemon` and calls `libc::setsid()` (`crates/librefang-cli/src/commands/daemon.rs:110-118`) before the parent returns.
A `Type=simple` or `Type=exec` unit invoking the bare `start` subcommand would see its main process exit within seconds and then kill the detached child that is doing the actual work.
With `--foreground` (`crates/librefang-cli/src/cli.rs:79-81`) control falls through to `rt.block_on(run_daemon(...))` (`crates/librefang-cli/src/commands/daemon.rs:465-527`) and the process blocks for its lifetime.

`Type=exec` is used rather than `notify`: nothing in `crates/` calls `sd_notify`, so there is no readiness protocol for systemd to wait on.

The unit exports `LIBREFANG_HOME=${stateDir}`.
That variable is what `librefang_home()` reads, ahead of the user's home directory (`crates/librefang-kernel/src/config.rs:535-542`), and it is mandatory here: a system user left on the NixOS default home of `/var/empty` would otherwise resolve an unwritable `/var/empty/.librefang`.
`HOME` is set to the same path, because `dirs::home_dir()` consults `$HOME` first and the auto-run first-start init (`crates/librefang-cli/src/commands/daemon.rs:296-318`) exits 1 when it resolves to nothing (`crates/librefang-cli/src/commands/init.rs:13-19`).

The port reaches the daemon as `LIBREFANG_LISTEN=127.0.0.1:${port}`, which `Kernel::boot_with_config` applies over `config.api_listen` (`crates/librefang-kernel/src/kernel/boot.rs:47-49`) before `cmd_start` hands the value to `run_daemon` (`crates/librefang-cli/src/commands/daemon.rs:481-482,518-519`).
The module deliberately does not generate `config.toml`, because the daemon writes that file itself: the boot-time MCP migrator rewrites it whenever it actually migrates something (`crates/librefang-runtime/src/mcp_migrate.rs:383`, reached from `crates/librefang-kernel/src/kernel/boot.rs:985`), and config edits made through the API or the dashboard are written back by several handlers through `crate::atomic_write` — `POST /api/config/set` at `crates/librefang-api/src/routes/config/manage.rs:1297`, the budget `PUT` at `crates/librefang-api/src/routes/budget.rs:669`, the `[default_model]` writers at `crates/librefang-api/src/routes/providers.rs:2579,2826,2871`.
A read-only store path would break both.
Everything else in `config.toml` therefore stays under the daemon's and the dashboard's control, exactly as on a non-NixOS host.

`pkgs.git` is placed on the unit's `PATH`, because `init_git_if_missing` spawns `git` by bare name on every boot to version-control the state directory (`crates/librefang-kernel/src/kernel/workspace_setup.rs:203-207`).

Hardening mirrors the hand-written reference unit at `deploy/librefang.service:22-32`: `NoNewPrivileges`, `ProtectSystem=strict`, `ProtectHome`, `PrivateTmp`, `ProtectKernelTunables`, `ProtectKernelModules`, `ProtectControlGroups`, `RestrictSUIDSGID` and `RestrictRealtime`.
`RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX` is an addition the NixOS unit makes on top of that list — the directive appears nowhere in `deploy/librefang.service`.
All three address families are load-bearing: the API server binds TCP, and the ACP bridge binds a unix socket at `<stateDir>/acp.sock` (`crates/librefang-api/src/server.rs:2055`, `crates/librefang-api/src/acp_uds.rs:119-141`).
`MemoryDenyWriteExecute` is left off on purpose, as in `deploy/librefang.service:31`, because the WASM plugin sandbox needs writable-executable pages.

## Provider API keys

Put them in a file that systemd reads as root before dropping privileges, and point `environmentFile` at it:

```
ANTHROPIC_API_KEY=sk-ant-...
OPENAI_API_KEY=sk-...
GROQ_API_KEY=gsk_...
LIBREFANG_VAULT_KEY=<44-character base64 of 32 random bytes>
```

The reason to use `EnvironmentFile` rather than `extraEnvironment` is that a Nix store path is world-readable to every local user, and a unit's inline `Environment=` block is itself a store file.
`environmentFile` is asserted not to be a store path, so writing `environmentFile = ./secrets.env;` (a Nix path literal, which Nix copies into the store) fails evaluation rather than silently publishing your keys.

Generate the file out-of-band. With [sops-nix](https://github.com/Mic92/sops-nix):

```nix
{ config, ... }:

{
  sops.secrets.librefang-env = {
    sopsFile = ./secrets/librefang.yaml;
    key = "env";
    mode = "0400";
  };

  services.librefang.environmentFile = config.sops.secrets.librefang-env.path;
}
```

With [agenix](https://github.com/ryantm/agenix):

```nix
{ config, ... }:

{
  age.secrets.librefang-env.file = ./secrets/librefang.env.age;
  services.librefang.environmentFile = config.age.secrets.librefang-env.path;
}
```

Keys saved from the dashboard land in `<stateDir>/secrets.env` instead, and survive restarts without any unit-level wiring: the foreground start path loads that file into its own process environment before building the tokio runtime (`crates/librefang-cli/src/commands/daemon.rs:421-437`).
`environmentFile` and dashboard-saved keys therefore coexist; the declarative file is read first and the dashboard's copy overrides it.

`LIBREFANG_VAULT_KEY` must base64-decode to exactly 32 bytes — use `openssl rand -base64 32`, which produces 44 characters. Thirty-two ASCII characters are not 32 bytes.

## Exposing the API off-host

The module binds loopback and there is no `host` option, so `openFirewall` on its own opens a port that nothing off-host can reach; the module emits a warning saying exactly that.
To bind a routable address, override the environment variable directly:

```nix
{
  services.librefang = {
    enable = true;
    openFirewall = true;
    extraEnvironment.LIBREFANG_LISTEN = "0.0.0.0:4545";
    environmentFile = "/run/secrets/librefang.env"; # must export LIBREFANG_DASHBOARD_USER and LIBREFANG_DASHBOARD_PASS
  };
}
```

Authentication is not optional in this configuration, and the environment is a narrower lever than it looks.
`api_key` is read from `<stateDir>/config.toml` only — no environment variable feeds it — so the sole authentication source the daemon picks up from the unit environment is a dashboard credential pair, `LIBREFANG_DASHBOARD_USER` plus `LIBREFANG_DASHBOARD_PASS`.
An `environmentFile` holding nothing but provider keys satisfies neither `any_auth_configured` nor the daemon's boot-time refusal.

`run_daemon` refuses to start on a non-loopback bind with no authentication configured and no `LIBREFANG_ALLOW_NO_AUTH` opt-in (`check_bind_auth_safety` and `any_auth_configured` in `crates/librefang-api/src/server.rs`).
The module asserts at evaluation time as well, but it cannot read the contents of a secret produced out-of-band, so `environmentFile != null` is only a proxy for the real condition.
When authentication lives in a `config.toml` this module does not manage, declare that with `authConfiguredExternally = true` instead of pointing `environmentFile` at a file that carries no credentials.
Prefer an SSH tunnel or a reverse proxy with TLS over a raw public bind — an unauthenticated LibreFang API grants shell execution, vault access, and your LLM API keys to anyone who can reach the port.

## Custom user or state directory

The module declares the `librefang` system user and group only while `user` and `group` are left at their defaults.
If you point them elsewhere you must declare the account yourself, with `home` set to the state directory — the first-start init path exits 1 when `dirs::home_dir()` resolves to nothing (`crates/librefang-cli/src/commands/init.rs:13-19`).
The module warns when it detects this.

A `stateDir` under `/var/lib` is managed with `StateDirectory=`, which creates it with the right ownership and mode 0700 before the daemon starts.
Any other location gets a `systemd.tmpfiles` rule plus an explicit `ReadWritePaths=` entry instead, because `ProtectSystem=strict` mounts everything outside the declared write set read-only.

A `stateDir` inside `/home`, `/root` or `/run/user` fails evaluation: the unit sets `ProtectHome=true`, which systemd documents as making exactly those three trees inaccessible and empty for the service, so the daemon could not read or write its own state directory there.
Both the tmpfiles rule and the `ReadWritePaths=` entry would still be generated, which is why the module asserts on this rather than leaving it to be discovered at runtime.

## Known sharp edges

### The dashboard is downloaded at first start, not embedded

A Nix-built `librefang-cli` contains no dashboard assets.
`crates/librefang-api/static/react/` is gitignored (`.gitignore:3`) and `crates/librefang-api/build.rs` creates it as an empty placeholder purely so the `include_dir!` at `crates/librefang-api/src/webchat.rs:82` compiles.
Asset resolution prefers `<stateDir>/dashboard/` and falls back to the embedded copy (`crates/librefang-api/src/webchat.rs:1-8`), so a freshly installed daemon serves a "Downloading dashboard assets…" placeholder page (`crates/librefang-api/src/webchat.rs:34-54`) until the runtime download finishes.
That needs outbound network access on first start.
Only the dashboard asset route is affected; whether every other route behaves identically during the download was not traced for this document.

### First start is not offline-clean

`cmd_start` calls `ensure_initialized` (`crates/librefang-cli/src/commands/daemon.rs:321`), which auto-runs `librefang init` when `<stateDir>/config.toml` is absent, and that path syncs the skill/agent registry over the network (`crates/librefang-cli/src/commands/init.rs:65`).
On an air-gapped host, seed `<stateDir>/config.toml` before enabling the service.

### `ProtectHome=true` blocks bring-your-own-CLI credentials

The hardened unit cannot see `/home`, which forecloses the credential discovery that resolves `~/.claude`, `~/.codex`, `~/.gemini` and `~/.qwen` (`crates/librefang-api/src/routes/providers.rs:91,122,144,150`).
Supply those providers' keys through `environmentFile` instead.
This is the same trade-off `deploy/librefang.service:24` already makes.

### Desktop tray icon: a dlopen the linker cannot see

`tray-icon` reaches `libayatana-appindicator3.so.1` through `dlopen` with no `DT_NEEDED` entry.
The first attempt to fix this used `patchelf --add-rpath`, which writes `DT_RUNPATH` — and `ld.so` consults `DT_RUNPATH` only for `DT_NEEDED` dependencies, never for `dlopen` string lookups.
So the RPATH fix (#3052) never actually worked and the tray icon silently failed to appear on NixOS (#3192).
The working fix wraps the binary with `wrapGAppsHook3` and prepends the appindicator library directory to `LD_LIBRARY_PATH` through `gappsWrapperArgs` (`flake.nix:167-179`, landed as #3197).
If you build the desktop package outside this flake, reproduce the `LD_LIBRARY_PATH` prefix or the tray icon will be missing with no error message.

### Past Nix-path breakages worth knowing about

Regular CI does not exercise the Nix path, so these all reached `main` before being noticed (`.github/workflows/nix-build.yml:3-5`):

- **#2937 / #2974** — `nix build .#librefang-cli` failed on stock NixOS because the deps-only build was not package-scoped and dragged the desktop crate's GTK/webview stack into a server-only build. Fixed by splitting `cliArgs` from `desktopArgs` with separate crane artifact sets (`flake.nix:102-133`).
- **#3052** — the original `patchelf --add-rpath` tray-icon fix, described above. It did not work.
- **#3156** — `librefang-desktop` shipped no `.desktop` entry or icons, so it never appeared in a desktop launcher. Fixed with `makeDesktopItem` plus hicolor icon installs (`flake.nix:138-151,193-200`).
- **#3197** — the `wrapGAppsHook3` wrapping that finally made the tray icon resolve (`flake.nix:161-179`).
- **#6081** — cold builds filled the `ubuntu-latest` runner's ~14 GB of free disk and the nix-daemon was killed mid-build with "Nix daemon disconnected unexpectedly". CI now reclaims ~25 GB of preinstalled toolchains first (`.github/workflows/nix-build.yml:101-109`).

Two source-filter regressions in the same family are worth adding to the list, because they fail only under Nix and not under a plain `cargo build`: #5714 (`sdk/python/librefang`, embedded via `include_dir!`) and #6547 (`crates/librefang-runtime/openrouter-models.snapshot.json`, embedded via `include_str!`).
Any new compile-time-embedded asset must be added to the `fileset` union at `flake.nix:66-94` or the Nix build will fail to read it while every other build path succeeds.

### CI coverage of the Nix path

`nix flake check` now includes the desktop derivation on Linux and a `nixos-module-eval` check that builds a throwaway `lib.nixosSystem` with `services.librefang.enable = true` and asserts on the generated unit.
A `pull_request` job runs evaluation only — no compilation — so a broken flake or module is caught before merge in minutes rather than after merge in an hour and a half.
The expensive `nix build` matrix still runs on push-to-main only; the rationale is recorded at `.github/workflows/nix-build.yml:7-14`.

### Proving the service actually starts

`nixos-module-eval` proves the unit has the right shape; it cannot prove the daemon survives being started by systemd.
That is what the `nixos-vm-test` check is for — it boots a real NixOS guest with `services.librefang.enable = true`, waits for the unit, waits for port 4545, and requests `/api/health`.

Run it on a Linux host with a working `/nix` and KVM:

```bash
nix build .#checks.x86_64-linux.nixos-vm-test -L
```

**CI does not run this check, and the check being green is not something CI can tell you.**
The pull-request lane runs `nix flake check --no-build`, which instantiates every check and builds none, so it verifies the test expression still evaluates — a rename in the module that broke the test would be caught — while compiling nothing.
The push-to-main matrix builds `.#librefang-cli` and `.#librefang-desktop` and never touches `checks` at all.

Building it for real compiles `librefang-cli` and boots a VM, which is why it is opt-in.
Whether GitHub's hosted runners can nest KVM for a NixOS guest is not established anywhere in this repository, so nothing here claims they can; giving this check a CI lane is an open question for the maintainers rather than a settled design.

## Packaging the desktop app outside nixpkgs

Nothing in this flake asserts which shared libraries a given distribution ships, and neither should a downstream package.
Nix sidesteps the question entirely: `desktopBuildInputs` names nixpkgs attributes (`glib`, `gtk3`, `libsoup_3`, `webkitgtk_4_1`, `atkmm`, `cairo`, `gdk-pixbuf`, `pango`, `libayatana-appindicator`) whose existence the evaluator checks for you (`flake.nix:50-64`).
Outside Nix there is no such guarantee, so probe at build or install time rather than hardcoding a distribution's package name.

The soname the binary actually needs — `libayatana-appindicator3.so.1`, and the webkit2gtk series the Tauri webview links — is a stable property of the code.
Which distribution package provides it is not, and it varies between Debian, Fedora, Arch and the downstream derivatives.
`librefang doctor` implements exactly this probe (see `WEBKIT_PKG_MODULES` and `TRAY_PKG_MODULE` in `crates/librefang-cli/src/doctor.rs`) and reports what the host has rather than what a package list claims it should have; reuse those module names instead of inventing your own.
