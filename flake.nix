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
        src = craneLib.cleanCargoSource ./.;

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

        apps.default = flake-utils.lib.mkApp {
          drv = librefang-cli;
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
