# ADR-015: Browser Automation Layer — ARIA Snapshot Driver, Web Scraper Hand, and LinkedIn Integration

**Status**: Draft
**Phase**: 1
**Date**: 2026-03-21
**Authors**: Daniel Alberttis

## Version History

| Version | Date | Author | Changes |
|---------|------|--------|---------|
| 0.1 | 2026-03-21 | Daniel Alberttis | Initial draft. Four-phase browser automation plan: MCP config (Phase 1, zero-code), openfang-browser crate (Phase 2), Web Scraper Hand (Phase 3), LinkedIn Hand (Phase 4). ARIA snapshot approach documented as the anti-drift mechanism. OpenClaw CDP relay wired as default. globodai-mcp-linkedin-sales-navigator CSS selector problem stated and rejected as the implementation path. |

---

> **PHASE 2 GATE — READ BEFORE IMPLEMENTING**
>
> Before beginning Phase 2 (`crates/openfang-browser`), confirm the following:
>
> **ADR-014 (Vendor Code Ownership):** Any Playwright or browser-adjacent Node.js types that are surfaced in Rust must be owned types under `crates/`, not vendored third-party skeletons. `agent-browser` is an external CLI binary invoked via stdio — it is not vendored. The Rust wrapper types (`BrowserDriver`, `AriaSnapshot`, `AriaRef`) are first-party and live in `crates/openfang-browser/`.
>
> **ADR-011 (RuVix Interface Contract) Constraint 3:** If browser action trajectories are shared between agents for SONA learning (Phase 2, trajectory recording), signals must cross a typed Queue boundary — not a shared `Arc<Mutex<T>>`. Each agent's `BrowserDriver` records its own trajectory; SONA learning signals are submitted via the consolidation hook, not by direct cross-agent memory access.
>
> Reference: `docs/adr/ADR-011-ruvix-interface-contract.md`, `docs/adr/ADR-014-vendor-to-first-party-crates.md`

---

## Context

OpenFang agents currently have no browser control capability. Agents can query APIs, call LLMs, read files, and execute shell commands — but they cannot navigate websites, fill forms, or interact with authenticated web sessions. This is a significant capability gap for any workflow that involves web-native data (LinkedIn, HubSpot, CRMs, data portals, internal tools with no API).

### The Existing Approach and Its Problem: `globodai-mcp-linkedin-sales-navigator`

A LinkedIn MCP server (`globodai-mcp-linkedin-sales-navigator`) exists and exposes 7 MCP tools:

| Tool | Purpose |
|------|---------|
| `search_leads` | Search Sales Navigator with filters |
| `get_lead_profile` | Fetch a LinkedIn profile |
| `save_lead` | Save a lead to a list |
| `list_lead_lists` | List all lead lists |
| `create_lead_list` | Create a new lead list |
| `send_inmail` | Send an InMail message |
| `export_leads` | Export leads to CSV |

The implementation strategy in `src/browser/selectors.ts` is hardcoded CSS selectors: class names, `data-` attributes, and `aria-label` strings anchored to specific DOM node positions. **All 7 tools fail silently when LinkedIn updates its DOM** — a pattern called "selector drift". LinkedIn's frontend is rebuilt on a regular cycle; CSS class names are hashed and change with every deploy. The `selectors.ts` file requires manual maintenance after each LinkedIn redesign.

Additional problems:
- No rate limiting built in — sending InMails or running searches at arbitrary speed risks account restriction.
- No session health detection — when a LinkedIn session expires, tools return empty results with no notification.
- ToS risk is not documented in the codebase, creating compliance ambiguity.

### What Was Researched

**Ruv's `agent-browser` in claude-flow v3** (`@claude-flow/browser` package, `infrastructure/agent-browser-adapter.ts`) provides a Playwright-based browser MCP server with a fundamentally different approach to element resolution:

**ARIA snapshot approach**: `snapshot()` captures the page's accessibility tree and assigns short `ref` IDs (`e1`, `e2`, ...) to every interactive element. The LLM reads structured output like:

```
role=button [ref=e12] name="Send InMail"
role=textbox [ref=e23] label="Search leads"
role=link [ref=e7] name="Daniel Alberttis - Engineering Manager"
```

The LLM then acts on `e12`, `e23`, or `e7` — zero hardcoded CSS. Element resolution happens at *runtime* from the live accessibility tree, not at *code-write time* from a developer-maintained selector file.

**Semantic locator family**: `findByRole()`, `findByText()`, `findByLabel()`, `findByPlaceholder()`, `findByTestId()`, `findFirst()`, `findLast()`, `findNth()` — Playwright's resilient locators. These target ARIA roles and visible labels, not DOM structure.

**CDP connect**: A `connect(port)` method attaches to a live Chrome session via CDP. Combined with the OpenClaw relay, this means the user's authenticated browser session is available to agents — no second login, no separate headless Chrome.

**Workflow templates**: Pre-built templates for auth, data-extraction, form-submission, navigation, testing, and monitoring.

**Trajectory tracking**: Records action sequences, integrating with SONA learning (ADR-009).

