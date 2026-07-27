{
  description = "LibreFang - Open-source Agent Operating System";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, crane, flake-utils, rust-overlay, ... }:
    # Per-system outputs (packages / checks / apps / devShells) go inside `eachDefaultSystem`; the system-agnostic ones (`nixosModules`, `overlays`) are merged onto the result at the bottom of this file.
    # A `nixosModule` nested inside `eachDefaultSystem` would land at `nixosModules.<system>.default`, which is not the schema `nixos-rebuild` or `lib.nixosSystem` read — the module would silently be unusable.
    (flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };

        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" "clippy" ];
        };

        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

        # Common build inputs needed by every workspace crate.
        nativeBuildInputs = with pkgs; [
          pkg-config
          rustToolchain
          perl
        ];

        buildInputs = with pkgs; [
          openssl
          dbus
        ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
          pkgs.apple-sdk
          pkgs.libiconv
        ];

        # `librefang-desktop` pulls in Tauri / wry, which require the GTK
        # webview stack at link time. Split these out so the CLI build (the
        # common case) doesn't pay for the heavy native graphics deps just to
        # produce a server binary — this is what breaks `nix build
        # .#librefang-cli` on stock NixOS today (#2937).
        desktopBuildInputs = pkgs.lib.optionals pkgs.stdenv.isLinux (with pkgs; [
          glib
          gtk3
          libsoup_3
          webkitgtk_4_1
          atkmm
          cairo
          gdk-pixbuf
          pango
          # tray-icon dlopens libayatana-appindicator3.so.1 at runtime, not
          # a link dep. wrapGAppsHook3 + gappsWrapperArgs in the desktop
          # derivation below puts this lib dir on LD_LIBRARY_PATH so the
          # dlopen resolves (#3052, #3192).
          libayatana-appindicator
        ]);

        # Filter source to include Rust files plus non-Rust assets needed at compile time
        src = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions [
            (craneLib.fileset.commonCargoSources ./.)
            ./crates/librefang-types/locales
            # librefang-runtime embeds this at compile time via include_str!
            # (crates/librefang-runtime/src/model_catalog.rs) as the offline
            # fallback catalog; without it the Nix build fails to read the file.
            ./crates/librefang-runtime/openrouter-models.snapshot.json
            ./crates/librefang-api/static
            ./crates/librefang-api/src/login_page.html
            ./crates/librefang-cli/templates
            ./crates/librefang-cli/locales
            ./crates/librefang-desktop/tauri.conf.json
            ./crates/librefang-desktop/capabilities
            ./crates/librefang-desktop/icons
            ./crates/librefang-desktop/gen
            # librefang-channels embeds this tree via include_dir!
            # at compile time (crates/librefang-channels/src/embedded_sdk.rs).
            ./sdk/python/librefang
            ./packages/whatsapp-gateway
            ./deploy/docker-compose.observability.yml
            ./deploy/grafana
            ./deploy/otel-collector
            ./deploy/prometheus
            ./deploy/tempo
          ];
        };

        commonArgs = {
          inherit src nativeBuildInputs buildInputs;
          pname = "librefang";
          strictDeps = true;
        };

        # CLI build scope — do NOT compile the desktop crate's native
        # dependencies just to produce the CLI binary. Scoping the
        # deps-only build to `--package librefang-cli` keeps
        # `nix build .#librefang-cli` green on machines that don't have
        # the GTK / webview stack installed.
        cliArgs = commonArgs // {
          pname = "librefang-cli";
          cargoExtraArgs = "--package librefang-cli";
        };

        cliCargoArtifacts = craneLib.buildDepsOnly cliArgs;

        librefang-cli = craneLib.buildPackage (cliArgs // {
          cargoArtifacts = cliCargoArtifacts;
          doCheck = false; # Tests require network/runtime setup.
          meta = with pkgs.lib; {
            description = "LibreFang — Open-source Agent Operating System (CLI / daemon)";
            homepage = "https://github.com/librefang/librefang";
            license = licenses.mit;
            platforms = platforms.unix;
            mainProgram = "librefang";
          };
        });

        # Desktop build scope — adds the GTK / webview deps on Linux.
        desktopArgs = commonArgs // {
          pname = "librefang-desktop";
          cargoExtraArgs = "--package librefang-desktop";
          buildInputs = buildInputs ++ desktopBuildInputs;
        };

        desktopCargoArtifacts = craneLib.buildDepsOnly desktopArgs;

        # Desktop entry assembled with the standard nixpkgs helper so the
        # output matches XDG conventions (proper escaping, hicolor icon
        # theme layout, no manual heredoc).
        librefangDesktopItem = pkgs.makeDesktopItem {
          name = "librefang-desktop";
          desktopName = "LibreFang";
          comment = "Open-source Agent Operating System";
          exec = "librefang-desktop";
          icon = "librefang-desktop";
          terminal = false;
          type = "Application";
          categories = [ "Development" "Utility" ];
          keywords = [ "AI" "Agent" "LLM" "Automation" ];
          # Match the GTK app id Tauri reports so launchers can pair the
          # window with its menu entry / icon.
          startupWMClass = "librefang-desktop";
        };

        librefang-desktop = craneLib.buildPackage (desktopArgs // {
          cargoArtifacts = desktopCargoArtifacts;
          doCheck = false;
          # `copyDesktopItems` is a no-op on darwin; gating the hook on
          # Linux keeps the macOS build path unchanged.
          nativeBuildInputs = nativeBuildInputs
            ++ pkgs.lib.optionals pkgs.stdenv.isLinux [
              pkgs.copyDesktopItems
              # wrapGAppsHook3 injects LD_LIBRARY_PATH (via gappsWrapperArgs
              # below) and the GTK runtime env (XDG_DATA_DIRS,
              # GIO_MODULE_DIR, GSETTINGS_SCHEMA_DIR, …) the webview needs.
              pkgs.wrapGAppsHook3
            ];
          desktopItems = pkgs.lib.optionals pkgs.stdenv.isLinux [ librefangDesktopItem ];
          # tray-icon → libappindicator-sys dlopens
          # `libayatana-appindicator3.so.1` at runtime with no DT_NEEDED
          # entry. patchelf --add-rpath writes DT_RUNPATH, which ld.so only
          # consults for DT_NEEDED deps — never for dlopen string lookups —
          # so the previous RPATH fix (#3052) never actually worked, the
          # tray icon silently failed to appear on NixOS (#3192). Wrapping
          # with gappsWrapperArgs prepends the appindicator lib dir to
          # LD_LIBRARY_PATH so the dlopen call resolves.
          preFixup = pkgs.lib.optionalString pkgs.stdenv.isLinux ''
            gappsWrapperArgs+=(
              --prefix LD_LIBRARY_PATH : "${pkgs.libayatana-appindicator}/lib"
            )
          '';
          postInstall =
            let
              # `128x128@2x.png` contains an `@`, which is not a legal
              # character inside `${…}` Nix path-expression interpolation,
              # so we bind the icons directory once and concatenate the
              # filenames at the shell layer.
              iconsDir = ./crates/librefang-desktop/icons;
            in
            pkgs.lib.optionalString pkgs.stdenv.isLinux ''
              # Install icons into the hicolor theme at every native size
              # we ship in the repo so DEs can pick the right one without
              # rescaling. Icon name must match the desktop entry's Icon=
              # key.
              install -Dm644 "${iconsDir}/32x32.png" \
                "$out/share/icons/hicolor/32x32/apps/librefang-desktop.png"
              install -Dm644 "${iconsDir}/128x128.png" \
                "$out/share/icons/hicolor/128x128/apps/librefang-desktop.png"
              install -Dm644 "${iconsDir}/128x128@2x.png" \
                "$out/share/icons/hicolor/256x256/apps/librefang-desktop.png"
              install -Dm644 "${iconsDir}/icon.png" \
                "$out/share/icons/hicolor/512x512/apps/librefang-desktop.png"
            '';
          meta = with pkgs.lib; {
            description = "LibreFang — Open-source Agent Operating System (desktop UI)";
            homepage = "https://github.com/librefang/librefang";
            license = licenses.mit;
            platforms = platforms.linux ++ platforms.darwin;
            mainProgram = "librefang-desktop";
          };
        });

        # Full-workspace args for checks (clippy runs across the whole tree
        # including librefang-desktop, so it needs the GTK inputs too).
        workspaceArgs = commonArgs // {
          buildInputs = buildInputs ++ desktopBuildInputs;
        };

        workspaceCargoArtifacts = craneLib.buildDepsOnly workspaceArgs;

        # End-to-end evaluation of `nixosModules.default`: build a throwaway NixOS system that enables `services.librefang`, then assert on the unit the module generated.
        # This is the only way CI can catch a broken module without a NixOS host — `nix flake check` on a plain package set never touches the module at all.
        nixosModuleEval =
          let
            evaluated = nixpkgs.lib.nixosSystem {
              inherit system;
              modules = [
                self.nixosModules.default
                ({ config, ... }: {
                  # Suppresses the bootloader and `fileSystems."/"` assertions a real host config would satisfy, so the eval stays about `services.librefang` and nothing else.
                  boot.isContainer = true;
                  system.stateVersion = config.system.nixos.release;
                  services.librefang = {
                    enable = true;
                    # Deliberately not the 4545 default, so an option that silently fails to reach the unit shows up as a mismatch.
                    port = 4646;
                    environmentFile = "/run/secrets/librefang.env";
                    extraEnvironment.RUST_LOG = "info";
                  };
                })
              ];
            };
            svc = evaluated.config.systemd.services.librefang;
            expectations = [
              {
                name = "librefang.service unit is generated";
                ok = evaluated.config.systemd.units ? "librefang.service";
              }
              {
                name = "librefang.service renders to a derivation";
                ok = pkgs.lib.isDerivation evaluated.config.systemd.units."librefang.service".unit;
              }
              {
                name = "ExecStart passes --foreground so systemd keeps the daemon in the foreground";
                ok = pkgs.lib.hasSuffix "/bin/librefang start --foreground" svc.serviceConfig.ExecStart;
              }
              {
                name = "Type=exec (the daemon never calls sd_notify and writes no PIDFile)";
                ok = svc.serviceConfig.Type == "exec";
              }
              {
                name = "LIBREFANG_HOME points at the state dir";
                ok = svc.environment.LIBREFANG_HOME == "/var/lib/librefang";
              }
              {
                name = "LIBREFANG_LISTEN carries the configured port";
                ok = svc.environment.LIBREFANG_LISTEN == "127.0.0.1:4646";
              }
              {
                name = "extraEnvironment is merged into the unit environment";
                ok = svc.environment.RUST_LOG == "info";
              }
              {
                name = "state dir is managed through StateDirectory";
                ok = svc.serviceConfig.StateDirectory == "librefang";
              }
              {
                name = "secrets arrive through EnvironmentFile, not the store";
                ok = svc.serviceConfig.EnvironmentFile == [ "/run/secrets/librefang.env" ];
              }
              {
                name = "TCP and unix sockets are both permitted";
                ok = svc.serviceConfig.RestrictAddressFamilies == [ "AF_INET" "AF_INET6" "AF_UNIX" ];
              }
              {
                name = "hardening is applied";
                ok = svc.serviceConfig.ProtectSystem == "strict"
                  && svc.serviceConfig.NoNewPrivileges
                  && svc.serviceConfig.PrivateTmp
                  && svc.serviceConfig.ProtectHome;
              }
              {
                name = "git is on the unit PATH";
                ok = pkgs.lib.any (p: (p.pname or p.name or "") == "git") svc.path;
              }
              {
                name = "the librefang system user is declared with the state dir as its home";
                ok = evaluated.config.users.users.librefang.home == "/var/lib/librefang"
                  && evaluated.config.users.users.librefang.isSystemUser;
              }
            ];
            failed = map (e: e.name) (pkgs.lib.filter (e: !e.ok) expectations);
          in
          # The expectations run while this attribute is *evaluated*, so both `nix flake check --no-build` and `nix eval .#checks.<system>.…` fail on a regression.
          # The derivation itself deliberately holds no reference to the rendered unit: that text embeds `${librefang-cli}/bin/librefang`, and depending on it would turn a PR-time eval into the 80-95 minute cold workspace compile documented at .github/workflows/nix-build.yml:89-90.
          assert pkgs.lib.assertMsg (failed == [ ]) ''
            nixosModules.default evaluated, but the generated unit failed these expectations: ${pkgs.lib.concatStringsSep "; " failed}
          '';
          pkgs.runCommand "librefang-nixos-module-eval" { } ''
            printf '%s\n' "librefang nixosModule: ${toString (pkgs.lib.length expectations)} expectations passed" > "$out"
          '';

        # Boots a real NixOS guest with `services.librefang.enable = true` and asserts the daemon actually comes up.
        # This is the one thing `nixosModuleEval` above cannot do: evaluation proves the unit has the right shape, not that the process survives being started by systemd.
        #
        # Cost, and why it is safe to have in `checks`: building this derivation compiles `librefang-cli` and boots a VM, so it is expensive.
        # CI never pays that. The PR lane runs `nix flake check --no-build`, which instantiates every check and builds none, so it verifies this expression still evaluates without compiling anything — measured at 43s for the whole lane.
        # The push-to-main matrix runs `nix build .#librefang-cli` / `.#librefang-desktop` and never touches `checks`, so it is unaffected too.
        # Running it for real is a deliberate act on a Linux host with a working /nix and KVM: `nix build .#checks.x86_64-linux.nixos-vm-test -L`.
        # Whether GitHub's hosted runners can nest KVM for a NixOS guest is deliberately not asserted here, because nothing in this repo has established it — see docs/operations/nixos.md.
        nixosVmTest = pkgs.testers.runNixOSTest {
          name = "librefang-nixos-module";

          nodes.machine = { ... }: {
            imports = [ self.nixosModules.default ];

            services.librefang = {
              enable = true;
              # Set explicitly rather than relying on the `mkDefault` the flake's nixosModules wrapper applies, so this test pins "the package this flake builds boots" independently of how the default is resolved.
              package = librefang-cli;
              # The guest has no outbound network. Without this the first boot tries to sync the agent/hand registry and the unit's startup is at the mercy of a network call — the same reason the macOS CI lane sets it (.github/workflows/ci.yml).
              extraEnvironment.LIBREFANG_REGISTRY_OFFLINE = "1";
            };

            environment.systemPackages = [ pkgs.curl ];
            # The daemon boots a Rust kernel and an axum server; the 1024 MB default leaves it thrashing.
            virtualisation.memorySize = 2048;
          };

          # Deliberately narrow: this asserts the module's contract (the unit starts, stays up, and the daemon binds its port) and not the API surface.
          # `/api/health` is the one route the project treats as a stable operator-facing contract (it is the smoke check in CLAUDE.md's live-verification recipe), so one request against it separates "systemd reports active" from "the server is really serving".
          testScript = ''
            machine.wait_for_unit("librefang.service")
            machine.wait_for_open_port(4545)
            machine.succeed("curl -sf http://127.0.0.1:4545/api/health")

            # StateDirectory= created the state dir, and LIBREFANG_HOME pointed the daemon at it rather than at the service user's home.
            machine.succeed("test -d /var/lib/librefang")
            machine.succeed("systemctl show -p MainPID --value librefang.service | grep -qv '^0$'")
          '';
        };
      in
      {
        checks = {
          inherit librefang-cli;

          librefang-clippy = craneLib.cargoClippy (workspaceArgs // {
            cargoArtifacts = workspaceCargoArtifacts;
            cargoClippyExtraArgs = "--workspace --all-targets -- -D warnings";
          });

          librefang-fmt = craneLib.cargoFmt {
            inherit src;
            pname = "librefang";
          };
        }
        # The desktop derivation — Tauri link step, `wrapGAppsHook3`, `copyDesktopItems`, the hicolor icon installs and the `libayatana-appindicator` `LD_LIBRARY_PATH` fix above — used to be reachable only through `packages`, so a regression in the packaging logic passed `nix flake check` and only the CI matrix leg caught it.
        # Gated on Linux: `checks` is evaluated for every system `eachDefaultSystem` covers, and darwin has no GTK / webview stack (`desktopBuildInputs` is empty there).
        // pkgs.lib.optionalAttrs pkgs.stdenv.isLinux {
          inherit librefang-desktop;
        }
        # `lib.nixosSystem` only evaluates for Linux hosts, and only the two architectures NixOS actually targets.
        // pkgs.lib.optionalAttrs (system == "x86_64-linux" || system == "aarch64-linux") {
          nixos-module-eval = nixosModuleEval;
          nixos-vm-test = nixosVmTest;
        };

        packages = {
          default = librefang-cli;
          inherit librefang-cli librefang-desktop;
        };

        apps.default = (flake-utils.lib.mkApp {
          drv = librefang-cli;
        }) // {
          # Propagate the package's meta so `nix flake check` doesn't warn
          # about the app lacking metadata.
          meta = librefang-cli.meta;
        };

        devShells.default = craneLib.devShell {
          checks = self.checks.${system};

          packages = with pkgs; [
            # Rust tooling (provided by crane devShell via checks)
            cargo-watch
            cargo-edit
            cargo-expand

            # Development tools
            just
            gh
            git
            nodejs
            python3
          ] ++ desktopBuildInputs;

          inputsFrom = [ librefang-cli ];

          shellHook = ''
            echo "LibreFang development environment loaded"
            echo "Rust: $(rustc --version)"
          '';
        };
      }
    ))
    # System-agnostic outputs.
    # These MUST sit outside `eachDefaultSystem`: a NixOS module takes `pkgs` from the consuming host's configuration, so it has no system of its own, and the flake schema expects it at `nixosModules.<name>` rather than `nixosModules.<system>.<name>`.
    // {
      nixosModules.librefang = { lib, pkgs, ... }: {
        imports = [ ./nix/nixos-module.nix ];
        # Point `services.librefang.package` at this flake's own build so importing the module is sufficient — the consumer does not also have to apply `overlays.default`.
        # `mkDefault` keeps an explicit `services.librefang.package = …` in the host config winning, and keeps the throw below lazy: it only fires if the option is actually read on a system this flake does not build for.
        services.librefang.package = lib.mkDefault (
          let
            inherit (pkgs.stdenv.hostPlatform) system;
          in
          self.packages.${system}.librefang-cli or (throw ''
            The LibreFang flake does not build librefang-cli for ${system}.
            Set services.librefang.package to a package you build yourself.
          '')
        );
      };

      # `nixosModules.default` is what `nix flake show` and most `imports = [ librefang.nixosModules.default ]` snippets reach for; the named alias reads better in a host config that imports several flakes' modules.
      nixosModules.default = self.nixosModules.librefang;

      overlays.default = final: prev:
        let
          # Read the target system from `prev`, not `final`: deciding *which* attributes an overlay defines based on `final` makes the nixpkgs fixed point self-referential.
          inherit (prev.stdenv.hostPlatform) system;
        in
        # The derivations come from this flake's pinned nixpkgs / crane / rust-overlay inputs rather than the consumer's nixpkgs.
        # That is the point: the three deps-only artifact sets and the CLI/desktop buildInputs split above are what keep `nix build .#librefang-cli` green on a host without the GTK stack (#2937), and re-instantiating crane against a foreign nixpkgs would fork that wiring.
        nixpkgs.lib.optionalAttrs (self.packages ? ${system}) {
          inherit (self.packages.${system}) librefang-cli librefang-desktop;
        };
    };
}
