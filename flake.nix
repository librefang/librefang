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
    flake-utils.lib.eachDefaultSystem (system:
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
        ];

        buildInputs = with pkgs; [
          openssl
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
          # tray-icon dlopens at runtime, not a link dep — patchelf below
          # adds it to RPATH so the tray plugin can find it (#3052).
          libayatana-appindicator
        ]);

        # Filter source to include Rust files plus non-Rust assets needed at compile time
        src = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions [
            (craneLib.fileset.commonCargoSources ./.)
            ./crates/librefang-types/locales
            ./crates/librefang-api/static
            ./crates/librefang-api/src/login_page.html
            ./crates/librefang-cli/templates
            ./crates/librefang-cli/locales
            ./crates/librefang-desktop/tauri.conf.json
            ./crates/librefang-desktop/capabilities
            ./crates/librefang-desktop/icons
            ./crates/librefang-desktop/gen
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
            ++ pkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.copyDesktopItems ];
          desktopItems = pkgs.lib.optionals pkgs.stdenv.isLinux [ librefangDesktopItem ];
          postFixup = pkgs.lib.optionalString pkgs.stdenv.isLinux ''
            patchelf --add-rpath "${pkgs.libayatana-appindicator}/lib" "$out/bin/librefang-desktop"
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
    );
}