**OpenClaw Chrome relay**: A Chrome extension that runs a CDP relay at `127.0.0.1:18792`. Any tool speaking CDP on that port can control the user's live, authenticated Chrome session. This eliminates the need to manage a separate browser authentication state.

**`hubspot-linkedin` project** (local at `/Users/danielalberttis/Desktop/Projects/hubspot-linkedin`): A Chrome extension content script approach for LinkedIn-to-HubSpot contact sync. This project is now superseded by the CDP/ARIA approach for two reasons:

1. Content script injection is subsumed by the CDP + ARIA snapshot pattern — full-page control without extension installation on each target.
2. The extension's own extraction logic in `companyExtraction.ts` already reveals that the most reliable data comes from `aria-label` attributes (Strategy 0 in that file), independently confirming the ARIA-first direction.

The field extraction schema from this project remains useful as a reference schema for the LinkedIn Hand (company name, size, industry, location, LinkedIn URL, contact details, title, tenure).

### Key Architectural Insight: Code-Write Time vs. Runtime Element Resolution

| Approach | When element is resolved | What breaks on redesign |
|----------|--------------------------|------------------------|
| CSS selector (`selectors.ts`) | Code-write time — developer hardcodes class or attribute | Silent failure — selector returns empty, no error |
| ARIA snapshot + ref | Runtime — LLM reads fresh accessibility tree at each action | Nothing — the new snapshot reflects the new structure |

LinkedIn has legal and accessibility compliance incentives to maintain ARIA attributes (`aria-label`, `role`, WCAG 2.1). CSS class names (generated by Webpack, PostCSS, or CSS-in-JS) have no such constraint and change freely. The ARIA snapshot approach exploits this asymmetry.

### Current OpenFang State

- No browser automation capability of any kind
- MCP server support exists: `drivers/mod.rs`, `[[mcp_servers]]` array in `config.toml`
- Docker sandbox: `--cap-drop ALL`, `--read-only`, network flag — browser driver runs outside the sandbox boundary
- 30+ LLM drivers, cost metering system, Hand system for tool wrappers
- RVF HNSW vector store, SONA learning framework (ADR-009), ruvllm routing (ADR-010)

---

## Decision

The browser automation layer is delivered in four phases. Phases 1 and 2 are sequential. Phases 3 and 4 are parallel once Phase 2 is complete.

```
Phase 1 (MCP config, ~1h)  ──▶  Phase 2 (openfang-browser crate, ~1 week)
                                        │
                                        ├──▶  Phase 3 (Web Scraper Hand, ~3 days)
                                        │
                                        └──▶  Phase 4 (LinkedIn Hand, ~3 days)
```

---

### 1. What Changes — and What Doesn't

The existing LLM driver layer (`AnthropicDriver`, `GeminiDriver`, `OpenAIDriver`) is unchanged. The existing Hand system (`crates/openfang-runtime/src/hands/`) is unchanged in interface. `agent-browser` is an external CLI binary; it is not vendored and not compiled into the workspace.

```
Agent runtime
      │
      ├── [existing] LLM drivers (anthropic, openai, gemini, groq, ...)
      │
      └── [new] BrowserDriver (Phase 2)
            │
            ├── agent-browser CLI (external, via stdio MCP or subprocess)
            │     └── Playwright (headless or CDP-attached)
            │           └── CDP relay at 127.0.0.1:18792 (OpenClaw Chrome extension)
            │
            ├── ARIA snapshot → ref map → LLM reads → acts on ref
            │
            ├── Rate limiter (per-domain action budget)
            │
            ├── Cost metering (browser sessions attributed per agent)
            │
            └── Trajectory recorder → SONA hook (ADR-009)
```

**Phase 1 is zero Rust code.** It registers `agent-browser` as an MCP server in `~/.openfang/config.toml`. All agents gain browser tools via the existing MCP infrastructure immediately.

**Phase 2** wraps `agent-browser` in a first-party Rust crate `crates/openfang-browser`, providing rate limiting, cost metering, and SONA trajectory integration that the raw MCP config cannot provide.

**Phases 3 and 4** build Hands on top of `BrowserDriver`. Neither Hand contains hardcoded CSS selectors.

---

### 2. Phase 1 — MCP Config (Zero-Code, Available Immediately)

Add `agent-browser` as an MCP server in `~/.openfang/config.toml`:

```toml
[[mcp_servers]]
name        = "agent-browser"
transport   = "stdio"
command     = "agent-browser"
args        = ["mcp"]

[mcp_servers.env]
CDP_ENDPOINT = "http://127.0.0.1:18792"
```

**What this gives every agent immediately:**
- `browser_snapshot` — capture ARIA tree of the current page
- `browser_click` — click element by ARIA ref (e.g. `e12`)
- `browser_fill` — fill input by ARIA ref
- `browser_navigate` — navigate to URL
- `browser_find_by_role` — locate element by ARIA role + name
- `browser_find_by_text` — locate element by visible text
- `browser_find_by_label` — locate element by accessible label
- `browser_screenshot` — capture screenshot for debugging

