# ADR-012: iOS Companion App

**Status**: Proposed
**Date**: 2026-03-19
**Authors**: Daniel Alberttis

## Version History

| Version | Date | Author | Changes |
|---------|------|--------|---------|
| 0.1 | 2026-03-19 | Daniel Alberttis | Initial draft — openclaw iOS reference analysis, architecture decision, scope definition |
| 0.2 | 2026-03-19 | Daniel Alberttis | Tauri mobile re-evaluation: clarified embedded-kernel constraint (not possible on iOS), added two-track decision (Tauri for Phase 1 chat MVP, SwiftUI for sensor bridge) |
| 0.3 | 2026-03-19 | Daniel Alberttis | DeepWiki source verification against upstream: (1) `#[cfg_attr(mobile)]` and `#[cfg(desktop)]` guards in `lib.rs` are fork additions, not upstream; (2) corrected WebSocket protocol — actual message types are `text_delta`/`typing`/`response`/`tool_start`/`tool_end`, not `token`/`done`; (3) sensor bridge requires new protocol work — no `tool_call` → client dispatch path exists today; (4) pairing issues no per-device auth token; (5) full section restructure and consistency pass |
| 0.4 | 2026-03-19 | Daniel Alberttis | Automated ADR accuracy audit (agentic QE agent, 15 claims checked): 14/15 correct; corrected port claim — `KernelConfig.api_listen` code default is `127.0.0.1:50051` not 4200 (4200 is this fork's `openfang.toml` override per CLAUDE.md); CSP example updated to port-agnostic wildcard |

---

## Context

OpenFang runs as a daemon exposing a REST/WebSocket API (code default: `127.0.0.1:50051`; this fork configures `127.0.0.1:4200` via `openfang.toml`). It has no mobile client. OpenClaw — the closest comparable open-source agent OS — ships a native SwiftUI iOS companion app (`apps/ios/`) that pairs with its local gateway daemon over WebSocket, exposes iOS device sensors as agent tools, and provides a thin chat/voice UI. Studying that implementation reveals a well-validated pattern applicable here.

### What the OpenClaw iOS App Does (Reference Analysis)

The OpenClaw iOS app (`github.com/openclaw/openclaw/tree/main/apps/ios`) is a **SwiftUI companion node** written in Swift 6.0, targeting iOS 18.0+, built with XcodeGen. Its architecture:

| Dimension | OpenClaw |
|-----------|----------|
| UI framework | SwiftUI (declarative, modern) |
| Backend protocol | WebSocket to local gateway (`ws://127.0.0.1:18789`) — custom binary/JSON frame protocol |
| Auth | Setup code + TLS certificate pinning on first connect |
| Role | Thin sensor bridge — AI processing stays server-side |
| Device capabilities exposed | Camera, location, contacts, calendar, microphone, motion, photo library |
| Extensions | watchOS companion, Share extension, Live Activities / Dynamic Island, home screen widget |
| Architecture | `GatewayConnectionController` → WebSocket → custom `ConnectParams`/`HelloOk`/`RequestFrame`/`ResponseFrame`/`EventFrame` protocol |
| Shared code | `OpenClawKit` Swift package (protocol types, chat UI components) shared between iOS and Watch targets |

The key insight: the iOS app is **stateless** with respect to AI. It relays user input to the daemon and renders whatever the daemon returns. Sensors are exposed as callable tools the agent runtime invokes over the WebSocket channel. All intelligence — LLM calls, memory, tool routing — lives in the daemon.

### OpenFang's Current State Relevant to iOS

OpenFang already has the building blocks for a companion app:

| Capability | Status | Detail |
|-----------|--------|--------|
| REST API | Live | ~160 routes; listen address from `KernelConfig.api_listen` (code default `127.0.0.1:50051`, fork config `4200`) |
| WebSocket streaming | Live | `GET /api/agents/:id/ws` (`crates/openfang-api/src/ws.rs`) — bidirectional real-time chat |
| Device pairing API | Live, disabled by default | `POST /api/pairing/request`, `POST /api/pairing/complete`, `GET /api/pairing/devices`, `DELETE /api/pairing/devices/:id`, `POST /api/pairing/notify` — `PairingConfig.enabled = false` |
| Push notifications | Live | `/api/pairing/notify` → ntfy or Gotify; no APNs yet |
| SSE log streaming | Live | `GET /api/logs/stream` |
| OpenAI-compat streaming | Live | `POST /v1/chat/completions` with SSE |
| Tauri mobile scaffolding | Partial (our fork only) | `#[cfg_attr(mobile, tauri::mobile_entry_point)]` + `#[cfg(desktop)]` guards already added to `crates/openfang-desktop/src/lib.rs` — not present in upstream |

### The Embedded-Kernel Constraint

`openfang-desktop` (Tauri 2.0) boots `OpenFangKernel` in-process, binds to `127.0.0.1:0`, and points its WKWebView at that localhost server. This is why the desktop Tauri app is so elegant — kernel, API server, and UI are one binary.

**This model does not work on iOS.** iOS forbids apps from binding TCP ports and serving inbound connections. Any iOS app, regardless of framework, must be a *remote client* connecting to a daemon running elsewhere (on a Mac, a home server, etc.).

Consequence for Tauri mobile: a Tauri iOS target would be a WKWebView pointing at `http://<mac-ip>:<port>` (whatever `api_listen` is configured to). That is functionally equivalent to bookmarking the existing WebChat SPA in Mobile Safari, with two additions: a home screen icon and the ability to ship native plugins. **The WebChat SPA already runs on an iPhone today** — open the daemon's listen address in Safari.

---

## Decision

### 1. Two-Track Approach

The embedded-kernel constraint, the maturity gap in Tauri mobile, and the distinct nature of sensor-bridge work lead to a two-track strategy:

| Track | Framework | Goal | When |
|-------|-----------|------|------|
| **Phase 1 — Chat MVP** | Tauri mobile (iOS + Android) | WebChat SPA on device home screen; same UI as desktop, minimal new code | Now |
| **Phase 2 — Sensor Bridge** | Native SwiftUI (`apps/ios/`) | iOS device capabilities as agent tools; Share extension; watchOS; Live Activities | After Phase 1 validates demand |

Phase 1 and Phase 2 are not mutually exclusive. Tauri mobile ships first. The SwiftUI app supplements or replaces it once sensor-bridge use cases are proven.

### 2. Phase 1 — Tauri Mobile Chat MVP

#### What's Already Done (Our Fork, Not Upstream)

`crates/openfang-desktop/src/lib.rs` already has:
- `#[cfg_attr(mobile, tauri::mobile_entry_point)]` — mobile entry point annotated
- `#[cfg(desktop)]` guards on all desktop-only plugins: tray, autostart, global shortcuts, single-instance, updater

These are fork additions. Upstream OpenFang has none of this.

#### Remaining Code Changes

Three changes to `crates/openfang-desktop/src/lib.rs`:

**1. Split the kernel boot by platform:**
```rust
#[cfg(desktop)]
let (url, _server) = {
    let handle = server::start_server().expect("Failed to start OpenFang server");
    let url = format!("http://127.0.0.1:{}", handle.port);
    (url, Some(handle))
};

#[cfg(mobile)]
let (url, _server) = {
    let url = load_daemon_url();  // from tauri-plugin-store
    (url, None::<()>)
};
```

**2. Add `load_daemon_url()`** — reads stored URL from `tauri-plugin-store`; returns a bundled `connect.html` URL if none is saved (first-run setup flow).

**3. Add `connect.html`** as a bundled Tauri asset — a minimal HTML form: daemon URL input → save → navigate to daemon. No framework, plain HTML/JS.

#### Config Change

Update CSP in `crates/openfang-desktop/tauri.conf.json`. Current value is locked to `127.0.0.1:*`. Mobile needs:
```
connect-src 'self' http://127.0.0.1:* ws://127.0.0.1:* http://*:* ws://*:*;
```
Scoped to mobile builds via Tauri's platform-specific config or a `#[cfg(mobile)]` build script.

#### One-Time Setup (Mac, Not Code)

```bash
rustup target add aarch64-apple-ios
cd crates/openfang-desktop
cargo tauri ios init    # generates gen/apple/
cargo tauri ios dev     # boots iOS simulator
```

#### Phase 1 Server-Side Changes

None. All functionality uses existing routes.

#### Phase 1 Scope

| In scope | Out of scope |
|----------|-------------|
| WebChat SPA on device home screen | iOS sensors as agent tools |
| Pairing setup flow (existing routes) | watchOS companion |
| Push notifications via ntfy (existing `PairingConfig`) | Live Activities / Dynamic Island |
| Android target (same Tauri codebase) | APNs native push |

### 3. Phase 2 — Native SwiftUI Sensor Bridge

#### Why SwiftUI and Not Tauri for Phase 2

For the sensor bridge specifically:
- Camera (AVFoundation), CoreLocation, Contacts, EventKit require native Swift APIs — they cannot be accessed from a WKWebView without native Swift plugins regardless of shell
- WKWebView WebSocket connections are suspended when the app backgrounds on iOS; URLSession WebSocket is not
- Share extensions, Live Activities, Dynamic Island, watchOS companion, and App Clips require native targets
- The sensor bridge is real new work (see §4) — at that point a native app costs no more than a Tauri app with native plugins

#### Tech Stack (Phase 2)

| Decision | Choice | Reason |
|----------|--------|--------|
| Language | Swift 6.0 with strict concurrency | Same as OpenClaw reference; catches async bugs at compile time |
| UI | SwiftUI | Declarative, required for Live Activities; avoids UIKit boilerplate |
| Minimum deployment | iOS 17.0+ | `Observable` macro, improved `ScrollView`; >90% device coverage |
| Build | XcodeGen | YAML-driven project generation; avoids `.xcodeproj` Git conflicts |
| Networking | `URLSession` WebSocket + data tasks | No additional library needed; URLSession WebSocket stable on iOS 13+ |
| Auth storage | Keychain (`Security` framework) | Secure, device-only, not iCloud-backed |
| Package management | Swift Package Manager | Standard; no CocoaPods/Carthage |
| Linting | SwiftLint + SwiftFormat | CI enforcement |

#### Repository Structure (Phase 2)

```
openfang-ai/
└── apps/
    └── ios/
        ├── project.yml                  ← XcodeGen project definition
        ├── Signing.xcconfig             ← Code signing (gitignored values)
        ├── OpenFangApp/
        │   ├── Sources/
        │   │   ├── OpenFangApp.swift        ← @main entry point
        │   │   ├── RootView.swift           ← navigation root
        │   │   ├── Connection/
        │   │   │   ├── DaemonConnection.swift       ← URLSession WebSocket manager
        │   │   │   ├── DaemonServiceResolver.swift  ← Bonjour mDNS discovery
        │   │   │   ├── DaemonHealthMonitor.swift    ← ping + reconnect loop
        │   │   │   └── KeychainStore.swift          ← credential persistence
        │   │   ├── Pairing/
        │   │   │   ├── PairingView.swift
        │   │   │   └── PairingController.swift
        │   │   ├── Chat/
        │   │   │   ├── ChatView.swift
        │   │   │   ├── ChatModel.swift
        │   │   │   └── MessageBubble.swift
        │   │   ├── Sensors/
        │   │   │   ├── SensorBridge.swift          ← routes ios_tool_call dispatch
        │   │   │   ├── CameraCapture.swift
        │   │   │   ├── LocationProvider.swift
        │   │   │   ├── ContactsProvider.swift
        │   │   │   ├── CalendarProvider.swift
        │   │   │   └── MicrophoneCapture.swift
        │   │   ├── Agents/
        │   │   │   ├── AgentsView.swift
        │   │   │   └── AgentRow.swift
        │   │   ├── Settings/
        │   │   │   └── SettingsView.swift
        │   │   └── Voice/                   ← Phase 3
        │   └── Resources/
        │       ├── Assets.xcassets
        │       └── Info.plist
        └── Packages/
            └── OpenFangKit/
                ├── Package.swift
                └── Sources/
                    ├── OpenFangProtocol/    ← message types
                    │   ├── WsMessage.swift
                    │   ├── PairingMessage.swift
                    │   └── IosToolCall.swift
                    └── OpenFangChatUI/      ← reusable components
                        ├── MessageList.swift
                        └── InputBar.swift
```

### 4. WebSocket Protocol (Source-Verified)

The actual message types in `crates/openfang-api/src/ws.rs`, verified against the upstream source via DeepWiki:

**Server → Client:**

| Type | Payload | Notes |
|------|---------|-------|
| `connected` | `{ agent_id }` | On upgrade |
| `typing` | `{ state: "start"\|"tool"\|"stop", tool? }` | Activity indicator |
| `text_delta` | `{ content }` | Streaming tokens, debounced |
| `response` | `{ content, usage, cost, iterations }` | Turn complete |
| `tool_start` | `{ tool, input }` | Observability — server ran a tool |
| `tool_end` | `{ tool, result }` | Observability — server tool finished |
| `error` | `{ message }` | |
| `silent_complete` | — | Agent chose not to reply |
| `canvas` | `{ html }` | Structured HTML output |
| `agents_updated` | agent list | Periodic broadcast |
| `pong` | — | Keepalive response |

**Client → Server:**

| Type | Payload | Notes |
|------|---------|-------|
| `message` | `{ content, files? }` | User sends a chat message |
| `command` | `{ content }` | Slash command e.g. `/model gpt-4o` |
| `ping` | — | Keepalive |

**Important:** `tool_start`/`tool_end` are read-only notifications — the server ran a tool and is informing the client. The server **never** sends a tool dispatch to the client for execution today. The sensor bridge (§5) requires adding new message types.

### 5. Sensor Bridge Protocol (Phase 2 New Work)

There is no `tool_call` → client dispatch path in the current codebase. Building sensor tools requires a new bidirectional exchange:

**New message types to add:**

| Type | Direction | Payload |
|------|-----------|---------|
| `ios_tool_call` | Server → Client | `{ id, tool, args }` |
| `ios_tool_result` | Client → Server | `{ id, result }` |
| `ios_tool_error` | Client → Server | `{ id, error }` |

**Server-side flow (`tool_runner.rs`):**

1. Agent invokes `ios_camera_photo`. `tool_runner.rs` checks if the agent has an active WS session with an iOS-paired device.
2. Sends `{ type: "ios_tool_call", id: "<uuid>", tool: "ios_camera_photo", args: {} }` over WS.
3. Parks on a `oneshot::Receiver<IosToolResult>` with a 30s timeout.
4. Client captures the photo, sends `{ type: "ios_tool_result", id: "<uuid>", result: { image_base64: "..." } }`.
5. Server resolves the `oneshot`, returns result to `run_agent_loop`.

**Client-side flow (`SensorBridge.swift`):**

Receives `ios_tool_call` → match on `tool` → dispatch to `CameraCapture`, `LocationProvider`, etc. → send `ios_tool_result` back over the same WS connection.

**New iOS tool inventory:**

| Tool name | iOS capability | Permission key |
|-----------|---------------|----------------|
| `ios_camera_photo` | AVFoundation — still image | `NSCameraUsageDescription` |
| `ios_camera_video` | AVFoundation — short clip | `NSCameraUsageDescription`, `NSMicrophoneUsageDescription` |
| `ios_location` | CoreLocation — coordinates + address | `NSLocationWhenInUseUsageDescription` |
| `ios_contacts_search` | Contacts.framework | `NSContactsUsageDescription` |
| `ios_calendar_list` | EventKit — upcoming events | `NSCalendarsUsageDescription` |
| `ios_microphone_record` | AVAudioSession — audio clip | `NSMicrophoneUsageDescription` |

**Server-side file changes (Phase 2, all additive):**

| File | Change |
|------|--------|
| `crates/openfang-runtime/src/tool_runner.rs` | Add `ios_*` dispatch branch; `oneshot` await with 30s timeout |
| `crates/openfang-api/src/ws.rs` | Handle `ios_tool_result` / `ios_tool_error` inbound messages; route to parked `oneshot` |
| `crates/openfang-api/src/routes.rs` | Add `POST /api/ios/tool-result` — HTTP fallback if WS dropped |
| `crates/openfang-types/src/config.rs` | `PairingConfig`: add `push_provider = "apns"` variant; `apns_key_path`, `apns_key_id`, `apns_team_id` fields |

### 6. Connection and Pairing Flow

**Pairing (verified against source):**

`POST /api/pairing/complete` accepts `display_name`, `platform`, `push_token` and stores a `PairedDevice` record. **It does not issue a per-device auth token.** The device is recorded for push-notification targeting only.

Auth for API calls uses the daemon's global API key from config (`api_key` in `KernelConfig`), passed as `Authorization: Bearer <api_key>` on HTTP requests and the WebSocket upgrade. If `api_key` is empty (default), no auth is required.

**Connection lifecycle:**

```
App launch
  ↓
Load stored { daemon_url } from tauri-plugin-store (Phase 1) or Keychain (Phase 2)
  ↓ First run: no URL stored
Setup screen — user enters daemon URL (e.g. http://192.168.1.100:50051 or :4200 depending on config)
  → GET /api/health                               ← verify reachable
  → POST /api/pairing/request                     ← { token, qr_uri, expires_at }
  → User enters token on desktop / scans QR
  → POST /api/pairing/complete                    ← { display_name, platform, push_token }
  → Save { daemon_url, device_id }
  ↓ Subsequent runs
  → GET /api/health                               ← liveness check
  → GET /api/agents                               ← agent list
  → WebSocket: GET /api/agents/:id/ws             ← streaming chat
     Authorization: Bearer <api_key>  (if configured)
```

**Local network discovery (Phase 2):** when the daemon URL is unknown, `DaemonServiceResolver` uses `NWBrowser` to discover the daemon via Bonjour mDNS. Requires the daemon to advertise `_openfang._tcp.` — see §7.

### 7. Bonjour mDNS Advertisement (Phase 2 Server Addition)

When pairing is enabled, the daemon advertises itself on the local network so the iOS app can discover it without the user knowing its IP address.

**Implementation:** `mdns-sd` crate (pure Rust, no system daemon dependency).

```
Service type: _openfang._tcp.
Service name: "OpenFang on {hostname}"
Port: daemon listen port (from `KernelConfig.api_listen`; code default 50051)
TXT record: { "version": "0.4.8", "api": "v1" }
```

Registered on server start, unregistered on graceful shutdown. Controlled by new `KernelConfig` field:
```toml
[pairing]
enabled = true
mdns_advertise = true   # default true when pairing.enabled = true
```

**File changes:** `crates/openfang-api/src/server.rs` (register service), `crates/openfang-types/src/config.rs` (add `mdns_advertise` to `PairingConfig`), `Cargo.toml` (add `mdns-sd`).

### 8. Push Notifications

The existing `POST /api/pairing/notify` endpoint broadcasts to all paired devices. Supported providers:

| Provider | Phase | Notes |
|----------|-------|-------|
| `none` | — | Default, no notifications |
| `ntfy` | Phase 1 | User installs ntfy iOS app; daemon pushes to `ntfy_url/ntfy_topic`. No Apple Developer account required. |
| `gotify` | Phase 1 | Self-hosted push server |
| `apns` | Phase 2 | Native Apple Push Notification Service. Requires Apple Developer Program (~$99/yr). `push_token` from iOS app passed during pairing. Adds `apns_key_path`, `apns_key_id`, `apns_team_id` to `PairingConfig`. |

### 9. What Does Not Change

No existing crate interface, API contract, or route is modified by either phase.

| Crate / Component | Status |
|-------------------|--------|
| `openfang-kernel` | Unchanged |
| `openfang-runtime` | Phase 1: unchanged. Phase 2: `tool_runner.rs` additive only |
| `openfang-memory` | Unchanged |
| `openfang-channels` | Unchanged |
| `openfang-wire` | Unchanged |
| `openfang-api` routes | Phase 1: unchanged. Phase 2: additive new routes under `/api/ios/` |
| Pairing subsystem | Used as-is; `enabled` set to `true` in config |
| WebChat SPA | Unchanged — served as-is to both the desktop Tauri window and the mobile Tauri WKWebView |

### 10. Server-Side Additions Summary

**Phase 1 — none.**

**Phase 2 additions (all additive):**

| Change | File | What |
|--------|------|------|
| `ios_tool_call` dispatch | `tool_runner.rs` | New `ios_*` tool branch; `oneshot` await |
| `ios_tool_result` inbound | `ws.rs` | Route inbound result to parked `oneshot` |
| HTTP fallback route | `routes.rs` | `POST /api/ios/tool-result` |
| APNs config fields | `config.rs` | `apns_*` fields on `PairingConfig` |
| mDNS advertisement | `server.rs` | `_openfang._tcp` Bonjour service |
| `mdns_advertise` field | `config.rs` | Field on `PairingConfig` |
| `mdns-sd` crate | `Cargo.toml` | Pure-Rust mDNS |

---

## Alternatives Considered

### Alternative A: React Native

Cross-platform. Rejected because:
- Camera, CoreLocation, Contacts, EventKit deep integration requires native modules regardless
- ~20MB runtime bundle overhead
- OpenClaw validates native Swift for the sensor bridge use case
- Swift 6 strict concurrency is a better fit for the async WebSocket model

### Alternative B: Custom Binary Frame Protocol (OpenClaw-style)

OpenClaw uses a custom binary/JSON frame protocol over WebSocket. Rejected because:
- OpenFang already has a working HTTP/WebSocket REST API that is well-tested
- Introducing a second protocol requires parallel server infrastructure with no benefit
- The existing JSON WebSocket protocol (`text_delta`, `typing`, `response`, etc.) is sufficient for Phase 1 chat
- For Phase 2 sensor dispatch, the new `ios_tool_call`/`ios_tool_result` message types extend the existing protocol — no custom framing needed

### Alternative C: Tauri Mobile Only (No SwiftUI Phase 2)

Keep Tauri mobile permanently and implement sensors via native Tauri Swift plugins instead of a standalone SwiftUI app.

This is viable if sensor bridge demand does not materialise. If it does:
- Writing native Swift sensor plugins for Tauri achieves the same result as a SwiftUI `SensorBridge.swift` — the code is identical Swift
- The Tauri plugin system adds boilerplate (Rust bindings, JSON serialisation layer) around native Swift that a standalone SwiftUI app does not need
- Live Activities, Dynamic Island, watchOS companion, and Share extensions cannot be implemented as Tauri plugins — they require native targets
- If the sensor bridge is built, a dedicated SwiftUI app is cleaner than a Tauri app with native plugin scaffolding that covers the same ground

Conclusion: Tauri-only is the right choice if the use case stays at "chat client." SwiftUI becomes the right choice at the moment sensor work begins.

### Alternative D: Tauri Mobile Embedding the Kernel (Not Possible)

Rejected categorically. iOS forbids TCP socket binding in apps. `server::start_server()` fails on iOS. The desktop Tauri model of embedding the kernel is not portable to mobile regardless of toolchain choice.

---

## Consequences

### Positive

- Phase 1 requires minimal new code — the Tauri mobile scaffolding (`#[cfg_attr(mobile)]`, `#[cfg(desktop)]`) is already done in our fork
- The existing WebChat SPA, pairing API, WebSocket handler, and push notification infrastructure are all validated by a real mobile client before any Phase 2 work begins
- Phase 2 (if built) makes iOS device capabilities first-class agent tools — use cases that cannot exist with a desktop-only daemon
- `OpenFangKit` Swift package (Phase 2) creates a reusable foundation for watchOS, visionOS, and macOS AppKit targets
- The two-track approach means Phase 1 can ship immediately without committing to Phase 2

### Negative

- Phase 1 Tauri mobile gives the user a wrapped browser, not a native iOS experience
- Requires Apple Developer Program membership for TestFlight / App Store distribution (~$99/yr)
- Phase 2 sensor bridge requires modifying `tool_runner.rs` and `ws.rs` — contained but non-trivial
- Phase 2 mDNS adds `mdns-sd` as a server dependency
- `apps/ios/` (Phase 2) lives outside the Cargo workspace — separate CI pipeline required (`xcodebuild` or Xcode Cloud)

### Neutral

- Swift/Xcode development requires macOS — no Windows/Linux iOS build path (standard Apple constraint)
- Phase 1 Tauri iOS build requires `rustup target add aarch64-apple-ios` and Xcode on the build machine
- Phase 1 and Phase 2 can coexist — the Tauri mobile app and the native SwiftUI app are not mutually exclusive

---

## References

- [OpenClaw iOS app](https://github.com/openclaw/openclaw/tree/main/apps/ios) — reference implementation (analysed via DeepWiki)
- `ADR-001-openfang-baseline.md` §14 — full API surface including pairing and WebSocket routes
- `crates/openfang-desktop/src/lib.rs` — Tauri app entry point with existing mobile scaffolding
- `crates/openfang-api/src/ws.rs` — WebSocket chat handler and message protocol
- `crates/openfang-api/src/routes.rs:10326` — pairing route handlers
- `crates/openfang-types/src/config.rs:595` — `PairingConfig` struct (fields: `enabled`, `max_devices`, `token_expiry_secs`, `push_provider`, `ntfy_url`, `ntfy_topic`)
- [Tauri 2.0 iOS distribution docs](https://v2.tauri.app/distribute/app-store/)
- [mdns-sd crate](https://github.com/keepsimple1/mdns-sd) — pure-Rust mDNS implementation
- [XcodeGen](https://github.com/yonaskolb/XcodeGen) — YAML-driven Xcode project generation (Phase 2)
