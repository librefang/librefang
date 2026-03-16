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

        # Common build inputs needed by native dependencies
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

        # Filter source to only include Rust-relevant files
        src = craneLib.cleanCargoSource ./.;

        commonArgs = {
          inherit src nativeBuildInputs buildInputs;
          pname = "librefang";
          strictDeps = true;
        };

        # Build workspace dependencies first (for caching)
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        # Build the full workspace
        librefang = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
          # The default package is the CLI binary
          cargoExtraArgs = "--package librefang-cli";
          doCheck = false; # Tests require network/runtime setup
        });
      in
      {
        checks = {
          inherit librefang;

          librefang-clippy = craneLib.cargoClippy (commonArgs // {
            inherit cargoArtifacts;
            cargoClippyExtraArgs = "--workspace --all-targets -- -D warnings";
          });

          librefang-fmt = craneLib.cargoFmt {
            inherit src;
            pname = "librefang";
          };
        };

        packages = {
          default = librefang;
          librefang-cli = librefang;
        };

        apps.default = flake-utils.lib.mkApp {
          drv = librefang;
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
          ];

          inputsFrom = [ librefang ];

          shellHook = ''
            echo "LibreFang development environment loaded"
            echo "Rust: $(rustc --version)"
          '';
        };
      }
    );
}