No rate limiting, no cost metering, no trajectory recording — those are Phase 2. Phase 1 is for validating the ARIA snapshot approach works against live pages before writing Rust.

**Prerequisite**: `npm install -g agent-browser` on the host. Node.js runtime is required. This is the only Node.js dependency in the OpenFang stack.

**Phase 1 tests:**
- `test_agent_browser_mcp_tools_available` — MCP tool list from `agent-browser mcp` includes expected tool names
- `test_snapshot_returns_aria_tree` — `browser_snapshot` on a test page returns a non-empty ARIA ref map
- `test_cdp_connect_to_relay` — CDP connect to `127.0.0.1:18792` succeeds when OpenClaw is active; gracefully falls back to headless when relay is unavailable

---

### 3. Phase 2 — `crates/openfang-browser` Native Driver

A first-party Rust crate wrapping the `agent-browser` CLI. Per ADR-014, this is owned code under `crates/`, not a vendor path.

#### 3.1 Core Types

```rust
// crates/openfang-browser/src/lib.rs

/// A short reference ID assigned by agent-browser to an ARIA element.
/// Example: "e12" refers to the element at position 12 in the snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AriaRef(pub String);

/// A snapshot of the page's accessibility tree, with ref IDs assigned
/// to every interactive element. The LLM reads this and chooses refs to act on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AriaSnapshot {
    pub url:      String,
    pub title:    String,
    pub elements: Vec<AriaElement>,
    pub raw:      String,   // Full text representation for LLM context
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AriaElement {
    pub ref_id:      AriaRef,
    pub role:        String,    // e.g. "button", "textbox", "link", "heading"
    pub name:        String,    // Accessible name / label
    pub value:       Option<String>,
    pub disabled:    bool,
    pub children:    Vec<AriaRef>,
}

/// Browser action result — either success data or a structured error.
#[derive(Debug, Serialize, Deserialize)]
pub enum BrowserResult<T> {
    Ok(T),
    Err(BrowserError),
}

#[derive(Debug, Serialize, Deserialize)]
pub enum BrowserError {
    ElementNotFound { ref_id: String },
    NavigationTimeout { url: String, timeout_ms: u64 },
    ActionTimeout { action: String, timeout_ms: u64 },
    SessionExpired { domain: String },
    RateLimitExceeded { domain: String, limit: u32, window: &'static str },
    CdpConnectionFailed { endpoint: String },
    SnapshotEmpty { url: String },
}
```

#### 3.2 `BrowserDriver` Struct

```rust
// crates/openfang-browser/src/driver.rs

pub struct BrowserDriver {
    config:       BrowserConfig,
    rate_limiter: DomainRateLimiter,   // Per-domain action budget
    cost_meter:   Arc<dyn CostMeter>,  // Plugs into existing cost tracking
    trajectory:   TrajectoryRecorder,  // SONA integration hook (ADR-009)
    process:      Option<ChildProcess>, // agent-browser subprocess handle
}

impl BrowserDriver {
    /// Capture the current page's ARIA accessibility tree.
    /// This is the primary mechanism for element discovery — no CSS selectors.
    pub async fn snapshot(&mut self) -> BrowserResult<AriaSnapshot>;

    /// Navigate to a URL and wait for the page to settle.
    pub async fn navigate(&mut self, url: &str) -> BrowserResult<()>;

    /// Click the element identified by the given ARIA ref.
    pub async fn click(&mut self, ref_id: &AriaRef) -> BrowserResult<()>;

    /// Fill an input element with the given value.
    pub async fn fill(&mut self, ref_id: &AriaRef, value: &str) -> BrowserResult<()>;

    /// Press a keyboard key (e.g. "Enter", "Tab", "Escape").
    pub async fn press(&mut self, ref_id: &AriaRef, key: &str) -> BrowserResult<()>;

    /// Find an element by its ARIA role and accessible name.
    /// Returns the first matching AriaRef.
    pub async fn find_by_role(&mut self, role: &str, name: &str) -> BrowserResult<AriaRef>;

    /// Find an element by its visible text content.
    pub async fn find_by_text(&mut self, text: &str) -> BrowserResult<AriaRef>;

    /// Find an element by its accessible label.
    pub async fn find_by_label(&mut self, label: &str) -> BrowserResult<AriaRef>;

    /// Find an element by its placeholder text.
    pub async fn find_by_placeholder(&mut self, placeholder: &str) -> BrowserResult<AriaRef>;

    /// Connect to a live Chrome session via CDP relay.
    /// Falls back to headless mode if the relay endpoint is unreachable.
    pub async fn connect_cdp(&mut self, endpoint: &str) -> BrowserResult<()>;

    /// Take a screenshot for debugging. Returns PNG bytes.
    pub async fn screenshot(&mut self) -> BrowserResult<Vec<u8>>;

    /// Wait for an element matching the given role/name to appear.
    pub async fn wait_for_role(&mut self, role: &str, name: &str, timeout_ms: u64) -> BrowserResult<AriaRef>;
}
```

#### 3.3 Rate Limiter

