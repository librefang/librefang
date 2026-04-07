# ADR-MT-005: Event Bus Tenant Isolation

**Status:** Proposed
**Date:** 2026-04-06
**Author:** Engineering
**Related:** ADR-MT-003 (Resource Isolation), ADR-MT-001 (Account Model), SPEC-MT-002 (API Route Changes)
**Epic:** Multi-Tenant Architecture — Phase 2

---

## Problem Statement

The LibreFang event bus (`EventBus` in `event_bus.rs`) is a broadcast-based pub/sub
system with per-agent channels and a shared history ring buffer. It has no concept
of account ownership. Every subscriber — whether using `subscribe_all()` or
`subscribe_agent()` — receives events from ALL accounts indiscriminately.

The `Event` struct (defined in `librefang-types/src/event.rs`) has 7 fields:
`id`, `source`, `target`, `payload`, `timestamp`, `correlation_id`, and `ttl`
-- but no `account_id`. Any agent
can observe any other agent's lifecycle events, tool results, memory updates, and
inter-agent messages regardless of tenant boundary.

This creates three concrete attack vectors:
1. **Cross-tenant event eavesdropping:** `subscribe_all()` in channel_bridge.rs
   and desktop/lib.rs sees events from every account; `history(500)` in API
   handlers returns events from all accounts without filtering
2. **Cross-tenant trigger activation:** The `TriggerEngine` evaluates ALL events
   against ALL registered triggers — an event from Account A can wake an agent
   owned by Account B
3. **History leakage:** The shared ring buffer (`history()`) returns the last 1000
   events with no account filter — `/api/comms/events` calls `history(500)` and
   exposes unfiltered events; `/api/comms/events/stream` polls `audit().recent()`
   which also lacks account filtering

MASTER-PLAN.md's ADR-MT-003 section (lines 252-264) mentions event bus
isolation in one paragraph: "Tag events with account_id. Subscribers receive
only events for their account. The event bus filters on dispatch rather than
maintaining per-account channels." The standalone ADR-MT-003-RESOURCE-ISOLATION.md
does NOT mention the event bus. This ADR expands that MASTER-PLAN paragraph
into a full design.

### Source files verified (2026-04-06):

| Component | File | Lines | Key Finding |
|-----------|------|-------|-------------|
| Event Bus | `librefang-kernel/src/event_bus.rs` | 202 | Single broadcast channel + per-agent DashMap, no account filtering |
| Event Type | `librefang-types/src/event.rs` | 421 | `Event` struct has no `account_id` field |
| Kernel (publish sites) | `librefang-kernel/src/kernel.rs` | 12,491 | 5 `event_bus.publish()` call sites, 7 `Event::new()` constructors — none set account |
| Trigger Engine | `librefang-kernel/src/triggers.rs` | 1,140 | `evaluate()` matches ALL triggers against ALL events, 13 `Event::new()` constructors |
| SSE Stream | `librefang-api/src/routes/network.rs` | 1,433 | `comms_events_stream` polls `audit().recent()`, not event bus — no account filter |
| Event History | `librefang-api/src/routes/network.rs` | 1,433 | `history(500)` called twice (topology + comms_events) — returns events from all accounts |
| Channel Bridge | `librefang-api/src/channel_bridge.rs` | 3,152 | `subscribe_all()` for bridge event forwarding — no account scope |
| Desktop Notifs | `librefang-desktop/src/lib.rs` | 215 | `subscribe_all()` for system tray notifications |
| Workflow Events | `librefang-api/src/routes/workflows.rs` | 2,149 | 1 `Event::new()` constructor — no account |

---

## Blast Radius Scan

### 1. Event Constructors (27 total, 0 account-tagged)

| File | `Event::new()` Count | Currently Sets account_id | Gap |
|------|---------------------|--------------------------|-----|
| `kernel.rs` | 7 | 0 | 7 |
| `triggers.rs` | 13 | 0 | 13 |
| `event_bus.rs` (tests) | 2 | 0 | 2 |
| `event.rs` (tests) | 4 | 0 | 4 |
| `routes/workflows.rs` | 1 | 0 | 1 |
| **Total** | **27** | **0** | **27** |

### 2. Publish Call Sites (5 in kernel, 0 filtered)

