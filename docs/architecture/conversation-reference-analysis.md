# Conversation surfaces: reference analysis

Status: phase 0 deliverable for block 4, item 39.
This document is the reference study the backlog asks for before implementing conversation features.
It stops here for operator review: no implementation is included.

Code baseline: `origin/main` at `7fa13649f`.
References read on 2026-11-25 from the vendors' own documentation (URLs at the end).

## Scope

Block 4 of the backlog: conversation sidebar, multi-conversation per agent, channels modelled as sessions with declared capabilities, one command registry with per-surface dispatch, contextual `/help`, conversation export and print, Mermaid rendering, file upload from the agent menu, and audio.
The message grouping and queue mechanism of section 4.1 is included because it changes the same conversation surface.
Full-duplex voice (block 10) stays out.

## Current baseline (verified in code)

| Area | Today | Evidence |
|---|---|---|
| Sessions | A session store with a canonical session per agent, channel-scoped session ids, and API routes to list, switch, export and import sessions. | `crates/librefang-types/src/agent.rs:380`, `crates/librefang-api/src/routes/agents/mod.rs:146-162` |
| Conversation list | The dashboard shows one active conversation per agent; there is no sidebar and no session switcher in the chat page. | `crates/librefang-api/dashboard/src/pages/ChatPage.tsx` |
| Multi-conversation | The API can switch sessions; there is no per-agent flag and no multi-session UI. | `POST /api/agents/{id}/sessions/{session_id}/switch` |
| Channels as sessions | Channel turns are session-scoped, but `ChannelAdapter` has no capability declaration, so behaviour still branches on channel type. | `crates/librefang-channels/src/types.rs:827` |
| Command registry | 38 `CommandDef`s in one registry with `CLI`, `CHANNEL` and `DASHBOARD` scopes. The dashboard palette derives from `GET /api/commands`. | `crates/librefang-channels/src/commands/mod.rs:163`, `crates/librefang-api/dashboard/src/pages/ChatPage.tsx:140` |
| Command dispatch | Behaviour still lives in `bridge::handle_command`; the TUI keeps a manual `match` with its own `/help` text. | `crates/librefang-channels/src/bridge.rs:7626`, `crates/librefang-cli/src/tui/mod.rs:3300` |
| Grouping (window A) | The channel bridge already debounces per `(channel, chat)`, with typing-triggered flush, a hard max window and a buffer cap; it is disabled by default and comes from per-adapter overrides. The dashboard and TUI have no grouping. | `crates/librefang-channels/src/bridge.rs:879`, `:1721`, `crates/librefang-types/src/config/types.rs:172` |
| Queue (window B) | No queue, steer or interrupt. `/queue` is dashboard-only and only reports whether the agent is processing. | `crates/librefang-channels/src/commands/mod.rs:505` |
| Export | Markdown export exists in the dashboard and as an API route; there is no message selection and no print. | `ChatPage.tsx:51`, `POST /api/agents/{id}/sessions/{session_id}/export` |
| Mermaid | Rendered inline; there is no click-to-fullscreen. | `crates/librefang-api/dashboard/src/components/ui/MarkdownContent.tsx:58` |
| Files | Upload exists in the chat; there is no upload entry in the agent parameter menu. | `ChatPage.tsx` |
| Audio | None. | — |
| Effort control | Reasoning-effort selector in chat and agent editor. | #8616, #8617 |

## Reference matrix

Legend: ✅ has it · ◐ partial or different shape · — absent.
"Adopt" is the recommendation for block 4; "how" is the reinterpretation for our agent-and-channel model.