```rust
// crates/openfang-browser/src/rate_limiter.rs

pub struct DomainRateLimiter {
    limits:   HashMap<String, RateLimitPolicy>,
    counters: HashMap<String, WindowCounter>,
}

pub struct RateLimitPolicy {
    pub actions_per_minute: u32,
    pub daily_budget:       u32,
    /// Jitter range in milliseconds added between actions (min..max).
    pub jitter_ms:          (u64, u64),
}

impl DomainRateLimiter {
    /// Check whether the action is permitted. If over budget, returns Err.
    /// If permitted, records the action and applies jitter delay.
    pub async fn check_and_record(&mut self, domain: &str) -> Result<(), BrowserError>;
}
```

Domain rate limits are configured per-domain in `[browser.rate_limits]`. The LinkedIn domain has a tighter policy than the default (see §4 Configuration).

#### 3.4 Trajectory Recorder (SONA Integration)

```rust
// crates/openfang-browser/src/trajectory.rs

/// Records browser action sequences as SONA trajectory steps.
/// Hooks into the consolidation.rs sona_step() call point defined in ADR-009.
pub struct TrajectoryRecorder {
    agent_id:    AgentId,
    session_id:  Uuid,
    steps:       Vec<BrowserTrajectoryStep>,
}

pub struct BrowserTrajectoryStep {
    pub step_id:    u64,
    pub action:     String,       // e.g. "click", "fill", "snapshot"
    pub element:    Option<String>, // ARIA role+name of target element
    pub url:        String,
    pub reward:     f32,          // 1.0 for success, 0.0 for error, partial for timeout
    pub latency_ms: u64,
}

impl TrajectoryRecorder {
    /// Record a completed action step and emit to SONA via the consolidation hook.
    pub fn record(&mut self, step: BrowserTrajectoryStep);

    /// Flush trajectory to SONA at session end.
    pub fn flush(&mut self, sona: &mut ConsolidationEngine);
}
```

This is gated on `features = ["browser-sona"]` in `crates/openfang-browser/Cargo.toml`. Builds without the feature compile and run correctly; SONA trajectory recording is simply skipped.

#### 3.5 Cost Metering

Browser sessions are attributed per agent using the same cost tracking infrastructure as LLM calls. Each `BrowserDriver` holds an `Arc<dyn CostMeter>` injected at construction time. A browser session records:

- Session duration (seconds)
- Number of actions taken
- Number of snapshots captured (snapshot context window cost is estimated at a fixed token rate based on page complexity)

These are reported to the existing `/api/budget/agents/{id}` endpoint alongside LLM token costs.

#### 3.6 Feature Flags

```toml
# crates/openfang-browser/Cargo.toml
[features]
default = []
browser-sona = ["dep:openfang-sona"]    # SONA trajectory recording (ADR-009)
```

**Phase 2 tests:**
- `test_browser_driver_snapshot_parses_refs` — `snapshot()` returns `AriaSnapshot` with at least one element; ref IDs are non-empty strings
- `test_find_by_role_locates_element` — `find_by_role("button", "Submit")` returns the correct `AriaRef` on a test page with a known button
- `test_find_by_text_locates_element` — `find_by_text("Next page")` returns the correct `AriaRef`
- `test_find_by_label_locates_element` — `find_by_label("Search")` returns the correct `AriaRef` for a labeled input
- `test_rate_limiter_blocks_over_budget` — after `actions_per_minute` actions within a 60-second window, the next action returns `BrowserError::RateLimitExceeded`
- `test_rate_limiter_resets_after_window` — counter resets after the window expires; subsequent actions succeed
- `test_trajectory_recorded_on_action` — after a successful click, `TrajectoryRecorder::steps` contains one entry with `action = "click"` and `reward = 1.0`
- `test_cost_attributed_to_agent` — after a browser session, the agent's cost record in the metering layer reflects a non-zero browser session cost
- `test_cdp_fallback_to_headless` — when CDP relay is unreachable, `connect_cdp()` falls back to headless mode without error

---

### 4. Configuration

New section in `KernelConfig` / `config.toml`. Per ADR-013 workflow requirements, a new config field requires: struct field + `#[serde(default)]` + `Default` impl entry + Serialize/Deserialize derives.

```toml
[browser]
enabled  = false                               # Disabled by default — opt-in
cdp_endpoint = "http://127.0.0.1:18792"        # OpenClaw relay default
headless = false                               # Use live session via CDP by default
navigation_timeout_ms = 30000
action_timeout_ms     = 10000

[browser.rate_limits.default]
actions_per_minute = 60
daily_budget       = 2000
jitter_ms          = [500, 1500]               # Random jitter between actions (ms)

[browser.rate_limits."www.linkedin.com"]
actions_per_minute = 20
daily_budget       = 500
jitter_ms          = [2000, 5000]              # Slower, more human-like cadence

[browser.rate_limits."www.linkedin.com".linkedin]
inmail_per_hour    = 10                        # Hard limit on InMail sends
search_per_hour    = 30                        # Search requests per hour
```

