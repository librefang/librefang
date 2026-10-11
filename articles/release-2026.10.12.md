---
title: "LibreFang 2026.10.12 Released"
published: true
description: "LibreFang v2026.10.12 release notes — open-source Agent OS built in Rust"
tags: rust, ai, opensource, release
canonical_url: https://github.com/librefang/librefang/releases/tag/v2026.10.12
cover_image: https://raw.githubusercontent.com/librefang/librefang/main/public/assets/logo.png
---

# LibreFang 2026.10.12 Released

We're excited to announce **LibreFang v2026.10.12**, packed with quality-of-life improvements, power-user features, and critical fixes.

_129 PRs from 6 contributors since v2026.9.19._

## The Big Picture

This release focuses on **making agents more expressive, conversations richer, and operations more transparent**. You can now give agents real avatars instead of initials, see live workflow progress in your terminal, export the exact messages you need from conversations, and configure your most finicky models with all the sampling knobs at once. Behind the scenes, we've hardened identity tracking, fixed RBAC to actually work in the dashboard, and made config reloading predictable. If you've been working around rough edges, this update smooths many of them out.

## 👤 Agents Get Personalities (Avatars & Identity)

Agents and users can now have **emoji glyphs and actual pictures** instead of just initials on a circle.
- Upload an agent's avatar image via `POST /api/agents/{id}/avatar`; the daemon stores it and serves it authenticated, so your agent's face lives safely behind your API.
- Users get the same treatment: `POST /api/users/{name}/avatar` for the image and `PATCH /api/users/{name}/identity` for the emoji.
- Avatars now render **everywhere** — agent list, chat picker, message bubbles, and the sessions sidebar — with graceful fallback to emoji, then initials.
- The dashboard agent editor and TUI now let you upload and manage avatars; a persona name in an agent's identity file is no longer read-only.
- **Bug fix:** Editing agent personality now actually changes what the agent says. The dashboard was writing to the wrong file, so renamings and vibe changes were stored but ignored by the prompt engine.

## 📚 Knowledge Bases: One Upload, Many Agents

Stop copy-pasting documents. Upload files once, grant agents access, and every holder sees them at `@name/whatever.md` in their workspace.
- A knowledge base is a named directory plus an access list — simple, durable, and built on the `[workspaces]` mechanism agents have supported since day one.
- Share a base with multiple agents and revoke access anytime; TOOLS.md rewrites instantly.
- Delete a base the same way you delete any directory — no index to disagree with what's on disk.
- The API and dashboard both handle creation and sharing; you're no longer hand-editing manifests to use this.

## 💬 Richer Conversations

Talking to agents is more fun now.
- **Mermaid diagrams render inline** instead of showing source code. Agents draw architecture sketches and flow charts; now you see them instead of reading ASCII. Tall diagrams cap to a thumbnail; click to expand full-size. The renderer is bundled lazily and sandboxed against hostile input.
- **Conversation tabs** let you keep several chats with the same agent open at once — no more hunting through a dropdown menu to find the one you meant.
- **Export what matters.** Tick the messages you want in a file, include or skip reasoning traces, and print a clean PDF without sidebar and composer noise. No more exporting the whole transcript just to share one exchange.
- Rich text in Telegram now includes `||spoiler||` and `==highlight==` again — both fell off when rich blocks landed, now they're back with correct Telegram semantics.

## 🎛️ Model Tuning: All Knobs in One Place

Power users can now reach every sampling parameter from the same interface.
- Dashboard agent editor and TUI both expose `top_k`, `min_p`, `repeat_penalty` alongside `top_p` and `frequency_penalty`.
- Local-model deployments no longer have to sneak tuning parameters into opaque `extra_params` tables.
- Each provider gets only the knobs it reads — Anthropic gets `top_k`, Ollama gets all three, OpenAI-format drivers send to the right models.
- Sampling overrides now work on every dispatch path: persistent sessions, `/btw` ephemeral turns, streaming, and worker spawns — not just persistent.
- **API fix:** Sampling parameters were being silently dropped on ephemeral and streaming paths while working fine in persistent sessions. No longer.

## 🔄 Full Agent Editor Parity

The dashboard's agent configurator now covers the whole manifest, not a hand-picked 11 fields.
- Model parameters, routing, channels, tools, MCP servers — everything you can hand-edit in TOML is now reachable through the UI.
- The same form the create flow uses powers the existing-agent editor, so behavior is consistent.
- **Skill requirements are now visible.** `GET /api/skills` reports what each skill needs (`required_tools`, `required_capabilities`), so you can't accidentally assign a skill to an agent missing a tool before the skill runs and fails.

## 📊 Observability Wins