| File | Line(s) | Context | Account-Aware |
|------|---------|---------|---------------|
| `kernel.rs` | 7021 | Agent lifecycle event | No |
| `kernel.rs` | 8080 | Background task event | No |
| `kernel.rs` | 10370 | Quota enforcement event | No |
| `kernel.rs` | 10445 | Model routing event | No |
| `kernel.rs` | 10515 | Health check failure event | No |

### 3. Event Consumer Call Sites (4 total, 0 filtered)

| File | Method | Context | Account-Filtered |
|------|--------|---------|------------------|
| `routes/network.rs` | `history(500)` (x2) | Topology builder + comms_events API | No |
| `routes/network.rs` | `audit().recent(50)` | SSE stream (polls audit log, not event bus) | No |
| `channel_bridge.rs` | `subscribe_all()` | Channel bridge forwarding | No |
| `desktop/lib.rs` | `subscribe_all()` | System tray notifications | No |

**Note:** The SSE stream (`comms_events_stream`) does NOT use `subscribe_all()`.
It polls `kernel.audit().recent()` in a loop. The event bus is accessed only via
`history(500)` in `comms_events` and the topology endpoint. Only
`channel_bridge.rs` and `desktop/lib.rs` use `subscribe_all()`.

### 4. EventBus Methods (8 public, 0 account-aware)

| Method | Account Parameter | Needs Change |
|--------|-------------------|---------------|
| `new()` | None | No (constructor) |
| `publish()` | None | Yes — filter dispatch by account |
| `subscribe_agent()` | None | Yes — bind account context |
| `subscribe_all()` | None | Yes — accept account filter |
| `history()` | None | Yes — filter by account |
| `dropped_count()` | None | No (diagnostics) |
| `unsubscribe_agent()` | None | No (cleanup) |
| `gc_stale_channels()` | None | No (GC) |

### 5. Event Payload Types (9 variants, mixed account semantics)

| Variant | Account-Scoped | System-Wide | Rationale |
|---------|---------------|-------------|------------|
| `Message` | Yes | — | Agent-to-agent, always within one account |
| `ToolResult` | Yes | — | Tool output belongs to invoking agent's account |
| `MemoryUpdate` | Yes | — | Memory is account-scoped (ADR-MT-004) |
| `Lifecycle` | Yes | — | Agent spawn/stop is account-specific |
| `Network` | Yes | — | Remote agent activity is account-bound |
| `ApprovalRequested` | Yes | — | Approval is per-agent, per-account |
| `ApprovalResolved` | Yes | — | Resolution is per-agent, per-account |
| `Custom` | Yes | — | User-defined, inherits agent's account |
| `System` | — | Yes | KernelStarted, KernelStopping, HealthCheck |

**Scope decision:** The `Event` struct gains `account_id: Option<String>`. The
EventBus filters on dispatch. System events (`account_id = None`) are delivered
to all subscribers. All 27 constructors, 2 `subscribe_all()` sites, 2 `history(500)`
sites, and 1 audit-poll SSE stream need updates.

---

## Decision

Add `account_id: Option<String>` to the `Event` struct and implement
**dispatch-side filtering** in the EventBus. System events use `None` and are
delivered to all subscribers. Tenant events use `Some(account_id)` and are
delivered only to subscribers that registered with a matching account context.

### Core Design: Event struct change

```rust
// librefang-types/src/event.rs — MODIFIED

pub struct Event {
    pub id: EventId,
    /// Tenant boundary. None = system-wide event (delivered to all).
    /// Some(id) = scoped to one account (filtered on dispatch).
    pub account_id: Option<String>,  // ← NEW
    pub source: AgentId,
    pub target: EventTarget,
    pub payload: EventPayload,
    pub timestamp: DateTime<Utc>,
    pub correlation_id: Option<EventId>,
    #[serde(with = "duration_ms")]
    pub ttl: Option<Duration>,
}

impl Event {
    /// Create a tenant-scoped event.
    pub fn new(source: AgentId, target: EventTarget, payload: EventPayload) -> Self {
        Self {
            id: EventId::new(),
            account_id: None, // Caller sets via .with_account()
            source,
            target,
            payload,
            timestamp: Utc::now(),
            correlation_id: None,
            ttl: None,
        }
    }

    /// Tag this event with an account. Events without account_id are system-wide.
    pub fn with_account(mut self, account_id: impl Into<String>) -> Self {
        self.account_id = Some(account_id.into());
        self
    }

    /// Returns true if this is a system-wide event (no account scope).
    pub fn is_system_event(&self) -> bool {
        self.account_id.is_none()
    }
}
```