Corresponding Rust types in `crates/openfang-types/src/config.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BrowserConfig {
    pub enabled:               bool,
    pub cdp_endpoint:          String,
    pub headless:              bool,
    pub navigation_timeout_ms: u64,
    pub action_timeout_ms:     u64,
    pub rate_limits:           BrowserRateLimits,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            enabled:               false,
            cdp_endpoint:          "http://127.0.0.1:18792".to_string(),
            headless:              false,
            navigation_timeout_ms: 30_000,
            action_timeout_ms:     10_000,
            rate_limits:           BrowserRateLimits::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BrowserRateLimits {
    pub default:  RateLimitPolicy,
    pub domains:  HashMap<String, RateLimitPolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitPolicy {
    pub actions_per_minute: u32,
    pub daily_budget:       u32,
    pub jitter_ms:          (u64, u64),
}

impl Default for RateLimitPolicy {
    fn default() -> Self {
        Self {
            actions_per_minute: 60,
            daily_budget:       2000,
            jitter_ms:          (500, 1500),
        }
    }
}
```

When `browser.enabled = false` (the default), no `BrowserDriver` is constructed and no MCP server is spawned. Zero regression risk for existing deployments.

---

### 5. Phase 3 — Web Scraper Hand

A built-in Hand that exposes schema-driven web scraping to agents. No hardcoded CSS selectors anywhere in the implementation.

**Key file**: `crates/openfang-runtime/src/hands/web_scraper.rs`

#### 5.1 Hand Interface

```rust
/// Web scraping Hand — extracts structured data from any URL using ARIA snapshots.
/// Schema describes what data to extract; the LLM discovers how to find it.
pub struct WebScraperHand {
    driver: Arc<Mutex<BrowserDriver>>,
}

/// Schema definition for what data to extract from a page.
/// Keys are field names in the output; values are natural-language descriptions
/// of what the field contains (used by the LLM to identify elements).
pub type ExtractionSchema = HashMap<String, String>;

impl WebScraperHand {
    /// Scrape a single page and extract fields matching the schema.
    pub async fn scrape(
        &mut self,
        url: &str,
        schema: ExtractionSchema,
    ) -> Result<Vec<HashMap<String, String>>, BrowserError>;

    /// Scrape multiple pages by following pagination automatically.
    /// Pagination is detected via find_by_role("button", "Next") or
    /// find_by_text("Next page") — no hardcoded pagination selectors.
    pub async fn scrape_paginated(
        &mut self,
        url: &str,
        schema: ExtractionSchema,
        max_pages: usize,
    ) -> Result<Vec<HashMap<String, String>>, BrowserError>;

    /// Export scraped results to CSV.
    pub async fn export_csv(
        &mut self,
        url: &str,
        schema: ExtractionSchema,
    ) -> Result<String, BrowserError>;
}
```

#### 5.2 Scrape Workflow

The workflow for `scrape(url, schema)`:

1. `BrowserDriver::navigate(url)` — navigate and wait for load
2. `BrowserDriver::snapshot()` — capture ARIA tree
3. LLM receives: (a) the ARIA snapshot, (b) the schema, (c) system prompt instructing it to extract each schema field from the snapshot using ref IDs
4. LLM outputs a JSON array of field→value mappings, one entry per data row found in the snapshot
5. For `scrape_paginated`: after extracting rows, `find_by_role("button", "Next")` — if found, click and repeat from step 2. If `find_by_text("Next page")` also returns a ref, prefer the `role=button` variant (more reliable ARIA signal). Stop when neither is found or `max_pages` is reached.

The LLM drives extraction. The Hand does not contain field-specific logic — it passes the schema and snapshot to the LLM and parses the structured output. This means the Hand survives any site redesign: on the next scrape, the new snapshot reflects the new structure, and the LLM adapts its extraction accordingly.

#### 5.3 Agent Tool Registration

```rust
// In crates/openfang-runtime/src/hands/mod.rs
// Registered when browser.enabled = true

pub fn register_browser_hands(registry: &mut HandRegistry, driver: Arc<Mutex<BrowserDriver>>) {
    registry.register("web_scrape", WebScraperHand::new(driver.clone()));
    // LinkedIn Hand registered separately (Phase 4)
}
```

**Phase 3 tests:**
- `test_web_scrape_extracts_schema_fields` — scrape a local test page with known content; verify all schema fields are populated in the output
- `test_web_scrape_paginates` — test page with 3 paginated pages; verify all 3 pages' rows appear in the output and page count matches
- `test_web_scrape_no_css_selectors_used` — static analysis / grep confirms no CSS selector strings (`.class-name`, `#id`, `[data-*]`) appear in `web_scraper.rs`
- `test_web_scrape_empty_snapshot_returns_error` — when `BrowserDriver::snapshot()` returns `BrowserError::SnapshotEmpty`, `scrape()` propagates the error without panicking
- `test_web_scrape_export_csv_valid_format` — exported CSV has correct headers and row count

---

### 6. Phase 4 — LinkedIn Hand (Snapshot-Based, Not a CSS Selector Fork)

**Explicit architectural constraint**: Do NOT fork or extend `globodai-mcp-linkedin-sales-navigator`. Its CSS selector maintenance burden is the problem being solved, not a base to build on. The LinkedIn Hand is built on `BrowserDriver` and `AriaSnapshot` from the ground up.