Live progress visibility across the board.
- **Workflow runs show step progress** in the TUI: `▰▰▱▱▱ 2/5` updates every two seconds. Run history keeps its selection across refreshes, paused runs show `paused` (not `?`), and newest-first ordering finally landed.
- Agent-type history is now visible from the dashboard with a diff view before restore; you can see every field that diverged from the registry before overwriting it.
- **Goal runs no longer disappear mid-loop.** A race in the state lock made live runs look absent; fixed so `GET /api/goals/{id}/run` always returns the truth.

## 🔐 Auth & Access Control Actually Work

The RBAC system now applies everywhere, not just some paths.
- **Dashboard users are finally authenticated.** REST turns from the Web UI now carry the user's identity instead of just a client IP, so tool policy applies correctly. That silent approval queue for every non-read tool? Gone.
- Channel approvals are routed to the right sidecar by name instead of a missing account ID; adapters no longer lose approval notifications.
- Tool-call batches no longer burn your login rate limit — approvals are metered separately from auth failures.
- Session attribution reaches memory ACLs, so a restricted `memory_access` policy now blocks restricted users in the Web UI instead of silently failing open.

## 🛠️ Operations & Configuration

Several pain points for operators are gone.
- **Config reload is now predictable.** A partial config file no longer resets fields it doesn't mention. Top-level keys you omit stay live; keys you state replace their values wholesale. A restart requirement shows up explicitly instead of silently leaving the new value unused.
- **Provider discovery preference survives a restart.** The toggle now persists in `data/provider_discovery.json` instead of vanishing from `providers/{name}.toml` on boot.
- GitHub Enterprise support: `librefang skill publish` and skill promotions now respect `skills.promotion.api_base_url` and `skills.promotion.release_org`, so you can point both flows at your own GitHub.
- macOS app updates keep your FDA and Automation grants — binaries are now Developer ID signed instead of ad-hoc, so every release isn't treated as a new app.

## 🚀 TUI Gains Terminal-Native Power

Terminal users get the full dashboard experience.
- Shared-folders editor (`w` in agent detail) — add, edit, and delete workspaces without opening another window.
- Goals now expose loop engineering, tick interval, verifier agent, and evaluator model right in the create wizard.
- Templates (agent types) can be restored and version-history browsed with `v` and `Shift+R`.
- Settings exposes vault key management (`7` in the Settings tab) and GitHub token configuration with zero-copy buffers so your secrets never linger in memory.
- Every network failure now says what went wrong instead of showing an empty list — all 30 list fetches across the TUI report daemon errors clearly.

## 📋 Manifest & Template Improvements

Creating and editing agents got easier.
- **Model picker is searchable** — the free-text model ID box is gone. Catalog dropdown with provider+model search replaces it.
- **Agent types editor landed the features create had** — you now get validation, unknown-key warnings, parity with the manifest form, and a single atomic `POST /api/templates/{name}/toml` write instead of two requests that could partially fail.
- **TUI workflow editor** replaces the raw JSON text field with per-field controls. Add/delete steps with `a`/`d`, cycle agent source with Tab, set session mode per step. Can't save with empty prompts — the editor tells you which step is incomplete instead of letting the API reject it later.
- Task dialog exposes `priority` and `timeout_secs`; they stay task-level so one board default applies while others override it.

## 🔒 Security Hardening

- Updated russh to 0.62.5, closing four SSH transport advisories.
- Dashboard and REST auth now validates sender identity through RBAC, not just reads.
- Filename validation is now centralized and strict — no Windows device names, no path traversal attempts, no invisible characters masquerading as legitimate names.
- Mermaid pinned to exact version with lodash security overrides scoped to the dependencies that need them.
- Agent workspace assignment now refuses to hand an agent another agent's directory — spawn no longer creates identity collisions.

## 🐛 Notable Fixes Under the Hood

- Agent messages with no LLM provider configured now show as errors, not info.
- Command routing in channel conversations uses the same agent resolution as the chat itself, so `/model <name>` actually works.
- The inline login no longer loops — session cookie is now scoped correctly.
- Version binaries report the commit they were built from, not a stale SHA from build-script cache.
- Auto-replies run with proper sender context and respect `/think` preference; they're no longer re-dispatched through the message queue.
- Goal runs keep their paused state even if a memory read fails, and live runs aren't taken over by the boot recovery sweep.
- Editing agent identity no longer requires opening the manifest in another editor — rename and vibe changes land correctly.
- ClawHub installs have their own 300s budget and staging directory, so a slow hub doesn't look like a broken route.
- A restored agent type now snapshots what came before, so you have a history entry to go back to.
- Task assignment rejects unknown agents upfront with a 400, instead of creating a row that nothing can claim.

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
- [GitHub Release](https://github.com/librefang/librefang/releases/tag/v2026.10.12)
- [GitHub](https://github.com/librefang/librefang)
- [Discord](https://discord.gg/DzTYqAZZmc)
- [Contributing Guide](https://github.com/librefang/librefang/blob/main/docs/CONTRIBUTING.md)