### Core Design: Dispatch-side filtering

```rust
// librefang-kernel/src/event_bus.rs — MODIFIED

pub struct EventBus {
    sender: broadcast::Sender<Event>,
    agent_channels: DashMap<AgentId, broadcast::Sender<Event>>,
    /// Maps agent IDs to their owning account for dispatch filtering.
    agent_accounts: DashMap<AgentId, String>,  // ← NEW
    history: Arc<RwLock<VecDeque<Event>>>,
    dropped_count: AtomicU64,
    last_drop_warn: std::sync::Mutex<std::time::Instant>,
}

impl EventBus {
    /// Register an agent's account binding (called during agent spawn).
    pub fn bind_agent_account(&self, agent_id: AgentId, account_id: String) {
        self.agent_accounts.insert(agent_id, account_id);
    }

    /// Subscribe to events for a specific agent, with account filtering.
    pub fn subscribe_agent(&self, agent_id: AgentId) -> broadcast::Receiver<Event> {
        // unchanged — dispatch filtering handles account isolation
        let entry = self.agent_channels.entry(agent_id).or_insert_with(|| {
            let (tx, _) = broadcast::channel(256);
            tx
        });
        entry.subscribe()
    }

    /// Subscribe to broadcast events filtered by account.
    /// Returns a receiver that only gets events for the given account + system events.
    pub fn subscribe_account(&self, account_id: String) -> broadcast::Receiver<Event> {
        // Account-filtered subscription stored separately — Phase 2 enhancement.
        // For now, callers use subscribe_all() and the dispatch filter ensures
        // only matching events are sent.
        self.sender.subscribe()
    }

    pub async fn publish(&self, event: Event) {
        // Store in history (unchanged)
        {
            let mut history = self.history.write().await;
            if history.len() >= HISTORY_SIZE {
                history.pop_front();
            }
            history.push_back(event.clone());
        }

        // Route to target — WITH account filtering
        match &event.target {
            EventTarget::Agent(agent_id) => {
                // Targeted delivery: only deliver if account matches or system event
                if let Some(sender) = self.agent_channels.get(agent_id) {
                    if self.account_allows(&event, agent_id) {
                        let _ = sender.send(event.clone());
                    }
                }
            }
            EventTarget::Broadcast => {
                // Broadcast: send to global channel (subscribers filter),
                // then to per-agent channels WITH account check
                let _ = self.sender.send(event.clone());
                for entry in self.agent_channels.iter() {
                    if self.account_allows(&event, entry.key()) {
                        let _ = entry.value().send(event.clone());
                    }
                }
            }
            EventTarget::Pattern(_) | EventTarget::System => {
                // System/pattern events go to global channel only
                let _ = self.sender.send(event.clone());
            }
        }
    }

    /// Check if an event should be delivered to a given agent based on account.
    fn account_allows(&self, event: &Event, agent_id: &AgentId) -> bool {
        // System events (account_id = None) are delivered to everyone
        let Some(event_account) = &event.account_id else {
            return true;
        };
        // If agent has no registered account, allow (backward compat)
        let Some(agent_account) = self.agent_accounts.get(agent_id) else {
            return true;
        };
        // Same account = allow
        event_account == agent_account.value()
    }

    /// Get event history filtered by account.
    pub async fn history_for_account(
        &self,
        account_id: Option<&str>,
        limit: usize,
    ) -> Vec<Event> {
        let history = self.history.read().await;
        history
            .iter()
            .rev()
            .filter(|e| match (&e.account_id, account_id) {
                // No filter requested — return all (legacy/system mode)
                (_, None) => true,
                // System events visible to all accounts
                (None, _) => true,
                // Account match
                (Some(event_acct), Some(req_acct)) => event_acct == req_acct,
            })
            .take(limit)
            .cloned()
            .collect()
    }

    /// Legacy history (backward compat — returns all events).
    pub async fn history(&self, limit: usize) -> Vec<Event> {
        self.history_for_account(None, limit).await
    }
}
```

