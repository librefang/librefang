```markdown
---
title: "LibreFang 0.6.2 Released"
published: true
description: "LibreFang v0.6.2 release notes — open-source Agent OS built in Rust"
tags: rust, ai, opensource, release
canonical_url: https://github.com/librefang/librefang/releases/tag/v0.6.2-20260319
cover_image: https://raw.githubusercontent.com/librefang/librefang/main/public/assets/logo.png
---

# LibreFang 0.6.2 Released

We're thrilled to release **LibreFang v0.6.2**, a stability-focused update that strengthens the foundation of your Agent OS. This release zeroes in on reliability and developer experience, fixing critical issues that affect both the runtime and build pipeline.

## What's Fixed

**Provider Configuration** — We've resolved an issue where providers could unexpectedly appear in multiple tier groups, causing confusion and misconfiguration. This fix ensures your provider setup stays clean and predictable. [@SenZhangAI]

**Build Reliability** — Our CI pipeline was hit with 17 compilation errors. We've squashed them all, restoring full build stability and ensuring the main branch stays green. This makes contributing and deploying smoother for everyone. [@houko]

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
- [GitHub Release](https://github.com/librefang/librefang/releases/tag/v0.6.2-20260319)
- [GitHub](https://github.com/librefang/librefang)
- [Discord](https://discord.gg/DzTYqAZZmc)
- [Contributing Guide](https://github.com/librefang/librefang/blob/main/docs/CONTRIBUTING.md)
```
