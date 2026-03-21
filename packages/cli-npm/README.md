# @librefang/cli

LibreFang Agent OS — command-line interface.

## Install

```bash
npm install -g @librefang/cli
```

Or with other package managers:

```bash
# pip
pip install librefang

# Homebrew (macOS)
brew install librefang/tap/librefang

# Cargo
cargo install librefang
```

Or download pre-built binaries from [GitHub Releases](https://github.com/librefang/librefang/releases).

## Usage

```bash
# Initialize LibreFang
librefang init

# Start the daemon
librefang start

# Check system health
librefang doctor
```

## Supported Platforms

| Platform | Architecture | Package |
|----------|-------------|---------|
| macOS | Apple Silicon | `@librefang/cli-darwin-arm64` |
| macOS | Intel | `@librefang/cli-darwin-x64` |
| Linux | x64 (glibc) | `@librefang/cli-linux-x64` |
| Linux | arm64 (glibc) | `@librefang/cli-linux-arm64` |
| Linux | x64 (musl) | `@librefang/cli-linux-x64-musl` |
| Linux | arm64 (musl) | `@librefang/cli-linux-arm64-musl` |
| Windows | x64 | `@librefang/cli-win32-x64` |
| Windows | arm64 | `@librefang/cli-win32-arm64` |

The correct platform-specific binary is automatically installed via `optionalDependencies`.

## Documentation

- [Website](https://librefang.ai)
- [GitHub](https://github.com/librefang/librefang)
- [Documentation](https://librefang.ai/docs)

## License

MIT