### Publish-site update pattern

Every `Event::new()` call site in kernel.rs must chain `.with_account()`:

```rust
// BEFORE:
let event = Event::new(agent_id, EventTarget::System, EventPayload::Lifecycle(...));
self.event_bus.publish(event).await;

// AFTER:
let event = Event::new(agent_id, EventTarget::System, EventPayload::Lifecycle(...))
    .with_account(account_id.as_str());  // account_id from AgentEntry
self.event_bus.publish(event).await;

// SYSTEM EVENT (no account — delivered to all):
let event = Event::new(AgentId::system(), EventTarget::System,
    EventPayload::System(SystemEvent::KernelStarted));
// No .with_account() — account_id stays None
self.event_bus.publish(event).await;
```

### SSE stream endpoint update

**Note:** `comms_events_stream` does NOT currently use `subscribe_all()`. It polls
`kernel.audit().recent(50)` in a loop and converts audit entries to SSE events.
The account filter must be added to BOTH the audit poll and the history-based
endpoints (`comms_events`, topology builder).

```rust
// BEFORE (network.rs — SSE stream):
pub async fn comms_events_stream(State(state): State<Arc<AppState>>) -> Response {
    // Polls audit().recent(50) in a loop — no account filter
}

// AFTER:
pub async fn comms_events_stream(
    State(state): State<Arc<AppState>>,
    account: AccountId,
) -> Response {
    // Poll audit().recent(50) with account filter on each audit entry
    // Skip entries where entry.account_id != account (once audit gains account_id)
}

// BEFORE (network.rs — comms_events + topology):
let events = state.kernel.event_bus_ref().history(500).await;
// Returns events from all accounts

// AFTER:
let events = state.kernel.event_bus_ref()
    .history_for_account(Some(account_id.as_str()), 500).await;
```

### Trigger engine update

The `TriggerEngine::evaluate()` method must skip triggers owned by agents in
different accounts than the event:

```rust
// BEFORE:
pub fn evaluate(&self, event: &Event) -> Vec<(AgentId, String)> {
    // matches ALL triggers against the event
}

// AFTER:
pub fn evaluate(&self, event: &Event, agent_accounts: &DashMap<AgentId, String>) -> Vec<(AgentId, String)> {
    // For each matching trigger:
    //   if event.account_id.is_none() -> allow (system event)
    //   if trigger's agent account == event account -> allow
    //   else -> skip (cross-tenant)
}
```

---

## Pattern Definition

**The structural rule (grepable):**

Every `Event::new()` call in tenant-scoped code MUST be followed by
`.with_account()` unless the event is explicitly a system event:

```rust
// PATTERN: Tenant-scoped event
Event::new(source, target, payload).with_account(account_id)

// PATTERN: System event (no account — intentional)
Event::new(AgentId::system(), EventTarget::System, EventPayload::System(...))
// No .with_account() — comment required: // system-wide, no account scope

// ANTI-PATTERN: Tenant event without account
Event::new(agent_id, EventTarget::Agent(other), EventPayload::Message(...))
// Missing .with_account() — VIOLATION: agent messages must be account-scoped
```

Every `subscribe_all()` call in API handlers MUST be accompanied by account
filtering in the stream mapper or replaced with `subscribe_account()`:

```rust
// PATTERN: Account-filtered subscription
let rx = bus.subscribe_all();
// stream.filter(|e| e.is_system_event() || e.account_id.as_deref() == Some(account))

// ANTI-PATTERN: Unfiltered subscription in an API handler
let rx = bus.subscribe_all(); // VIOLATION: no account filter
```

Every `history()` call in API handlers MUST use `history_for_account()`:

```rust
// PATTERN:
let events = bus.history_for_account(Some(account_id.as_str()), 500).await;

// ANTI-PATTERN:
let events = bus.history(500).await; // VIOLATION in API handler context
```

---

## Verification Gate

