# LibreFang Mobile (iOS + Android)

LibreFang mobile is a **thin client** — the mobile app is a dashboard window that connects to a
remote `librefang` daemon running on your home server, VPS, NAS, or desktop.
The daemon is not embedded in the mobile binary.

## Architecture

Phone → HTTP/WS → librefang daemon (home server / VPS / NAS / desktop)

This is intentional: LibreFang runs cron, autodream, channel adapters, and triggers 24×7.
An iOS/Android app cannot guarantee uptime due to OS background limits.

## Prerequisites

### Android
- Android NDK 26+ (`$ANDROID_NDK_HOME` set)
- Android SDK with API 26+ target
- Java 17
- `cargo tauri android init` (run once in `crates/librefang-desktop/`)

### iOS (macOS only)
- Xcode 15+
- iOS Simulator runtime
- `cargo tauri ios init` (run once in `crates/librefang-desktop/`)

## Local dev commands

```bash
# From crates/librefang-desktop/

# Android emulator (after android init)
cargo tauri android dev

# iOS Simulator (after ios init, macOS only)
cargo tauri ios dev
```

## Generating the mobile scaffolds

The `gen/android/` (Gradle project) and `gen/apple/` (Xcode project) directories are generated
by the Tauri CLI and must be committed after generation:

```bash
cd crates/librefang-desktop
cargo tauri android init
cargo tauri ios init   # macOS only
```

Commit the resulting `gen/android/` and `gen/apple/` directories.

## Minimum OS versions

| Platform | Minimum |
|----------|---------|
| iOS | 14.0 |
| Android | API 26 (Android 8.0) |

## Desktop-only features

The following features are compiled out on iOS/Android via `cfg(not(any(target_os = "ios", target_os = "android")))`:
- System tray icon
- Single-instance enforcement
- Autostart on login
- Global keyboard shortcuts
- Auto-updater
- CLI process spawning (shell plugin)

## Related issues

- Epic: #3351
- Scaffold (this): #3342
- Mobile UX: #3343
- Connection wizard + QR: #3344
- CI build jobs: #3345
- Distribution: #3348
