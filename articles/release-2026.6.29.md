---
title: "LibreFang 2026.6.29 Released"
published: true
description: "LibreFang v2026.6.29 release notes — open-source Agent OS built in Rust"
tags: rust, ai, opensource, release
canonical_url: https://github.com/librefang/librefang/releases/tag/v2026.6.29
cover_image: https://raw.githubusercontent.com/librefang/librefang/main/public/assets/logo.png
---

# LibreFang 2026.6.29 Released

This release marks **three major engineering priorities**: bringing LibreFang to global audiences, expanding hardware support, and squashing critical reliability edge cases. **14 PRs from 4 contributors** landed since v2026.6.26-beta.24.

## What's New

### 🌍 Full Korean Language Support

We've shipped complete Korean translations across the entire stack:
- **UI**: 233 language keys translated
- **CLI/TUI**: Commands and help text in Korean
- **Error messages**: Expanded from 43 to 233 localized keys

Korean-speaking developers and teams can now use LibreFang entirely in their language.

### 🏗️ Multiplatform: ARM64 Linux & Android Ready

**aarch64 Linux is now fully supported:**
- Official ARM64 binaries published to AUR and the project-maintained pacman repository alongside x86_64
- Android NDK cross-compilation fixed (legacy binutils symlink + PATH resolution)

Deploy confidently on Raspberry Pi, Apple Silicon Linux containers, cloud ARM instances, and Android environments.

### 🛡️ Reliability Fixes & Security

- **Mixed-media message batches** — edge case fixed where coalesced batches with mixed content types weren't enriched on the debounced path
- **Telegram setup form resilience** — stays available even if the describe call times out, preventing users from getting stuck mid-setup
- **Security patch** — bumped `pdf-extract` to 0.12 to patch [RUSTSEC-2026-0187](https://rustsec.org/) (lopdf vulnerability)

### ⚡ Developer Experience

**Codex CLI** now works outside Git repositories — remove the requirement to run inside a `.git` tree. Perfect for standalone scripts, CI pipelines, and one-off analysis.

## Changes at a Glance

| Category | Changes |
|----------|---------|
| **Internationalization** | Korean UI (#6349), Korean errors (#6353), Korean CLI/TUI (#6356) |
| **Multiplatform** | aarch64 packages (#6358), NDK binutils fix (#6335), NDK PATH fix (#6338) |
| **Reliability** | Telegram setup (#6345), mixed-media enrichment (#6351), security patch (#6339) |
| **Developer Tools** | Codex CLI outside Git (#6347) |
| **Operations** | AUR automation (#6341), pacman repo to R2 (#6352), CI improvements (#6340, #6346) |

<details>
<summary>Detailed changelog</summary>

### Added
- UI Korean translation (#6349) (@seungjin)
- Complete Korean error translations (43 → 233 keys) (#6353) (@houko)
- Add Korean (ko) translation for the CLI/TUI (#6356) (@houko)
- Publish aarch64 packages alongside x86_64 (#6334) (#6358) (@houko)

### Fixed
- Bump pdf-extract 0.10→0.12 to patch lopdf RUSTSEC-2026-0187 (#6339) (@houko)
- Keep Telegram setup form available after describe timeout (#6345) (@pavver)
- Allow Codex CLI outside Git repositories (#6347) (@pavver)
- Enrich coalesced mixed-media batches on the debounced path (#6348) (#6351) (@houko)

### Maintenance
- Symlink legacy NDK binutils so vendored OpenSSL cross-compiles for Android (#6335) (@houko)
- Put NDK bin on PATH so openssl-src finds the legacy ranlib symlink (#6338) (@houko)
- Enable auto-merge instead of forcing --admin (#6340) (@houko)
- Publish AUR packages on release (#6334) (#6341) (@houko)
- Publish project-maintained pacman repo to R2 (#6334) (#6352) (@houko)
- Fix[flake.nix]: Add perl to nativeBuildInputs (#6346) (@FrantaNautilus)

</details>

## Install / Upgrade

```bash
# Binary
curl -fsSL https://get.librefang.ai | sh

# Rust SDK
cargo add librefang

# JavaScript SDK
npm install @librefang/sdk

# Python SDK
pip install librefang-sdk
```

## Links

- [Full Changelog](https://github.com/librefang/librefang/blob/main/CHANGELOG.md)
- [GitHub Release](https://github.com/librefang/librefang/releases/tag/v2026.6.29)
- [GitHub](https://github.com/librefang/librefang)
- [Discord](https://discord.gg/DzTYqAZZmc)
- [Contributing Guide](https://github.com/librefang/librefang/blob/main/docs/CONTRIBUTING.md)