**Key file**: `crates/openfang-runtime/src/hands/linkedin.rs`

#### 6.1 Lead Schema

The field schema is derived from the `hubspot-linkedin` project's extraction reference and aligns with LinkedIn's Sales Navigator data model:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lead {
    pub full_name:    String,
    pub title:        String,
    pub company:      String,
    pub company_size: Option<String>,
    pub industry:     Option<String>,
    pub location:     Option<String>,
    pub linkedin_url: String,
    pub email:        Option<String>,
    pub phone:        Option<String>,
    pub tenure:       Option<String>,   // Time in current role, if visible
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchFilters {
    pub keywords:      Option<String>,
    pub title:         Option<String>,
    pub company:       Option<String>,
    pub location:      Option<String>,
    pub industry:      Option<String>,
    pub company_size:  Option<Vec<String>>,  // e.g. ["11-50", "51-200"]
    pub connection_of: Option<String>,       // LinkedIn member URL
}
```

#### 6.2 LinkedIn Hand Interface

```rust
pub struct LinkedInHand {
    driver:    Arc<Mutex<BrowserDriver>>,
    base_url:  String,   // Default: "https://www.linkedin.com/sales"
}

impl LinkedInHand {
    /// Search Sales Navigator with the given filters.
    /// Navigates the search UI by filling filter fields by ARIA label,
    /// then extracts results from the snapshot.
    pub async fn search(
        &mut self,
        filters: SearchFilters,
    ) -> Result<Vec<Lead>, BrowserError>;

    /// Fetch a LinkedIn profile by URL.
    /// Navigates to the URL and extracts lead fields from the snapshot.
    pub async fn profile(
        &mut self,
        profile_url: &str,
    ) -> Result<Lead, BrowserError>;

    /// Send an InMail to a profile.
    /// Navigates to profile, clicks find_by_role("button", "Send InMail"),
    /// fills the subject and body, clicks "Send".
    /// Rate-limited to [browser.rate_limits."www.linkedin.com".linkedin].inmail_per_hour.
    pub async fn send_inmail(
        &mut self,
        profile_url: &str,
        subject: &str,
        message: &str,
        dry_run: bool,    // If true, fills the form but does not click Send
    ) -> Result<InMailResult, BrowserError>;

    /// Export all leads from a named list.
    /// Paginates via find_by_text("Next") until exhausted.
    pub async fn export_list(
        &mut self,
        list_name: &str,
    ) -> Result<Vec<Lead>, BrowserError>;
}
```

#### 6.3 ARIA-Based Element Resolution

All UI interactions in the LinkedIn Hand use ARIA-based resolution. The pattern for each operation:

**Filling a filter field:**
```rust
let field_ref = self.driver.find_by_label("Title").await?;
self.driver.fill(&field_ref, &filters.title.unwrap()).await?;
```

**Clicking Send InMail:**
```rust
let button_ref = self.driver.find_by_role("button", "Send InMail").await?;
self.driver.click(&button_ref).await?;
```

**Paginating results:**
```rust
loop {
    let snapshot = self.driver.snapshot().await?;
    // LLM extracts leads from snapshot
    leads.extend(extract_leads_from_snapshot(&snapshot)?);

    match self.driver.find_by_role("button", "Next").await {
        BrowserResult::Ok(next_ref) => { self.driver.click(&next_ref).await?; }
        BrowserResult::Err(BrowserError::ElementNotFound { .. }) => break,
        BrowserResult::Err(e) => return Err(e),
    }
}
```

When LinkedIn redesigns its UI, the ARIA tree in the next snapshot reflects the new structure. `find_by_role("button", "Send InMail")` continues to work as long as LinkedIn maintains ARIA compliance — which they are legally required to do.

#### 6.4 Session Expiry Detection

When any LinkedIn operation returns a snapshot that contains a login prompt — detectable via `find_by_role("button", "Sign in")` or `find_by_text("Join now")` returning a valid ref — the Hand emits a session expiry event:

```rust
pub enum LinkedInEvent {
    SessionExpired { detected_at: DateTime<Utc>, page: String },
    RateLimitWarning { action: &'static str, remaining: u32 },
    InMailSent { profile_url: String, dry_run: bool },
}
```

The kernel routes `SessionExpired` events to the dashboard notification system. The agent receives `BrowserError::SessionExpired { domain: "www.linkedin.com" }` and pauses its workflow. The dashboard displays: "LinkedIn session expired — please re-authenticate in your browser."

#### 6.5 ToS Notice

**LinkedIn automation is a violation of LinkedIn's Terms of Service.** This Hand is provided for internal research and development use only. OpenFang operators who deploy the LinkedIn Hand accept full responsibility for compliance with LinkedIn's ToS. This notice must be reproduced in any documentation or UI that exposes the LinkedIn Hand to end users.

The `dry_run: bool` parameter on `send_inmail` exists specifically to allow testing the full workflow without sending actual InMails.

**Phase 4 tests:**
- `test_linkedin_search_finds_leads` — mock driver returning a pre-captured ARIA snapshot; verify field extraction produces the correct `Lead` structs
- `test_linkedin_send_inmail_dry_run` — `send_inmail(..., dry_run: true)` fills the InMail form but does not click Send; snapshot after the call confirms the dialog is still open
- `test_linkedin_session_expiry_notification` — mock driver returns a snapshot containing a "Sign in" button; verify `BrowserError::SessionExpired` is returned and `LinkedInEvent::SessionExpired` is emitted
- `test_linkedin_rate_limiter_respected` — after `inmail_per_hour` InMail sends within one hour, the next `send_inmail` returns `BrowserError::RateLimitExceeded`
- `test_linkedin_pagination_iterates_all_pages` — mock driver returns 3 paginated snapshots; verify all leads across all pages appear in the result and the loop terminates when "Next" is no longer present
- `test_linkedin_no_css_selectors_used` — static analysis / grep confirms no CSS selector strings appear in `linkedin.rs`

---

### 7. File Layout

Phase 2 adds one new crate. Phases 3 and 4 add two new files within the existing runtime crate:

```
crates/
├── openfang-browser/           ← new (Phase 2)
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── driver.rs           ← BrowserDriver
│       ├── rate_limiter.rs     ← DomainRateLimiter
│       ├── trajectory.rs       ← TrajectoryRecorder (feature = "browser-sona")
│       └── aria.rs             ← AriaSnapshot, AriaElement, AriaRef
│
└── openfang-runtime/
    └── src/
        └── hands/
            ├── mod.rs          ← register_browser_hands() added (Phase 3)
            ├── web_scraper.rs  ← WebScraperHand (Phase 3, new)
            └── linkedin.rs     ← LinkedInHand (Phase 4, new)

~/.openfang/
├── shared.rvf              ← ADR-003
├── shared.access.db        ← ADR-003
├── sona.rvf                ← ADR-009
├── ruvllm.rvf              ← ADR-010
└── agents/
    ├── {agent_id}.rvf
    └── {agent_id}.access.db
```

No new files are added to `~/.openfang/`. Browser sessions are ephemeral and do not require a persistent store file.

---

### 8. Implementation Order

#### Phase 1 — MCP Config (Available Immediately)

> Prerequisite: `npm install -g agent-browser` on the host.

1. Add `[[mcp_servers]]` block with `agent-browser` config to `~/.openfang/config.toml` (documented in CLAUDE.md and operator guide, not hardcoded).
2. Verify MCP tool list returned by `agent-browser mcp` matches expected tools.
3. Test CDP connection to OpenClaw relay (127.0.0.1:18792).
4. Confirm snapshot on a test page returns ARIA refs.

Tests: `test_agent_browser_mcp_tools_available`, `test_snapshot_returns_aria_tree`, `test_cdp_connect_to_relay`

Acceptance criteria: Agent can call `browser_snapshot` and receive a non-empty ARIA tree from `agent-browser`. CDP relay connection succeeds when OpenClaw is active.

#### Phase 2 — `crates/openfang-browser` (Prerequisite for Phases 3 and 4)

1. Create `crates/openfang-browser/` with `Cargo.toml` (no external deps beyond `tokio`, `serde`, `uuid`; `openfang-sona` is optional behind `browser-sona` feature).
2. Implement `aria.rs` — `AriaSnapshot`, `AriaElement`, `AriaRef`, `BrowserError`.
3. Implement `rate_limiter.rs` — `DomainRateLimiter`, `RateLimitPolicy`, `WindowCounter`.
4. Implement `driver.rs` — `BrowserDriver` wrapping `agent-browser` subprocess. Wire `DomainRateLimiter` on every action.
5. Implement `trajectory.rs` behind `browser-sona` feature.
6. Add `BrowserConfig` and `BrowserRateLimits` to `crates/openfang-types/src/config.rs`.
7. Wire `BrowserDriver` construction into kernel when `browser.enabled = true`.
8. Tests: full Phase 2 test list (§3, Phase 2 tests).

Acceptance criteria: `cargo test -p openfang-browser` passes. Rate limiter blocks correctly. Trajectory steps are recorded on action when `browser-sona` feature is enabled. `cargo clippy -p openfang-browser -- -D warnings` exits zero.

#### Phase 3 — Web Scraper Hand (Parallel with Phase 4 once Phase 2 is complete)

1. Implement `crates/openfang-runtime/src/hands/web_scraper.rs`.
2. Register `web_scrape` Hand in `hands/mod.rs` when `browser.enabled = true`.
3. Tests: full Phase 3 test list (§5, Phase 3 tests).

Acceptance criteria: `cargo test -p openfang-runtime` passes with `web_scrape` Hand tests. `grep -r "\\." crates/openfang-runtime/src/hands/web_scraper.rs` finds zero CSS selector strings. Paginated scrape of a 3-page test fixture returns all rows.

#### Phase 4 — LinkedIn Hand (Parallel with Phase 3 once Phase 2 is complete)

1. Implement `crates/openfang-runtime/src/hands/linkedin.rs`.
2. Register `linkedin_search`, `linkedin_profile`, `linkedin_send_inmail`, `linkedin_export_list` Hands when `browser.enabled = true`.
3. Implement session expiry detection and `LinkedInEvent` emission.
4. Tests: full Phase 4 test list (§6, Phase 4 tests).

Acceptance criteria: `cargo test -p openfang-runtime` passes with LinkedIn Hand tests. `grep -r "\\." crates/openfang-runtime/src/hands/linkedin.rs` finds zero CSS selector strings. `send_inmail(..., dry_run: true)` does not trigger actual InMail send. Session expiry emits dashboard notification.

---

## Consequences

### Positive

- Agents gain general browser control — any website becomes queryable, not just those with published APIs
- ARIA snapshot approach eliminates selector drift: element resolution happens at runtime via the live accessibility tree, not at code-write time via a developer-maintained CSS selector file. When LinkedIn redesigns its DOM, the next snapshot reflects the new structure automatically
- LinkedIn capability without maintaining a CSS selector file — `find_by_role("button", "Send InMail")` continues to work as long as LinkedIn maintains ARIA compliance, which they are legally required to do
- Web scraping is schema-driven: operators describe what data they want in natural language; the LLM discovers how to find it in the current page structure
- CDP via OpenClaw relay means no separate Chrome process and no second login — agents use the user's real, already-authenticated session
- Rate limiting and session health monitoring are built into the `BrowserDriver` layer — prevents LinkedIn account restriction from automated traffic patterns
- Trajectory recording feeds SONA learning (ADR-009) — browser action patterns that lead to successful extractions are promoted to the pattern store and improve future performance
- Phase 1 is usable immediately with zero Rust code — the `agent-browser` MCP config wires into the existing MCP infrastructure

### Negative

- `agent-browser` CLI must be installed as an npm package — adds a Node.js runtime dependency to host machines. This is the only Node.js dependency in the OpenFang stack
- ARIA snapshots of complex pages (deep navigation trees, many interactive elements) can be verbose — the LLM context window must accommodate the full snapshot text. Very large pages may require chunked snapshot strategies (scroll-to-section before snapshot)
- LinkedIn automation violates LinkedIn's Terms of Service — must be documented clearly in all user-facing surfaces; intended for internal and research use only. Operators assume full compliance responsibility
- CDP relay requires the OpenClaw Chrome extension to be installed for live-session mode. Without it, the driver falls back to headless mode, which requires re-authentication for LinkedIn and other session-protected sites
- `dry_run: bool` on `send_inmail` reduces testing confidence — functional tests cannot verify the actual InMail network call without risking a real send, so Phase 4 tests are limited to dry-run and snapshot inspection

### Neutral

- Phase 1 (MCP config) is immediately usable with zero Rust code; the full native integration with rate limiting, cost metering, and SONA trajectory recording is Phase 2
- The `hubspot-linkedin` Chrome extension project at `/Users/danielalberttis/Desktop/Projects/hubspot-linkedin` is superseded and will not be migrated. Its field extraction schema (`companyExtraction.ts` Strategy 0) is used as a reference for the `Lead` struct in Phase 4; no code is copied
- The `globodai-mcp-linkedin-sales-navigator` MCP server remains available as a config option for operators who prefer its high-level 7-tool API. The LinkedIn Hand is the recommended path for selector-drift-free operation. Both can coexist in `[[mcp_servers]]`
- Browser session costs are attributed per agent alongside LLM token costs using the existing metering infrastructure — no new billing model is required

---

## References

| Source | What it documents |
|--------|------------------|
| `docs/research/ruv-browser-selector-analysis.md` | ARIA snapshot vs CSS selector analysis; agent-browser architecture |
| `docs/research/mcp-linkedin-sales-navigator-analysis.md` | globodai LinkedIn MCP tools, selector drift problem, ToS notes |
| `docs/research/openclaw-analysis.md` | Chrome extension CDP relay at 127.0.0.1:18792 |
| `docs/adr/ADR-009-memory-intelligence.md` | SONA framework; `sona_step()` hook that trajectory recording uses |
| `docs/adr/ADR-010-llm-intelligence-layer.md` | ruvllm routing; cost metering patterns this ADR follows |
| `docs/adr/ADR-011-ruvix-interface-contract.md` | Constraint 3: typed Queue semantics for inter-agent trajectory sharing |
| `docs/adr/ADR-013-development-workflow.md` | Mandatory dev workflow; config field requirements |
| `docs/adr/ADR-014-vendor-to-first-party-crates.md` | Why openfang-browser is a first-party crate, not a vendor path |
| `crates/openfang-runtime/src/drivers/mod.rs` | Existing driver registration pattern; `create_driver()` model |
| `crates/openfang-runtime/src/hands/mod.rs` | Existing Hand system (where Phase 3 and 4 integrate) |
| `crates/openfang-types/src/config.rs` | `KernelConfig` (where `[browser]` section goes); serde pattern to follow |
| `@claude-flow/browser` package, `infrastructure/agent-browser-adapter.ts` | Ruv's agent-browser integration; ARIA snapshot pattern reference |
