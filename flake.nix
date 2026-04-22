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
        ]);

               # Filter source to only include Rust-relevant files
        # Include locale/files needed for i18n compile-time embedding
        cleanSrc = craneLib.cleanCargoSource ./.;
        src = pkgs.runCommand "librefang-src" {} ''
          cp -a ${cleanSrc} $out
          chmod -R u+w $out
          # Restore locale dirs stripped by cleanCargoSource
          rm -rf $out/crates/librefang-types/locales
          cp -a ${./.}/crates/librefang-types/locales $out/crates/librefang-types/
          # Restore packages dir stripped by cleanCargoSource
          rm -rf $out/packages
          cp -a ${./.}/packages $out/packages
          # Restore static dir (locales, logo, favicon) stripped by cleanCargoSource
          rm -rf $out/crates/librefang-api/static
          cp -a ${./.}/crates/librefang-api/static $out/crates/librefang-api/
          # Restore login_page.html stripped by cleanCargoSource
          mkdir -p $out/crates/librefang-api/src
          cp -a ${./.}/crates/librefang-api/src/login_page.html $out/crates/librefang-api/src/
          # Restore CLI templates dir
          rm -rf $out/crates/librefang-cli/templates
          cp -a ${./.}/crates/librefang-cli/templates $out/crates/librefang-cli/
          # Restore CLI locales dir
          rm -rf $out/crates/librefang-cli/locales
          cp -a ${./.}/crates/librefang-cli/locales $out/crates/librefang-cli/
        '';

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

        librefang-desktop = craneLib.buildPackage (desktopArgs // {
          cargoArtifacts = desktopCargoArtifacts;
          doCheck = false;
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