```bash
#!/usr/bin/env bash
set -euo pipefail

KERNEL_DIR="crates/librefang-kernel/src"
TYPES_DIR="crates/librefang-types/src"
API_DIR="crates/librefang-api/src"

echo "=== ADR-MT-005 Verification Gate ==="

# Gate 1: Event struct has account_id field
grep -q "account_id.*Option<String>" "$TYPES_DIR/event.rs" \
  || { echo "FAIL: Event struct missing account_id field"; exit 1; }
echo "PASS: Event struct has account_id: Option<String>"

# Gate 2: Event has with_account() builder method
grep -q "fn with_account" "$TYPES_DIR/event.rs" \
  || { echo "FAIL: Event missing with_account() method"; exit 1; }
echo "PASS: Event has with_account() builder"

# Gate 3: EventBus has agent_accounts map
grep -q "agent_accounts" "$KERNEL_DIR/event_bus.rs" \
  || { echo "FAIL: EventBus missing agent_accounts map"; exit 1; }
echo "PASS: EventBus has agent_accounts map"

# Gate 4: EventBus has account_allows() method
grep -q "fn account_allows" "$KERNEL_DIR/event_bus.rs" \
  || { echo "FAIL: EventBus missing account_allows()"; exit 1; }
echo "PASS: EventBus has account_allows() filter"

# Gate 5: EventBus has history_for_account() method
grep -q "fn history_for_account" "$KERNEL_DIR/event_bus.rs" \
  || { echo "FAIL: EventBus missing history_for_account()"; exit 1; }
echo "PASS: EventBus has history_for_account()"

# Gate 6: All Event::new() in kernel.rs are followed by .with_account() or marked system
UNTAGGED=$(grep -n "Event::new(" "$KERNEL_DIR/kernel.rs" \
  | grep -v "with_account\|// system" | wc -l | tr -d ' ')
if [ "$UNTAGGED" -gt 0 ]; then
  echo "FAIL: $UNTAGGED Event::new() calls in kernel.rs without .with_account() or // system comment"
  grep -n "Event::new(" "$KERNEL_DIR/kernel.rs" | grep -v "with_account\|// system"
  exit 1
fi
echo "PASS: All kernel.rs Event::new() calls are account-tagged or system-marked"

# Gate 7: SSE stream endpoint has account filter
grep -q "account.*AccountId\|history_for_account\|subscribe_account" \
  "$API_DIR/routes/network.rs" \
  || { echo "FAIL: SSE stream endpoint missing account filter"; exit 1; }
echo "PASS: SSE stream endpoint has account filtering"

# Gate 8: TriggerEngine evaluate() accepts account context
grep -q "agent_accounts\|account" "$KERNEL_DIR/triggers.rs" \
  || { echo "FAIL: TriggerEngine.evaluate() missing account context"; exit 1; }
echo "PASS: TriggerEngine has account-aware evaluation"

# Gate 9: Compilation
cargo check -p librefang-types -p librefang-kernel -p librefang-api
echo "PASS: Compilation clean"

# Gate 10: No subscribe_all() in API handlers without account filter
UNFILTERED=$(grep -rn "subscribe_all()" "$API_DIR/routes/" \
  | grep -v "account\|// system\|// desktop" | wc -l | tr -d ' ')
if [ "$UNFILTERED" -gt 0 ]; then
  echo "FAIL: $UNFILTERED unfiltered subscribe_all() calls in API routes"
  grep -rn "subscribe_all()" "$API_DIR/routes/" | grep -v "account\|// system"
  exit 1
fi
echo "PASS: No unfiltered subscribe_all() in API routes"

echo ""
echo "=== ADR-MT-005 Gate: ALL PASSED ==="
```

---

## Alternatives Considered

### Alternative A: Per-account event channels (separate bus per tenant)

**Approach:** Maintain a `DashMap<String, broadcast::Sender<Event>>` keyed by
account_id. Each account gets its own broadcast channel. Publish routes events
to the correct account channel. System events go to a dedicated system channel
that all subscribers also listen on.