| Capability | LibreChat 0.8.9 | Open WebUI 0.13.5 | ChatGPT (Nov 2026) | Claude (Nov 2026) | Adopt / how |
|---|---|---|---|---|---|
| Conversation organization | Projects and folders | Nested folders, tags, pins, board/list | Projects with files and instructions | Projects with threads and memory | ✅ Sidebar of conversations per agent; folders belong to B8. |
| Several conversations per agent | One chat per agent, switchable | Tabs and `/side` side chats | Branch a chat from any message | Threads inside a project; branch by editing in the classic experience | ✅ Per-agent flag plus session tab strip; the API already lists and switches sessions. |
| Channels as first-class sessions | Agent Channels (Slack beta) with linked accounts | Channels: shared spaces with `@model`, threads, reactions | Connectors, not a conversation surface | Connectors, not a conversation surface | ✅ Keep our channel sessions; add declared adapter capabilities instead of channel-name branches. |
| Declared channel capabilities | Not a public contract | Not a public contract | Not applicable | Not applicable | ✅ New `ChannelCapabilities` on the adapter: `multi_session`, `threads`, `typing`, `audio_in`, `audio_out`, `attachments`, `edit`. |
| One command registry | Slash commands per surface | Slash commands per surface | `/` commands | `/` commands | ✅ Registry stays the only declaration; one dispatcher table consumes it on every surface. |
| Contextual `/help` | Per surface | Per surface | Per surface | Per surface | ✅ Filter by surface, identity and declared channel capabilities. |
| Window A: burst grouping | Queue picks the message up on send; no burst window | Queue; the reply wraps up at its next step | Chat has no queue (open feature request); Work and Codex steer | No queue; steering happens by posting in the thread | ✅ Extend the existing bridge debouncer to web and TUI, typing signal first; per-agent and per-channel window. |
| Window B: queue while running | Queue, steer, interrupt; durable queue with reorder, edit, send now | Server-persisted queue with send now, edit and delete | Work/Codex steer; Chat cannot queue | Steer a thread by posting into it | ✅ Three distinct actions: queue (default), steer (only before visible output or side effects), interrupt; never share a button with `/stop`. |
| Branching / fork | Fork from a message | `/side` side chat | Branch a chat | Branch by editing a message | ◐ Defer: it needs branch-aware storage; not part of block 4's delivery. |
| Export | Markdown, text, JSON, CSV, PNG; active or all branches | Folder export with permission | Account export | File export, artifact export | ✅ Keep Markdown; add message selection and print; evaluate JSON later. |
| Mermaid | Inline and exportable | Inline, click to expand | Inline visuals | Inline interactive visuals | ✅ Render small by default, click for fullscreen. |
| File upload | Composer palette | Composer and channels | Composer and voice | Composer, projects and artifacts | ✅ Existing upload; add the agent parameter menu entry. |
| Audio | Read aloud and dictation | STT, TTS, hands-free and realtime calls | Voice, Live mode | Voice mode | ◐ Defer until block 9 defines modality resolution; do not build a second path now. |
| Effort controls | Reasoning effort in composer | Model parameters | Reasoning controls | Reasoning controls | ✅ Already shipped; keep it in the conversation surface. |
| Multi-model side-by-side | Yes | Yes | No | No | No: our unit is an agent, not a model comparison. |
| Shared team space | Agent Channels beta | Channels | Shared projects | Shared projects | No for now: external channels already are the shared surface; internal team chat is a different product. |

## What this changes in our plan

The analysis supports the plan in section 4.1 but reorders one dependency: grouping and queueing are one per-session inbox, not two features.
The bridge already owns a debouncer for external channels; the missing half is the durable queue and the web/TUI surface.
Building the queue first and wiring the debouncer into it avoids two independent message buffers.

The command story is also cheaper than it looks: the registry is already the declaration and the dashboard already consumes it.
What remains is one dispatcher table with a `CommandContext` and the removal of the TUI's private `match`, not a new registry.

## Decisions required before implementation

1. **Default while the agent is working.** Queue the message, or steer the running turn.
   Recommendation: queue by default; steer is an explicit control.
2. **Mid-flight cancellation.** May a steer abort the running turn when nothing has been shown to the user and no tool with effects has run.
   Recommendation: yes, with a hard guard that refuses once either happened.
3. **Grouping defaults.** Web and TUI rely on the typing signal; messaging channels use a time window (the current 500 ms example).
   Recommendation: 0 ms on web/TUI, 500 ms on messaging, both overridable per agent and channel.
4. **Which adapters declare multi-session first.** Candidate order: Telegram topics, Discord threads, Slack threads.
   Confirm the order before the capability struct is frozen.
5. **Message branching.** Defer to the persistence block (B13), or include it in block 4.
   Recommendation: defer.
6. **Audio.** Wait for block 9's modality resolver, or ship STT/TTS now.
   Recommendation: wait; one audio path, not two.

## Sources

- LibreChat: message composer and queue/steer, `https://www.librechat.ai/docs/features/composer`; import/export, `https://www.librechat.ai/docs/features/import_convos`; v0.8.9 release notes, `https://www.librechat.ai/changelog/v0.8.9`.
- Open WebUI: feature index, `https://docs.openwebui.com/features/`; channels, `https://docs.openwebui.com/features/channels/`; folders and projects, `https://docs.openwebui.com/features/chat-conversations/chat-features/conversation-organization/`; v0.13.4 and v0.13.5 release notes.
- ChatGPT: projects, `https://help.openai.com/en/articles/10169521-projects-in-chatgpt`; release notes, `https://help.openai.com/en/articles/6825453-chatgpt-release-notes`; voice, `https://help.openai.com/en/articles/20001274-chatgpt-voice`; queue request for Chat mode, `https://community.openai.com/t/feature-request-add-message-queue-to-chat-mode/1392884`.
- Claude: release notes, `https://support.claude.com/en/articles/12138966-release-notes`; projects redesigned, `https://claude.com/resources/articles/projects-redesigned`; artifacts, `https://claude.com/features/artifacts`.