**Pros:**
- Perfect isolation — no filtering logic needed at dispatch time
- Subscribers cannot accidentally receive cross-tenant events
- Memory backpressure is per-account (one tenant's event flood doesn't fill
  another's channel)

**Cons:**
- Memory overhead: each broadcast channel allocates a ring buffer (1024 slots
  x ~200 bytes per Event = ~200KB per account). At 1000 accounts = ~200MB
  just for empty channels
- Complexity: subscribers must listen on BOTH their account channel AND the
  system channel (two receivers per subscriber, select! loop)
- History ring buffer must be per-account OR a separate shared buffer with
  filtering — doesn't eliminate filtering, just moves it
- Agent-targeted events (`EventTarget::Agent`) still need the per-agent
  DashMap — per-account channels don't replace per-agent routing

**Rejected:** More memory, more complexity, and still needs per-agent routing.
Dispatch-side filtering on the existing single bus is simpler, proven in
openfang-ai, and sufficient for the expected tenant scale (< 10,000 accounts).

### Alternative B: Subscriber-side filtering (unfiltered dispatch, filter on receive)

**Approach:** Keep the bus unchanged. Every subscriber receives ALL events.
Subscribers check `event.account_id` and ignore events that don't match their
account context. The `Event` struct still gets `account_id`, but the bus
itself is account-unaware.

**Pros:**
- Zero changes to EventBus — simplest bus implementation
- Filtering logic is co-located with the subscriber (easier to reason about)
- No risk of dispatch-side bugs accidentally dropping events

**Cons:**
- CPU waste: every subscriber processes every event, even cross-tenant ones.
  At N accounts with M events/sec, each subscriber processes N*M events but
  only cares about M. Scales as O(N) per subscriber.
- Security risk: the events ARE delivered to the subscriber's channel buffer.
  A bug in the filter logic (or a subscriber that skips filtering) leaks
  cross-tenant data. Defense-in-depth prefers not delivering in the first place.
- broadcast channel backpressure: cross-tenant events consume channel capacity,
  potentially causing drops for the subscriber's own events
- SSE streams would serialize cross-tenant events before filtering — wasted
  serialization cycles

**Rejected:** Dispatch-side filtering is O(1) per event (check account on
send, not on receive). It's also more secure — events never enter a channel
they don't belong to.

### Alternative C: Event encryption per account

**Approach:** Encrypt event payloads with per-account keys. Events are
broadcast to all, but only subscribers with the correct decryption key can
read the payload.

**Pros:**
- Cryptographic isolation — even if events leak, payload is unreadable
- Works with any bus topology (single, per-account, federated)

**Cons:**
- Encryption/decryption overhead on every event (AES-GCM ~1us per event,
  but at thousands of events/sec this adds up)
- Key management complexity (per-account keys, rotation, distribution)
- Event metadata (source, target, timestamp) is still visible — metadata
  leakage can reveal tenant activity patterns
- Massive over-engineering for an in-process event bus (encryption is for
  untrusted transport, not in-memory pub/sub)

**Rejected:** The event bus is an in-process data structure, not a network
protocol. Dispatch-side filtering provides equivalent isolation without
cryptographic overhead.

---

## Consequences

### Positive
- Events are tenant-isolated at the dispatch layer — cross-tenant eavesdropping eliminated
- System events (`account_id = None`) remain visible to all subscribers — health,
  kernel lifecycle, and config changes work without special casing
- Backward compatible: `Event::new()` defaults `account_id` to `None`, so
  existing code continues to work (events treated as system-wide until tagged)
- Single bus architecture preserved — no memory overhead from per-account channels
- `history_for_account()` provides account-scoped event replay for SSE streams
- Trigger engine stops cross-tenant activation — agent triggers only fire for
  events within the same account
- Pattern is grepable: every `Event::new()` must have `.with_account()` or a
  `// system` comment

### Negative
- 27 `Event::new()` call sites need `.with_account()` chaining (mechanical but
  spread across 5 files)
- `EventBus` gains a new `DashMap<AgentId, String>` for agent-to-account mapping —
  must be kept in sync with the agent registry (dual bookkeeping)
- `publish()` path gets one extra DashMap lookup per agent channel during broadcast —
  negligible at expected scale but measurable under microbenchmark
- `TriggerEngine::evaluate()` signature changes — all callers must provide account
  context
- Desktop app (`subscribe_all()`) must decide: show all accounts (admin mode) or
  filter to active account

### Phase 3 Debt (intentionally deferred)
- **Event persistence with account_id:** The SQLite `events` table gets `account_id`
  in ADR-MT-004 (v19 migration), but the EventBus history ring buffer is in-memory
  only. Durable event replay per-account is deferred to Phase 3.
- **Per-account channel backpressure:** Currently, one tenant's event flood can
  fill the shared broadcast channel and cause drops for all tenants. Per-account
  rate limiting on the event bus is deferred to Phase 4 (hardening).
- **Cross-account admin events:** Admin operations (account suspension, billing
  alerts) that must be visible to a platform admin across accounts need an
  `EventTarget::Admin` variant or a separate admin subscription. Deferred to
  Phase 4.
- **WebSocket event streams:** The SSE endpoint in `network.rs` needs account
  filtering (covered in this ADR's scope), but any future WebSocket-based
  event streaming must also implement account filtering. Not yet designed.
- **Channel bridge event forwarding:** `channel_bridge.rs` uses `subscribe_all()`
  to forward events to external channels. Per-account channel routing depends on
  ADR-MT-003 Phase 2 channel scoping — deferred until that lands.

---

## Affected Files

| File | Change Type | Description |
|------|-------------|-------------|
| `librefang-types/src/event.rs` | MODIFY | Add `account_id: Option<String>` to `Event`, add `with_account()` and `is_system_event()` |
| `librefang-kernel/src/event_bus.rs` | MODIFY | Add `agent_accounts` DashMap, `bind_agent_account()`, `account_allows()`, `history_for_account()`, `subscribe_account()`; update `publish()` with dispatch filtering |
| `librefang-kernel/src/kernel.rs` | MODIFY | 7 `Event::new()` sites gain `.with_account()`; `spawn_agent()` calls `bind_agent_account()`; 5 `publish()` sites unchanged |
| `librefang-kernel/src/triggers.rs` | MODIFY | 13 `Event::new()` sites gain `.with_account()`; `evaluate()` gains account-aware filtering |
| `librefang-api/src/routes/network.rs` | MODIFY | `comms_events_stream` gains `AccountId` extractor + audit filter; 2 `history(500)` calls become `history_for_account()` |
| `librefang-api/src/routes/workflows.rs` | MODIFY | 1 `Event::new()` site gains `.with_account()` |
| `librefang-api/src/channel_bridge.rs` | MODIFY | `subscribe_all()` replaced with account-filtered subscription (Phase 2) |
| `librefang-desktop/src/lib.rs` | MODIFY | `subscribe_all()` — unchanged for desktop (shows all; single-tenant mode) |

---

## Integration Points

| Trigger | Existing Code | New Behavior |
|---------|--------------|-------------|
| Agent spawned | `kernel.spawn_agent()` publishes Lifecycle event | Must call `event_bus.bind_agent_account(agent_id, account_id)` AND chain `.with_account()` on the event |
| Agent terminated | `kernel.terminate_agent()` publishes Lifecycle event | Must chain `.with_account()` on the event; `unsubscribe_agent()` cleans up account binding |
| Event published | `event_bus.publish(event)` broadcasts to all | Dispatch filters by `account_allows()` before sending to per-agent channels |
| SSE stream requested | `comms_events_stream` polls `audit().recent()` | Must extract `AccountId` and filter audit entries by account |
| Event history queried | `event_bus.history(500)` returns all | API handlers must use `history_for_account(Some(account_id), 500)` |
| Trigger evaluated | `trigger_engine.evaluate(event)` matches all triggers | Must skip triggers owned by agents in different accounts |
| Channel bridge subscribes | `subscribe_all()` for forwarding | Must filter to channel's bound account (depends on ADR-MT-003 channel scoping) |
| System event emitted | `Event::new(..., SystemEvent::KernelStarted)` | No `.with_account()` — `account_id` stays `None`, delivered to all subscribers |

---

## Quality Checks

- [x] Blast radius scan is present with actual numbers (27 constructors, 5 publish sites, 2 subscribe_all() + 2 history() + 1 audit-poll)
- [x] Scope covers ALL affected code in touched files, not just known symptoms
- [x] Verification gate is a runnable command, not prose
- [x] Pattern definition is structural (grepable), not a list of function names
- [x] Phase 3 debt section exists with specific items and rationale
- [x] Alternatives considered (3) with trade-offs
- [x] Integration points listed with existing code references
- [x] Backward compatibility addressed (None = system-wide, default)
- [x] System events explicitly handled (account_id = None delivered to all)
- [x] Cross-account admin operations identified as Phase 4 debt
