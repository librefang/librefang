# Sidecar Channel Configure-from-Dashboard Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Operator can configure a first-party sidecar channel (telegram, ntfy) entirely from the dashboard — fill a form, click Save, see it work — instead of hand-editing `~/.librefang/config.toml`.

**Architecture:** Three layers wire together. (1) SDK adapters self-describe via a `--describe` CLI flag returning JSON schema. (2) Rust API caches schemas at boot, surfaces them on `/api/channels`, accepts saves via a new `POST /api/channels/sidecar/{name}/configure` endpoint that splits user input across `config.toml` (non-secret) and `~/.librefang/secrets.env` (secret) — both already loaded into sidecar child env automatically. (3) Dashboard renders a real form per schema, submits via the new endpoint, invalidates the channels query.

**Tech Stack:** Python 3.8+ stdlib (sidecar SDK), Rust 1.83 (axum/tokio/serde/toml_edit/anyhow/tracing), React 19 + TanStack Query v5 (dashboard).

**Reuses existing infrastructure:**
- `~/.librefang/secrets.env` loader already implemented in `crates/librefang-extensions/src/dotenv.rs` (loaded at daemon startup, inherited by sidecar children — no schema change needed for secrets).
- `HotAction::ReloadChannels` already clears `mesh.channel_adapters` at `crates/librefang-kernel/src/kernel/config_reload_ops.rs:246-256`, and the bridge cycle re-inits from `kernel.config_ref().sidecar_channels` — extend diff to include `sidecar_channels` field.
- `toml_edit = "0.25"` already in `crates/librefang-api/Cargo.toml:35`.

**Estimated effort:** ~3 days across 6 phases.

---

## Phase 1 — SDK `--describe` CLI (~0.5 day)

Adds a stdlib-only self-description protocol to the sidecar SDK so adapters can declare their config schema; ships telegram + ntfy schemas as the first consumers.

### Task 1.1: Add `Field` and `Schema` types to `librefang.sidecar.protocol`

**Files:**
- Modify: `sdk/python/librefang/sidecar/protocol.py` (append at end)
- Test: `sdk/python/tests/test_describe_schema.py` (new)

**Step 1: Write the failing test**

```python
# sdk/python/tests/test_describe_schema.py
"""Schema-shape contract tests for the sidecar self-description protocol."""
from librefang.sidecar.protocol import Field, Schema


def test_field_secret_required():
    f = Field("TELEGRAM_BOT_TOKEN", "Bot Token", "secret",
              required=True, placeholder="123:ABC...")
    assert f.to_dict() == {
        "key": "TELEGRAM_BOT_TOKEN",
        "label": "Bot Token",
        "type": "secret",
        "required": True,
        "placeholder": "123:ABC...",
        "advanced": False,
    }


def test_field_advanced_list():
    f = Field("ALLOWED_USERS", "Allowed User IDs", "list", advanced=True)
    assert f.to_dict()["advanced"] is True
    assert f.to_dict()["type"] == "list"
    assert f.to_dict()["required"] is False  # default


def test_schema_serializes_fields():
    s = Schema(
        name="telegram",
        display_name="Telegram",
        description="Telegram Bot API adapter",
        fields=[
            Field("TELEGRAM_BOT_TOKEN", "Bot Token", "secret", required=True),
            Field("ALLOWED_USERS", "Allowed User IDs", "list"),
        ],
    )
    out = s.to_dict()
    assert out["name"] == "telegram"
    assert len(out["fields"]) == 2
    assert out["fields"][0]["key"] == "TELEGRAM_BOT_TOKEN"
    assert out["fields"][0]["type"] == "secret"


def test_field_rejects_unknown_type():
    import pytest
    with pytest.raises(ValueError, match="unknown field type"):
        Field("X", "X", "magic")
```

**Step 2: Run test to verify it fails**

```
cd sdk/python && python -m pytest tests/test_describe_schema.py -v
```
Expected: `ImportError: cannot import name 'Field'`

**Step 3: Implement Field/Schema in protocol.py**

Append to `sdk/python/librefang/sidecar/protocol.py`:

```python
# ---------------------------------------------------------------------------
# Self-description schema — emitted by `python -m <adapter> --describe`.
# Mirrors the FieldType enum in librefang-api's CHANNEL_REGISTRY so the
# dashboard can render either kind of channel with one form component.
# ---------------------------------------------------------------------------

_ALLOWED_FIELD_TYPES = {"text", "secret", "number", "list", "bool", "select"}


class Field:
    """One configurable field for a sidecar adapter.

    `type=secret` is routed to ~/.librefang/secrets.env on save (never
    written to config.toml). Every other type is stored in the
    [sidecar_channels.env] table of config.toml — these are the env
    vars the child process reads via os.environ on startup.
    """

    __slots__ = ("key", "label", "type", "required", "placeholder",
                 "advanced", "options")

    def __init__(self, key, label, type, *, required=False,
                 placeholder="", advanced=False, options=None):
        if type not in _ALLOWED_FIELD_TYPES:
            raise ValueError(
                f"unknown field type {type!r}; "
                f"allowed: {sorted(_ALLOWED_FIELD_TYPES)}"
            )
        self.key = key
        self.label = label
        self.type = type
        self.required = required
        self.placeholder = placeholder
        self.advanced = advanced
        self.options = options  # for type=select

    def to_dict(self):
        d = {
            "key": self.key,
            "label": self.label,
            "type": self.type,
            "required": self.required,
            "placeholder": self.placeholder,
            "advanced": self.advanced,
        }
        if self.options is not None:
            d["options"] = list(self.options)
        return d


class Schema:
    """Self-description payload emitted by `<adapter> --describe`."""

    __slots__ = ("name", "display_name", "description", "fields")

    def __init__(self, name, display_name, description, fields):
        self.name = name
        self.display_name = display_name
        self.description = description
        self.fields = list(fields)

    def to_dict(self):
        return {
            "name": self.name,
            "display_name": self.display_name,
            "description": self.description,
            "fields": [f.to_dict() for f in self.fields],
        }
```

**Step 4: Run test to verify it passes**

```
cd sdk/python && python -m pytest tests/test_describe_schema.py -v
```
Expected: all 4 tests PASS.

**Step 5: Commit**

```bash
cd /tmp/librefang-sidecar-configure
git add sdk/python/librefang/sidecar/protocol.py sdk/python/tests/test_describe_schema.py
git commit -m "feat(sdk/python): add Field/Schema types for sidecar --describe protocol"
```

---

### Task 1.2: Add `describe_main()` helper to runtime + plumb through `run_stdio`

**Files:**
- Modify: `sdk/python/librefang/sidecar/runtime.py` (add `describe_main`, modify `run_stdio` to dispatch `--describe`)
- Modify: `sdk/python/librefang/sidecar/__init__.py` (export `Field`, `Schema`, `describe_main`)
- Test: `sdk/python/tests/test_describe_main.py` (new)

**Step 1: Write the failing test**

```python
# sdk/python/tests/test_describe_main.py
"""End-to-end test: `python -m <adapter> --describe` writes JSON to stdout."""
import io
import json
import sys
from contextlib import redirect_stdout

from librefang.sidecar import Field, Schema, describe_main


class _AdapterWithSchema:
    SCHEMA = Schema(
        name="dummy",
        display_name="Dummy",
        description="Test adapter",
        fields=[Field("DUMMY_KEY", "Key", "text", required=True)],
    )


def test_describe_main_prints_json_and_exits_zero():
    buf = io.StringIO()
    rc = 99
    with redirect_stdout(buf):
        rc = describe_main(_AdapterWithSchema())
    assert rc == 0
    payload = json.loads(buf.getvalue())
    assert payload["name"] == "dummy"
    assert payload["fields"][0]["key"] == "DUMMY_KEY"


def test_describe_main_missing_schema_exits_two():
    class NoSchema:
        pass
    buf = io.StringIO()
    rc = 99
    with redirect_stdout(buf):
        rc = describe_main(NoSchema())
    assert rc == 2
    # Empty stdout on failure — daemon parses stdout, must not feed it junk
    assert buf.getvalue() == ""
```

**Step 2: Run test to verify it fails**

```
cd sdk/python && python -m pytest tests/test_describe_main.py -v
```
Expected: `ImportError: cannot import name 'describe_main'`

**Step 3: Implement `describe_main` and dispatch in `run_stdio`**

Append to `sdk/python/librefang/sidecar/runtime.py` (after `with_backoff`, before `run`):

```python
def describe_main(adapter):
    """Print the adapter's SCHEMA as JSON to stdout and return 0.

    Returns 2 (no schema declared) — same exit code as the missing-token
    case in telegram.py so the daemon's describe-cache logic can treat
    "no schema" identically to "describe failed" and fall back.
    """
    import json as _json
    schema = getattr(adapter, "SCHEMA", None)
    if schema is None:
        # Log via stderr (stdout is reserved for the JSON payload).
        log.error("adapter has no SCHEMA attribute; --describe failed",
                  adapter=type(adapter).__name__)
        return 2
    sys.stdout.write(_json.dumps(schema.to_dict()))
    sys.stdout.flush()
    return 0
```

Modify `run_stdio` in the same file. Find the existing definition (it currently looks like `def run_stdio(adapter, ...)`) and add a CLI dispatch at the very top:

```python
def run_stdio(adapter, *, ready_interval=2.0, ready_max_attempts=5):
    """Entry point. With `--describe` in argv, emit schema JSON and exit;
    otherwise run the normal stdio JSON-RPC loop."""
    if "--describe" in sys.argv[1:]:
        raise SystemExit(describe_main(adapter))
    # ... existing body unchanged ...
```

Update `sdk/python/librefang/sidecar/__init__.py` — extend the `from .protocol import (...)` block to also import `Field, Schema`, extend the `from .runtime import (...)` block to import `describe_main`, and append both to `__all__`.

**Step 4: Run test to verify it passes**

```
cd sdk/python && python -m pytest tests/test_describe_main.py tests/test_describe_schema.py -v
```
Expected: all 6 tests PASS.

**Step 5: Commit**

```bash
cd /tmp/librefang-sidecar-configure
git add sdk/python/librefang/sidecar/runtime.py sdk/python/librefang/sidecar/__init__.py sdk/python/tests/test_describe_main.py
git commit -m "feat(sdk/python): support \`--describe\` CLI flag on run_stdio for adapter self-description"
```

---

### Task 1.3: Declare SCHEMA on telegram + ntfy adapters

**Files:**
- Modify: `sdk/python/librefang/sidecar/adapters/telegram.py` (add `SCHEMA` class attribute)
- Modify: `sdk/python/librefang/sidecar/adapters/ntfy.py` (add `SCHEMA` class attribute)
- Test: `sdk/python/tests/test_first_party_describe.py` (new)

**Step 1: Write the failing test**

```python
# sdk/python/tests/test_first_party_describe.py
"""First-party adapters expose stable SCHEMA shapes."""
import subprocess
import sys
import json


def _describe(module):
    out = subprocess.check_output(
        [sys.executable, "-m", module, "--describe"],
        stderr=subprocess.PIPE,
    )
    return json.loads(out)


def test_telegram_describe_contract():
    s = _describe("librefang.sidecar.adapters.telegram")
    assert s["name"] == "telegram"
    keys = {f["key"]: f for f in s["fields"]}
    assert keys["TELEGRAM_BOT_TOKEN"]["type"] == "secret"
    assert keys["TELEGRAM_BOT_TOKEN"]["required"] is True
    assert keys["ALLOWED_USERS"]["type"] == "list"
    assert keys["TELEGRAM_CLEAR_DONE_REACTION"]["type"] == "bool"


def test_ntfy_describe_contract():
    s = _describe("librefang.sidecar.adapters.ntfy")
    assert s["name"] == "ntfy"
    keys = {f["key"]: f for f in s["fields"]}
    assert keys["NTFY_TOPIC"]["required"] is True
    assert keys["NTFY_TOKEN"]["type"] == "secret"
```

**Step 2: Run test to verify it fails**

```
cd sdk/python && python -m pytest tests/test_first_party_describe.py -v
```
Expected: `KeyError: 'TELEGRAM_BOT_TOKEN'` (the adapters don't emit schemas yet).

**Step 3: Add SCHEMA to telegram.py and ntfy.py**

In `sdk/python/librefang/sidecar/adapters/telegram.py`, find `class TelegramAdapter(SidecarAdapter):` and insert `SCHEMA` as a class attribute (after the `capabilities = [...]` line). Also extend the import block at the top of the file:

```python
from librefang.sidecar import Field, Schema
```

```python
class TelegramAdapter(SidecarAdapter):
    capabilities = ["typing", "reaction", "interactive", "thread", "streaming"]

    SCHEMA = Schema(
        name="telegram",
        display_name="Telegram",
        description="Telegram Bot API adapter (out-of-process sidecar)",
        fields=[
            Field("TELEGRAM_BOT_TOKEN", "Bot Token", "secret",
                  required=True,
                  placeholder="123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11"),
            Field("ALLOWED_USERS", "Allowed User IDs", "list",
                  placeholder="123456789, 987654321",
                  advanced=True),
            Field("TELEGRAM_CLEAR_DONE_REACTION", "Clear done reaction",
                  "bool", advanced=True),
        ],
    )

    def __init__(self) -> None:
        # ... existing body unchanged ...
```

In `sdk/python/librefang/sidecar/adapters/ntfy.py`, do the same for `NtfyAdapter`:

```python
from librefang.sidecar import Field, Schema

class NtfyAdapter(SidecarAdapter):
    capabilities = ["notification"]  # whatever it currently is

    SCHEMA = Schema(
        name="ntfy",
        display_name="ntfy",
        description="ntfy.sh pub/sub notifications (out-of-process sidecar)",
        fields=[
            Field("NTFY_TOPIC", "Topic", "text",
                  required=True, placeholder="my-topic"),
            Field("NTFY_SERVER_URL", "Server URL", "text",
                  placeholder="https://ntfy.sh", advanced=True),
            Field("NTFY_TOKEN", "Auth Token", "secret",
                  placeholder="tk_...", advanced=True),
            Field("NTFY_ACCOUNT_ID", "Account ID (multi-bot)", "text",
                  placeholder="topic-42", advanced=True),
        ],
    )

    def __init__(self) -> None:
        # ... existing body unchanged ...
```

**Step 4: Run test to verify it passes**

```
cd sdk/python && python -m pytest tests/test_first_party_describe.py -v
```
Expected: both contract tests PASS.

**Step 5: Commit**

```bash
cd /tmp/librefang-sidecar-configure
git add sdk/python/librefang/sidecar/adapters/telegram.py sdk/python/librefang/sidecar/adapters/ntfy.py sdk/python/tests/test_first_party_describe.py
git commit -m "feat(sdk/python): declare SCHEMA on first-party telegram & ntfy adapters"
```

---

## Phase 2 — Rust schema cache + discovery row `fields` (~0.5 day)

Daemon spawns each catalog adapter once with `--describe` at boot, caches the JSON, and surfaces `fields[]` on `/api/channels` so the dashboard knows what form to render.

### Task 2.1: Add `describe_sidecar()` helper to api crate

**Files:**
- Create: `crates/librefang-api/src/routes/sidecar_describe.rs`
- Modify: `crates/librefang-api/src/routes/mod.rs` (add `pub(crate) mod sidecar_describe;`)
- Test: `crates/librefang-api/tests/sidecar_describe_test.rs` (new)

**Step 1: Write the failing test**

Use a fake adapter that ships in the SDK already (we'll pretend telegram suffices, but for CI determinism the test should use a tiny inline script). For the first cut, just test against the real installed telegram module so it doubles as an integration check.

```rust
// crates/librefang-api/tests/sidecar_describe_test.rs
use librefang_api::routes::sidecar_describe::{describe_sidecar, SidecarSchema};

#[tokio::test]
async fn describe_telegram_returns_schema_or_skips_when_sdk_missing() {
    let result = describe_sidecar(
        "python3",
        &["-m".into(), "librefang.sidecar.adapters.telegram".into()],
    )
    .await;
    let schema = match result {
        Ok(s) => s,
        // Local dev without `pip install -e sdk/python` is a valid state;
        // skip rather than fail so CI without the SDK works.
        Err(e) => {
            eprintln!("describe failed (SDK not installed?): {e}");
            return;
        }
    };
    assert_eq!(schema.name, "telegram");
    let bot_token = schema
        .fields
        .iter()
        .find(|f| f.key == "TELEGRAM_BOT_TOKEN")
        .expect("schema must declare TELEGRAM_BOT_TOKEN");
    assert_eq!(bot_token.field_type, "secret");
    assert!(bot_token.required);
}

#[tokio::test]
async fn describe_failing_command_returns_err() {
    let result = describe_sidecar(
        "python3",
        &["-c".into(), "import sys; sys.exit(2)".into()],
    )
    .await;
    assert!(result.is_err());
}
```

**Step 2: Run test to verify it fails**

```
cd /tmp/librefang-sidecar-configure
cargo test -p librefang-api --test sidecar_describe_test 2>&1 | tail -20
```
Expected: compile error — `describe_sidecar` not defined.

**Step 3: Implement `describe_sidecar`**

```rust
// crates/librefang-api/src/routes/sidecar_describe.rs
//! Spawn a sidecar adapter with `--describe` and parse the JSON schema
//! it prints on stdout. Used at daemon boot to populate the Add-picker
//! form for each first-party SIDECAR_CATALOG entry.

use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::process::Command;

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct SidecarSchemaField {
    pub key: String,
    pub label: String,
    #[serde(rename = "type")]
    pub field_type: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub placeholder: String,
    #[serde(default)]
    pub advanced: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct SidecarSchema {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub fields: Vec<SidecarSchemaField>,
}

/// Spawn `<command> <args> --describe`, parse stdout as JSON.
///
/// Timeout is 5s — describe should be sub-second; if it hangs (the
/// adapter's __init__ blocks on a network call before reading argv,
/// for example) we'd rather skip than block daemon boot.
pub async fn describe_sidecar(
    command: &str,
    args: &[String],
) -> Result<SidecarSchema, String> {
    let mut full_args: Vec<String> = args.to_vec();
    full_args.push("--describe".into());

    let fut = Command::new(command)
        .args(&full_args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();

    let out = tokio::time::timeout(Duration::from_secs(5), fut)
        .await
        .map_err(|_| format!("`{command} ...--describe` timed out after 5s"))?
        .map_err(|e| format!("spawn failed: {e}"))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "describe exited {}: {}",
            out.status.code().unwrap_or(-1),
            stderr.trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str::<SidecarSchema>(stdout.trim())
        .map_err(|e| format!("invalid describe JSON: {e}; raw stdout: {stdout}"))
}
```

Add `pub(crate) mod sidecar_describe;` to `crates/librefang-api/src/routes/mod.rs` (place it alphabetically with the other `pub(crate) mod` declarations).

**Step 4: Run test to verify it passes**

```
cargo test -p librefang-api --test sidecar_describe_test 2>&1 | tail -20
```
Expected: 2 pass (or first one skipped if SDK absent).

**Step 5: Commit**

```bash
git add crates/librefang-api/src/routes/sidecar_describe.rs crates/librefang-api/src/routes/mod.rs crates/librefang-api/tests/sidecar_describe_test.rs
git commit -m "feat(api): spawn sidecar with --describe to retrieve config schema"
```

---

### Task 2.2: Cache schemas at boot and emit `fields[]` on `/api/channels`

**Files:**
- Modify: `crates/librefang-api/src/routes/channels.rs`:
  - `SidecarCatalogEntry`: drop `config_template`, add `command: &'static str`, `args: &'static [&'static str]`
  - Add `static SIDECAR_SCHEMA_CACHE: OnceLock<HashMap<&'static str, SidecarSchema>>`
  - Add `pub(crate) async fn populate_sidecar_schema_cache()` called by `lib.rs` boot path
  - Rewrite `sidecar_discovery_rows()` to emit `fields[]` from the cache
- Modify: `crates/librefang-api/src/lib.rs` (call cache populate inside the kernel-boot path)
- Modify: `crates/librefang-api/tests/channels_routes_test.rs` (update `channels_list_without_sidecar_surfaces_discovery_catalog` to also check `fields[]` shape, but tolerate empty when SDK absent)

**Step 1: Write the failing test**

Extend `crates/librefang-api/tests/channels_routes_test.rs` — find the existing `channels_list_without_sidecar_surfaces_discovery_catalog` test and add a new sibling test:

```rust
#[tokio::test(flavor = "multi_thread")]
async fn channels_list_discovery_rows_carry_form_fields_when_schema_cached() {
    // Pre-populate the schema cache with a synthetic telegram schema so the
    // test runs deterministically without depending on `pip install -e
    // sdk/python` on every CI box.
    librefang_api::routes::channels::__test_seed_sidecar_schema_cache(&[(
        "telegram",
        librefang_api::routes::sidecar_describe::SidecarSchema {
            name: "telegram".into(),
            display_name: "Telegram".into(),
            description: "Telegram Bot API adapter".into(),
            fields: vec![
                librefang_api::routes::sidecar_describe::SidecarSchemaField {
                    key: "TELEGRAM_BOT_TOKEN".into(),
                    label: "Bot Token".into(),
                    field_type: "secret".into(),
                    required: true,
                    placeholder: "123:ABC".into(),
                    advanced: false,
                    options: None,
                },
            ],
        },
    )]);

    let h = boot().await;
    let (_status, body) = json_request(&h, Method::GET, "/api/channels", None).await;
    let arr = body["items"].as_array().expect("items");
    let tg = arr.iter().find(|r| r["name"] == "telegram").expect("telegram row");
    let fields = tg["fields"].as_array().expect("fields[]");
    assert!(!fields.is_empty(), "discovery row must carry fields when cached");
    assert_eq!(fields[0]["key"], "TELEGRAM_BOT_TOKEN");
    assert_eq!(fields[0]["type"], "secret");
    assert_eq!(fields[0]["required"], true);
}
```

**Step 2: Run test to verify it fails**

```
cargo test -p librefang-api --test channels_routes_test channels_list_discovery_rows_carry_form_fields_when_schema_cached 2>&1 | tail -20
```
Expected: compile error — `__test_seed_sidecar_schema_cache` and `routes::channels::...` not exported.

**Step 3: Implement schema cache + rewrite discovery rows**

In `crates/librefang-api/src/routes/channels.rs`:

1. Add imports near the top:
```rust
use std::sync::OnceLock;
use super::sidecar_describe::{describe_sidecar, SidecarSchema};
```

2. Replace the `SidecarCatalogEntry` struct (currently has `name/display_name/description/config_template`) with:

```rust
struct SidecarCatalogEntry {
    name: &'static str,
    display_name: &'static str,
    description: &'static str,
    command: &'static str,
    args: &'static [&'static str],
}

const SIDECAR_CATALOG: &[SidecarCatalogEntry] = &[
    SidecarCatalogEntry {
        name: "telegram",
        display_name: "Telegram",
        description: "Telegram Bot API adapter (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.telegram"],
    },
    SidecarCatalogEntry {
        name: "ntfy",
        display_name: "ntfy",
        description: "ntfy.sh pub/sub notifications (out-of-process sidecar)",
        command: "python3",
        args: &["-m", "librefang.sidecar.adapters.ntfy"],
    },
];
```

3. Add the cache + populator:

```rust
static SIDECAR_SCHEMA_CACHE: OnceLock<
    std::sync::RwLock<HashMap<&'static str, SidecarSchema>>,
> = OnceLock::new();

fn schema_cache() -> &'static std::sync::RwLock<HashMap<&'static str, SidecarSchema>> {
    SIDECAR_SCHEMA_CACHE.get_or_init(|| std::sync::RwLock::new(HashMap::new()))
}

/// Spawn `<command> <args> --describe` for every catalog entry and
/// cache the schemas. Called once at daemon boot. Failures (SDK not
/// installed, describe crashed) are logged at WARN and the row falls
/// back to an empty `fields[]` — the operator then sees the description
/// + setup-steps text but no form. This keeps daemon boot resilient
/// in dev environments without `pip install -e sdk/python`.
pub async fn populate_sidecar_schema_cache() {
    for entry in SIDECAR_CATALOG {
        let args: Vec<String> = entry.args.iter().map(|s| s.to_string()).collect();
        match describe_sidecar(entry.command, &args).await {
            Ok(schema) => {
                tracing::info!(
                    adapter = entry.name,
                    fields = schema.fields.len(),
                    "sidecar schema cached"
                );
                schema_cache()
                    .write()
                    .unwrap()
                    .insert(entry.name, schema);
            }
            Err(e) => {
                tracing::warn!(
                    adapter = entry.name,
                    error = %e,
                    "sidecar --describe failed; discovery card will have no form fields"
                );
            }
        }
    }
}

#[doc(hidden)]
pub fn __test_seed_sidecar_schema_cache(entries: &[(&'static str, SidecarSchema)]) {
    let mut guard = schema_cache().write().unwrap();
    guard.clear();
    for (k, v) in entries {
        guard.insert(k, v.clone());
    }
}
```

4. Rewrite `sidecar_discovery_rows()` — its existing body builds rows with empty `fields[]` and a `setup_steps`/`config_template`. Replace with:

```rust
fn sidecar_discovery_rows(
    sidecar: &[librefang_types::config::SidecarChannelConfig],
) -> Vec<serde_json::Value> {
    let registry: std::collections::HashSet<&str> =
        CHANNEL_REGISTRY.iter().map(|c| c.name).collect();
    let mut covered: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for sc in sidecar {
        let kind = sc.channel_type.as_deref().unwrap_or(sc.name.as_str());
        covered.insert(kind);
        covered.insert(sc.name.as_str());
    }

    let cache_guard = schema_cache().read().unwrap();
    let mut rows = Vec::new();
    for entry in SIDECAR_CATALOG {
        if registry.contains(entry.name) || covered.contains(entry.name) {
            continue;
        }
        let fields: Vec<serde_json::Value> = cache_guard
            .get(entry.name)
            .map(|s| s.fields.iter()
                .map(|f| serde_json::json!({
                    "key": f.key,
                    "label": f.label,
                    "type": f.field_type,
                    "required": f.required,
                    "placeholder": f.placeholder,
                    "advanced": f.advanced,
                    "options": f.options,
                }))
                .collect())
            .unwrap_or_default();

        rows.push(serde_json::json!({
            "name": entry.name,
            "display_name": entry.display_name,
            "icon": "SC",
            "description": entry.description,
            "category": "sidecar",
            "setup_type": "sidecar",
            "configured": false,
            "instance_count": 0,
            "has_token": false,
            "fields": fields,
            "setup_steps": [
                "Runs as an out-of-process sidecar adapter",
                "Fill the form to save credentials to ~/.librefang/secrets.env \
                 (secrets) and ~/.librefang/config.toml (non-secrets)",
            ],
        }));
    }
    rows
}
```

5. In `crates/librefang-api/src/lib.rs`, find the function that brings up `AppState` (search for `pub async fn boot` or `pub async fn serve`) and add a call to `routes::channels::populate_sidecar_schema_cache().await;` after the kernel is ready but before the router starts accepting requests.

**Step 4: Run all channels tests**

```
cargo test -p librefang-api --test channels_routes_test 2>&1 | tail -30
```
Expected: previously-passing tests still pass, new test passes.

**Step 5: Commit**

```bash
git add crates/librefang-api/src/routes/channels.rs crates/librefang-api/src/routes/mod.rs crates/librefang-api/src/lib.rs crates/librefang-api/tests/channels_routes_test.rs
git commit -m "feat(api): cache sidecar schemas at boot, emit fields[] on /api/channels"
```

---

## Phase 3 — Save endpoint (~1 day)

`POST /api/channels/sidecar/{name}/configure` accepts a form payload, splits it across `secrets.env` (secret-typed fields) and `config.toml` (every other field + the `[[sidecar_channels]]` boilerplate), then triggers hot-reload.

### Task 3.1: Write helper to upsert one `KEY=VALUE` into `~/.librefang/secrets.env`

**Files:**
- Create: `crates/librefang-api/src/routes/secrets_env.rs`
- Modify: `crates/librefang-api/src/routes/mod.rs` (declare module)
- Test: `crates/librefang-api/tests/secrets_env_test.rs` (new)

**Step 1: Write the failing test**

```rust
// crates/librefang-api/tests/secrets_env_test.rs
use librefang_api::routes::secrets_env::upsert_secret;
use std::fs;
use tempfile::NamedTempFile;

#[test]
fn upsert_creates_file_with_600_perms() {
    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path().to_path_buf();
    fs::remove_file(&path).unwrap();   // we want upsert to create it
    upsert_secret(&path, "FOO", "bar").unwrap();

    let content = fs::read_to_string(&path).unwrap();
    assert_eq!(content.trim(), "FOO=bar");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "secrets file must be mode 600");
    }
}

#[test]
fn upsert_replaces_existing_key_preserves_other_lines() {
    let tmp = NamedTempFile::new().unwrap();
    fs::write(tmp.path(),
        "# top comment\n\
         A=1\n\
         FOO=old\n\
         B=2\n").unwrap();

    upsert_secret(tmp.path(), "FOO", "new").unwrap();

    let content = fs::read_to_string(tmp.path()).unwrap();
    assert_eq!(content,
        "# top comment\n\
         A=1\n\
         FOO=new\n\
         B=2\n");
}

#[test]
fn upsert_appends_when_key_absent() {
    let tmp = NamedTempFile::new().unwrap();
    fs::write(tmp.path(), "A=1\n").unwrap();

    upsert_secret(tmp.path(), "B", "2").unwrap();

    let content = fs::read_to_string(tmp.path()).unwrap();
    assert_eq!(content, "A=1\nB=2\n");
}

#[test]
fn upsert_rejects_value_with_newline() {
    let tmp = NamedTempFile::new().unwrap();
    let err = upsert_secret(tmp.path(), "K", "line1\nline2").unwrap_err();
    assert!(err.to_string().contains("newline"));
}
```

Add `tempfile = "3"` to `crates/librefang-api/Cargo.toml` under `[dev-dependencies]` if not already present.

**Step 2: Run test to verify it fails**

```
cargo test -p librefang-api --test secrets_env_test 2>&1 | tail -20
```
Expected: compile error — module doesn't exist.

**Step 3: Implement `upsert_secret`**

```rust
// crates/librefang-api/src/routes/secrets_env.rs
//! Append/replace a single `KEY=VALUE` line in ~/.librefang/secrets.env.
//!
//! The file is loaded into `std::env` at daemon boot by
//! `librefang_extensions::dotenv::load_dotenv()`; any non-system-env
//! KEY in this file becomes visible to sidecar child processes through
//! normal env inheritance. We only ever touch ONE line per call —
//! comments, ordering, and unrelated keys are preserved.

use std::fs;
use std::io::Write;
use std::path::Path;

pub fn upsert_secret(path: &Path, key: &str, value: &str) -> Result<(), String> {
    if value.contains('\n') || value.contains('\r') {
        return Err(format!(
            "secret value for `{key}` must not contain a newline"
        ));
    }
    if key.contains('=') || key.trim() != key || key.is_empty() {
        return Err(format!("invalid secret key `{key}`"));
    }

    let original = fs::read_to_string(path).unwrap_or_default();
    let mut out = String::with_capacity(original.len() + key.len() + value.len() + 2);
    let mut replaced = false;
    for line in original.lines() {
        let trimmed = line.trim_start();
        if !replaced && !trimmed.starts_with('#') {
            if let Some((existing_key, _)) = trimmed.split_once('=') {
                if existing_key.trim() == key {
                    out.push_str(&format!("{key}={value}\n"));
                    replaced = true;
                    continue;
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    if !replaced {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&format!("{key}={value}\n"));
    }

    // Atomic write to a sibling tempfile then rename.
    let parent = path.parent().ok_or("secrets path has no parent dir")?;
    fs::create_dir_all(parent)
        .map_err(|e| format!("mkdir {parent:?}: {e}"))?;
    let tmp = parent.join(format!(
        ".secrets.env.tmp.{}",
        std::process::id()
    ));
    {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)
            .map_err(|e| format!("open {tmp:?}: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perm = fs::Permissions::from_mode(0o600);
            fs::set_permissions(&tmp, perm)
                .map_err(|e| format!("chmod 600 {tmp:?}: {e}"))?;
        }
        f.write_all(out.as_bytes())
            .map_err(|e| format!("write {tmp:?}: {e}"))?;
        f.sync_all().ok();
    }
    fs::rename(&tmp, path)
        .map_err(|e| format!("rename {tmp:?} -> {path:?}: {e}"))?;
    Ok(())
}
```

Declare the module in `crates/librefang-api/src/routes/mod.rs`:
```rust
pub mod secrets_env;
```

**Step 4: Run test to verify it passes**

```
cargo test -p librefang-api --test secrets_env_test 2>&1 | tail -20
```
Expected: 4 tests PASS.

**Step 5: Commit**

```bash
git add crates/librefang-api/src/routes/secrets_env.rs crates/librefang-api/src/routes/mod.rs crates/librefang-api/Cargo.toml crates/librefang-api/tests/secrets_env_test.rs
git commit -m "feat(api): upsert_secret helper writes single KEY=VALUE to secrets.env atomically"
```

---

### Task 3.2: Write `upsert_sidecar_in_config_toml` using `toml_edit`

**Files:**
- Create: `crates/librefang-api/src/routes/sidecar_toml.rs`
- Modify: `crates/librefang-api/src/routes/mod.rs`
- Test: `crates/librefang-api/tests/sidecar_toml_test.rs` (new)

**Step 1: Write the failing test**

```rust
// crates/librefang-api/tests/sidecar_toml_test.rs
use librefang_api::routes::sidecar_toml::upsert_sidecar_block;
use std::collections::BTreeMap;
use std::fs;
use tempfile::NamedTempFile;

fn pairs(input: &[(&str, &str)]) -> BTreeMap<String, String> {
    input.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

#[test]
fn appends_when_absent_preserves_existing_keys() {
    let tmp = NamedTempFile::new().unwrap();
    fs::write(tmp.path(),
        "[default_model]\nprovider = \"ollama\"\n").unwrap();

    upsert_sidecar_block(
        tmp.path(),
        "telegram",
        "telegram",
        "python3",
        &["-m", "librefang.sidecar.adapters.telegram"],
        &pairs(&[("ALLOWED_USERS", "1,2")]),
    ).unwrap();

    let content = fs::read_to_string(tmp.path()).unwrap();
    assert!(content.contains("[default_model]"));
    assert!(content.contains("[[sidecar_channels]]"));
    assert!(content.contains("name = \"telegram\""));
    assert!(content.contains("channel_type = \"telegram\""));
    assert!(content.contains("ALLOWED_USERS = \"1,2\""));
}

#[test]
fn replaces_existing_block_with_same_name() {
    let tmp = NamedTempFile::new().unwrap();
    fs::write(tmp.path(),
        "[[sidecar_channels]]\n\
         name = \"telegram\"\n\
         channel_type = \"telegram\"\n\
         command = \"python3\"\n\
         args = [\"-m\", \"librefang.sidecar.adapters.telegram\"]\n\
         \n\
         [sidecar_channels.env]\n\
         TELEGRAM_BOT_TOKEN = \"old\"\n\
         OBSOLETE = \"x\"\n").unwrap();

    upsert_sidecar_block(
        tmp.path(),
        "telegram",
        "telegram",
        "python3",
        &["-m", "librefang.sidecar.adapters.telegram"],
        &pairs(&[("ALLOWED_USERS", "1,2")]),
    ).unwrap();

    let content = fs::read_to_string(tmp.path()).unwrap();
    assert!(!content.contains("OBSOLETE"),
            "stale env keys must be replaced wholesale, not merged");
    assert!(!content.contains("TELEGRAM_BOT_TOKEN"),
            "token field is never in config.toml — goes to secrets.env");
    assert!(content.contains("ALLOWED_USERS = \"1,2\""));
}

#[test]
fn does_not_touch_other_sidecar_blocks() {
    let tmp = NamedTempFile::new().unwrap();
    fs::write(tmp.path(),
        "[[sidecar_channels]]\nname = \"ntfy\"\nchannel_type = \"ntfy\"\n\
         command = \"python3\"\nargs = [\"-m\",\"librefang.sidecar.adapters.ntfy\"]\n\
         [sidecar_channels.env]\nNTFY_TOPIC = \"alerts\"\n\
         \n\
         [[sidecar_channels]]\nname = \"telegram\"\nchannel_type = \"telegram\"\n\
         command = \"python3\"\nargs = [\"-m\",\"librefang.sidecar.adapters.telegram\"]\n\
         [sidecar_channels.env]\n").unwrap();

    upsert_sidecar_block(
        tmp.path(),
        "telegram", "telegram", "python3",
        &["-m", "librefang.sidecar.adapters.telegram"],
        &pairs(&[("ALLOWED_USERS", "99")]),
    ).unwrap();

    let content = fs::read_to_string(tmp.path()).unwrap();
    assert!(content.contains("NTFY_TOPIC = \"alerts\""),
            "ntfy block must be untouched");
    assert!(content.contains("ALLOWED_USERS = \"99\""));
}
```

**Step 2: Run test to verify it fails**

```
cargo test -p librefang-api --test sidecar_toml_test 2>&1 | tail -20
```
Expected: compile error.

**Step 3: Implement using `toml_edit`**

```rust
// crates/librefang-api/src/routes/sidecar_toml.rs
//! Idempotent upsert of one `[[sidecar_channels]]` block in config.toml,
//! identified by its `name`. Uses toml_edit to preserve formatting,
//! comments, and key ordering of every other section.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use toml_edit::{value, Array, ArrayOfTables, DocumentMut, Item, Table};

pub fn upsert_sidecar_block(
    path: &Path,
    name: &str,
    channel_type: &str,
    command: &str,
    args: &[&str],
    env: &BTreeMap<String, String>,
) -> Result<(), String> {
    let original = fs::read_to_string(path).unwrap_or_default();
    let mut doc: DocumentMut = original
        .parse()
        .map_err(|e| format!("parse {path:?}: {e}"))?;

    // Build the replacement table for this single block.
    let mut block = Table::new();
    block["name"] = value(name);
    block["channel_type"] = value(channel_type);
    block["command"] = value(command);
    let mut args_arr = Array::new();
    for a in args {
        args_arr.push(*a);
    }
    block["args"] = value(args_arr);
    let mut env_table = Table::new();
    for (k, v) in env {
        env_table[k] = value(v.clone());
    }
    env_table.set_implicit(false);
    block["env"] = Item::Table(env_table);

    let aot_item = doc
        .entry("sidecar_channels")
        .or_insert_with(|| Item::ArrayOfTables(ArrayOfTables::new()));
    let aot = aot_item
        .as_array_of_tables_mut()
        .ok_or_else(|| "config.toml: `sidecar_channels` is not an array-of-tables".to_string())?;

    // Replace by `name`; if absent, append.
    let mut replaced = false;
    for i in 0..aot.len() {
        let existing_name = aot
            .get(i)
            .and_then(|t| t.get("name"))
            .and_then(|i| i.as_str())
            .unwrap_or("");
        if existing_name == name {
            *aot.get_mut(i).expect("indexed") = block.clone();
            replaced = true;
            break;
        }
    }
    if !replaced {
        aot.push(block);
    }

    // Atomic write.
    let parent = path.parent().ok_or("config path has no parent")?;
    let tmp = parent.join(format!(".config.toml.tmp.{}", std::process::id()));
    fs::write(&tmp, doc.to_string())
        .map_err(|e| format!("write {tmp:?}: {e}"))?;
    fs::rename(&tmp, path)
        .map_err(|e| format!("rename: {e}"))?;
    Ok(())
}
```

Declare module: `pub mod sidecar_toml;` in `routes/mod.rs`.

**Step 4: Run tests to verify**

```
cargo test -p librefang-api --test sidecar_toml_test 2>&1 | tail -20
```
Expected: 3 tests PASS.

**Step 5: Commit**

```bash
git add crates/librefang-api/src/routes/sidecar_toml.rs crates/librefang-api/src/routes/mod.rs crates/librefang-api/tests/sidecar_toml_test.rs
git commit -m "feat(api): upsert_sidecar_block edits config.toml [[sidecar_channels]] idempotently"
```

---

### Task 3.3: Wire `POST /api/channels/sidecar/{name}/configure` endpoint

**Files:**
- Modify: `crates/librefang-api/src/routes/channels.rs`:
  - Register route in `router()`
  - Implement `configure_sidecar_channel` handler
- Modify: `crates/librefang-api/src/routes/config.rs`: extend `is_writable_config_path` to no longer block `sidecar_channels` (we own the write path now)
- Test: `crates/librefang-api/tests/channels_routes_test.rs`

**Step 1: Write the failing test**

Add to `channels_routes_test.rs`:

```rust
#[tokio::test(flavor = "multi_thread")]
async fn configure_sidecar_writes_secret_to_env_and_nonsecret_to_toml() {
    use std::collections::HashMap;

    // Pre-seed cache so endpoint can validate fields.
    librefang_api::routes::channels::__test_seed_sidecar_schema_cache(&[(
        "telegram",
        librefang_api::routes::sidecar_describe::SidecarSchema {
            name: "telegram".into(),
            display_name: "Telegram".into(),
            description: "test".into(),
            fields: vec![
                librefang_api::routes::sidecar_describe::SidecarSchemaField {
                    key: "TELEGRAM_BOT_TOKEN".into(), label: "Token".into(),
                    field_type: "secret".into(), required: true,
                    placeholder: "".into(), advanced: false, options: None,
                },
                librefang_api::routes::sidecar_describe::SidecarSchemaField {
                    key: "ALLOWED_USERS".into(), label: "Users".into(),
                    field_type: "list".into(), required: false,
                    placeholder: "".into(), advanced: false, options: None,
                },
            ],
        },
    )]);

    let h = boot_with_temp_home().await;  // see helper below
    let mut body = HashMap::new();
    body.insert("values", serde_json::json!({
        "TELEGRAM_BOT_TOKEN": "secret-123",
        "ALLOWED_USERS": "1,2,3",
    }));
    let (status, _resp) = json_request(&h, Method::POST,
        "/api/channels/sidecar/telegram/configure",
        Some(serde_json::to_value(body).unwrap())).await;
    assert_eq!(status, StatusCode::OK);

    // Verify side effects.
    let home = h.home_dir();
    let secrets = std::fs::read_to_string(home.join("secrets.env")).unwrap();
    assert!(secrets.contains("TELEGRAM_BOT_TOKEN=secret-123"));
    assert!(!secrets.contains("ALLOWED_USERS"),
        "non-secret fields must NOT land in secrets.env");

    let toml = std::fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(toml.contains("[[sidecar_channels]]"));
    assert!(toml.contains("name = \"telegram\""));
    assert!(toml.contains("ALLOWED_USERS = \"1,2,3\""));
    assert!(!toml.contains("TELEGRAM_BOT_TOKEN"),
        "secrets must NOT leak into config.toml");
}

#[tokio::test(flavor = "multi_thread")]
async fn configure_sidecar_missing_required_returns_400() {
    librefang_api::routes::channels::__test_seed_sidecar_schema_cache(&[(
        "telegram",
        SidecarSchema { /* with required TELEGRAM_BOT_TOKEN */ ... },
    )]);
    let h = boot_with_temp_home().await;
    let body = serde_json::json!({ "values": { "ALLOWED_USERS": "1" } });
    let (status, resp) = json_request(&h, Method::POST,
        "/api/channels/sidecar/telegram/configure", Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(resp.to_string().contains("TELEGRAM_BOT_TOKEN"));
}

#[tokio::test(flavor = "multi_thread")]
async fn configure_sidecar_unknown_name_returns_404() {
    let h = boot_with_temp_home().await;
    let body = serde_json::json!({ "values": {} });
    let (status, _) = json_request(&h, Method::POST,
        "/api/channels/sidecar/nonexistent/configure", Some(body)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
```

You will need a `boot_with_temp_home()` helper that returns a Harness with `home_dir()` (a tempdir) and sets `LIBREFANG_HOME` for the kernel. Add it next to the existing `boot()` helper — model on the kernel's existing test harness pattern.

**Step 2: Run test to verify it fails**

```
cargo test -p librefang-api --test channels_routes_test configure_sidecar 2>&1 | tail -30
```
Expected: route 404 (not registered) / handler missing.

**Step 3: Implement handler + register route**

In `channels.rs` `router()` add:
```rust
.route(
    "/channels/sidecar/{name}/configure",
    axum::routing::post(configure_sidecar_channel),
)
```

Add handler:

```rust
#[derive(serde::Deserialize, utoipa::ToSchema)]
pub struct ConfigureSidecarBody {
    pub values: HashMap<String, String>,
}

#[utoipa::path(
    post,
    path = "/api/channels/sidecar/{name}/configure",
    tag = "channels",
    request_body = ConfigureSidecarBody,
    responses(
        (status = 200, description = "Saved; reload triggered"),
        (status = 400, description = "Missing required field / invalid value"),
        (status = 404, description = "Unknown catalog name"),
    )
)]
pub async fn configure_sidecar_channel(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<ConfigureSidecarBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiErrorResponse>)> {
    // 1. Catalog lookup
    let entry = SIDECAR_CATALOG.iter().find(|e| e.name == name).ok_or_else(|| {
        (StatusCode::NOT_FOUND, Json(ApiErrorResponse::new(
            format!("no sidecar adapter named `{name}`"),
        )))
    })?;

    // 2. Pull cached schema for validation
    let schema = schema_cache().read().unwrap().get(&entry.name).cloned();
    let schema = schema.ok_or_else(|| {
        (StatusCode::SERVICE_UNAVAILABLE, Json(ApiErrorResponse::new(
            format!("schema for `{name}` not cached — SDK module may be missing"),
        )))
    })?;

    // 3. Validate required fields present + non-empty
    for f in &schema.fields {
        if f.required {
            let v = body.values.get(&f.key).map(|s| s.trim()).unwrap_or("");
            if v.is_empty() {
                return Err((StatusCode::BAD_REQUEST, Json(ApiErrorResponse::new(
                    format!("required field `{}` is missing or empty", f.key),
                ))));
            }
        }
    }

    // 4. Split: secrets → secrets.env, non-secrets → config.toml env table
    let home = librefang_home();
    let secrets_path = home.join("secrets.env");
    let mut nonsecret_env: std::collections::BTreeMap<String, String> = Default::default();
    for f in &schema.fields {
        if let Some(v) = body.values.get(&f.key) {
            let trimmed = v.trim();
            if trimmed.is_empty() {
                continue;
            }
            if f.field_type == "secret" {
                super::secrets_env::upsert_secret(&secrets_path, &f.key, trimmed)
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiErrorResponse::new(e))))?;
            } else {
                nonsecret_env.insert(f.key.clone(), trimmed.to_string());
            }
        }
    }

    // 5. Upsert config.toml block
    let config_path = home.join("config.toml");
    super::sidecar_toml::upsert_sidecar_block(
        &config_path,
        &entry.name,
        &entry.name,            // channel_type defaults to name
        entry.command,
        entry.args,
        &nonsecret_env,
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiErrorResponse::new(e))))?;

    // 6. Trigger hot-reload (kernel re-reads file, diff produces ReloadChannels)
    let plan = state.kernel.reload_config().await.map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(ApiErrorResponse::new(e)))
    })?;

    Ok(Json(serde_json::json!({
        "status": "saved",
        "hot_actions_applied": plan.hot_actions,
        "restart_required": plan.restart_required,
    })))
}
```

In `crates/librefang-api/src/routes/config.rs`, find the `is_writable_config_path` function and the assertion at line ~3462; the function still returns `false` for `sidecar_channels` (and our new endpoint owns that path), so leave it — we are NOT going through `update_config`, we wrote a dedicated endpoint. Remove only the stale comment block at lines ~3452-3462 if it references `sidecar_channels` as "permanently unwriteable" — replace with a comment pointing at `POST /api/channels/sidecar/{name}/configure` as the right path.

**Step 4: Run all channels tests**

```
cargo test -p librefang-api --test channels_routes_test 2>&1 | tail -30
```
Expected: all pass, 3 new tests pass.

**Step 5: Commit**

```bash
git add crates/librefang-api/src/routes/channels.rs crates/librefang-api/src/routes/config.rs crates/librefang-api/tests/channels_routes_test.rs
git commit -m "feat(api): POST /api/channels/sidecar/{name}/configure splits values across secrets.env & config.toml"
```

---

## Phase 4 — Sidecar inclusion in `HotAction::ReloadChannels` (~0.5 day)

Make `config_reload.rs` notice `sidecar_channels` changes and trigger the existing channel-reload path.

### Task 4.1: Extend diff to cover `sidecar_channels` field

**Files:**
- Modify: `crates/librefang-kernel/src/config_reload.rs` (after the existing `if field_changed(&old.channels, &new.channels)` at line ~313)
- Modify: `crates/librefang-kernel/src/config_reload.rs` (unit tests near line ~666)

**Step 1: Write the failing test**

Find the existing test that asserts `ReloadChannels` fires on channels-config change (search `assert!(plan.hot_actions.contains(&HotAction::ReloadChannels))` — around line 666). Add a sibling test below:

```rust
#[test]
fn sidecar_channels_change_triggers_reload_channels_action() {
    use librefang_types::config::SidecarChannelConfig;
    let mut old = KernelConfig::default();
    let mut new = KernelConfig::default();
    new.sidecar_channels.push(SidecarChannelConfig {
        name: "telegram".into(),
        command: "python3".into(),
        args: vec!["-m".into(), "librefang.sidecar.adapters.telegram".into()],
        channel_type: Some("telegram".into()),
        env: Default::default(),
        ..Default::default()
    });
    let plan = compute_reload_plan(&old, &new);
    assert!(plan.hot_actions.contains(&HotAction::ReloadChannels),
        "adding a [[sidecar_channels]] entry must trigger ReloadChannels");
}
```

(`SidecarChannelConfig` doesn't currently `derive(Default)` — if the spread fails, build it from `serde_json::from_value` of the matching JSON for the test, mirroring `sidecar_telegram()` in `channels_routes_test.rs`.)

**Step 2: Run test to verify it fails**

```
cargo test -p librefang-kernel --lib sidecar_channels_change_triggers_reload_channels_action 2>&1 | tail -15
```
Expected: FAIL — `plan.hot_actions` does not contain `ReloadChannels`.

**Step 3: Add the diff check**

In `crates/librefang-kernel/src/config_reload.rs`, find the existing line:
```rust
if field_changed(&old.channels, &new.channels) {
    plan.hot_actions.push(HotAction::ReloadChannels);
}
```

Add immediately after:
```rust
if field_changed(&old.sidecar_channels, &new.sidecar_channels) {
    // Reuses the same hot action — `mesh.channel_adapters.clear()`
    // forces channel_bridge to re-init from `kernel.config_ref()`,
    // which already iterates `sidecar_channels` on every init pass.
    if !plan.hot_actions.contains(&HotAction::ReloadChannels) {
        plan.hot_actions.push(HotAction::ReloadChannels);
    }
}
```

**Step 4: Run test to verify it passes**

```
cargo test -p librefang-kernel --lib sidecar_channels_change_triggers_reload_channels_action 2>&1 | tail -10
cargo test -p librefang-kernel --lib config_reload 2>&1 | tail -10
```
Expected: new test PASS; the existing battery still PASS.

**Step 5: Commit**

```bash
git add crates/librefang-kernel/src/config_reload.rs
git commit -m "feat(kernel): include sidecar_channels in HotAction::ReloadChannels diff"
```

---

### Task 4.2: Verify bridge cycle picks up newly-added sidecars after `channel_adapters.clear()`

This is **a read-only audit task**, not a code change. The plan-executor must:

1. Re-read `crates/librefang-api/src/channel_bridge.rs` around the lines that build the adapter list (the loop at ~3719 that iterates `sidecar_cfg.sidecar_channels`).
2. Trace where the bridge cycle is triggered after `mesh.channel_adapters.clear()` — find the watcher loop or whatever calls back into `start_channel_bridge`.
3. **Confirm** that a `clear()` → next-cycle path leads back through that loop, so a newly-appended `[[sidecar_channels]]` is picked up. Document the call chain (file:line → file:line) inline in `channel_bridge.rs` as a comment above the `for sidecar_config in &sidecar_cfg.sidecar_channels` loop, prefaced with `// Re-init path:`.
4. If the chain is broken (clear doesn't actually retrigger init), open this as a blocker — Phase 4 design needs revisiting. **Do not proceed to Phase 5 until verified.**

No test, no code change required if the chain is intact. **Commit the doc comment** so future readers don't have to re-derive it:

```bash
git add crates/librefang-api/src/channel_bridge.rs
git commit -m "docs(channel-bridge): document sidecar re-init path after channel_adapters.clear()"
```

---

## Phase 5 — Dashboard: replace read-only modal with real form (~0.5 day)

`handlePick` routes `category === "sidecar"` to a new `SidecarForm` modal that renders the schema-driven fields and posts to the save endpoint.

### Task 5.1: Add `useSaveSidecarConfig` mutation hook

**Files:**
- Modify: `crates/librefang-api/dashboard/src/api.ts` (add `saveSidecarConfig` function + `SidecarSaveResult` type)
- Create: `crates/librefang-api/dashboard/src/lib/mutations/channels.ts` if not present, or modify if it is — append `useSaveSidecarConfig`
- Modify: `crates/librefang-api/dashboard/src/lib/queries/keys.ts` (sanity-check `channelsKeys` is correctly hierarchical)

**Step 1: Write the failing test (vitest)**

Look for existing mutation-hook tests under `crates/librefang-api/dashboard/src/lib/mutations/__tests__/` or co-located. If they exist, add:

```ts
// crates/librefang-api/dashboard/src/lib/mutations/channels.test.ts
import { describe, it, expect, vi } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import { useSaveSidecarConfig } from "./channels";
import * as api from "../../api";

vi.mock("../../api");

function wrapper(qc: QueryClient) {
  return ({ children }: { children: ReactNode }) =>
    <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
}

describe("useSaveSidecarConfig", () => {
  it("calls saveSidecarConfig and invalidates channelsKeys.all", async () => {
    const qc = new QueryClient();
    const invalidate = vi.spyOn(qc, "invalidateQueries");
    vi.mocked(api.saveSidecarConfig).mockResolvedValue({
      status: "saved",
      restart_required: false,
      hot_actions_applied: ["ReloadChannels"],
    });

    const { result } = renderHook(() => useSaveSidecarConfig(),
      { wrapper: wrapper(qc) });
    result.current.mutate({ name: "telegram", values: { TELEGRAM_BOT_TOKEN: "x" } });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(api.saveSidecarConfig).toHaveBeenCalledWith("telegram",
      { TELEGRAM_BOT_TOKEN: "x" });
    expect(invalidate).toHaveBeenCalled();
  });
});
```

**Step 2: Run test to verify it fails**

```
cd crates/librefang-api/dashboard && pnpm test channels.test.ts --run 2>&1 | tail -20
```
Expected: import error / not exported.

**Step 3: Implement**

Add to `crates/librefang-api/dashboard/src/api.ts`:
```ts
export interface SidecarSaveResult {
  status: "saved";
  restart_required: boolean;
  hot_actions_applied: string[];
}

export async function saveSidecarConfig(
  name: string,
  values: Record<string, string>,
): Promise<SidecarSaveResult> {
  const res = await apiFetch(`/api/channels/sidecar/${encodeURIComponent(name)}/configure`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ values }),
  });
  if (!res.ok) throw await ApiError.from(res);
  return res.json();
}
```
(`apiFetch` and `ApiError` are existing helpers in `src/lib/http/client.ts` — match the pattern used by `configureChannel` or similar elsewhere in `api.ts`.)

Add to `crates/librefang-api/dashboard/src/lib/mutations/channels.ts`:
```ts
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { saveSidecarConfig } from "../../api";
import { channelsKeys } from "../queries/keys";

export function useSaveSidecarConfig() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ name, values }: { name: string; values: Record<string, string> }) =>
      saveSidecarConfig(name, values),
    onSuccess: () => {
      // Whole channels domain may shift: discovery row vanishes, configured row appears.
      qc.invalidateQueries({ queryKey: channelsKeys.all });
    },
  });
}
```

**Step 4: Run tests + typecheck**

```
cd crates/librefang-api/dashboard
pnpm typecheck
pnpm test --run channels.test.ts
```
Expected: typecheck clean, mutation test pass.

**Step 5: Commit**

```bash
git add crates/librefang-api/dashboard/src/api.ts crates/librefang-api/dashboard/src/lib/mutations/channels.ts crates/librefang-api/dashboard/src/lib/mutations/channels.test.ts
git commit -m "feat(dashboard): useSaveSidecarConfig mutation + saveSidecarConfig api wrapper"
```

---

### Task 5.2: Add `SidecarForm` component and route picker to it

**Files:**
- Modify: `crates/librefang-api/dashboard/src/pages/ChannelsPage.tsx`:
  - Add `SidecarForm` component (renders schema-driven fields)
  - Modify `handlePick` to route `category === "sidecar"` to a new `sidecarFormChannel` state instead of `detailsChannel`
  - Add `sidecarFormChannel` state + render
- No new test file needed; the form is a thin wrapper over already-tested mutation. A smoke render-test in `ChannelsPage.test.tsx` (if it exists) is enough.

**Step 1: Sketch the form component (no test step — UI smoke is enough)**

Add inside `ChannelsPage.tsx`, near the existing `ChannelForm` definition:

```tsx
function SidecarForm({
  channel, onClose, t,
}: {
  channel: Channel;
  onClose: () => void;
  t: (key: string, opts?: { defaultValue?: string }) => string;
}) {
  const addToast = useUIStore((s) => s.addToast);
  const saveMut = useSaveSidecarConfig();
  const fields = (channel.fields ?? []).filter(f => !f.advanced);
  const advanced = (channel.fields ?? []).filter(f => f.advanced);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const visible = showAdvanced ? [...fields, ...advanced] : fields;

  const [values, setValues] = useState<Record<string, string>>(() =>
    Object.fromEntries((channel.fields ?? []).map(f => [f.key, ""])));

  const handleSubmit = () => {
    // Drop empty optional values; server validates required.
    const payload: Record<string, string> = {};
    for (const f of channel.fields ?? []) {
      if (values[f.key]?.trim()) payload[f.key] = values[f.key];
    }
    saveMut.mutate({ name: channel.name, values: payload }, {
      onSuccess: (res) => {
        addToast(
          res.restart_required
            ? t("channels.saved_restart_required",
                { defaultValue: "Saved — restart daemon to apply" })
            : t("channels.saved", { defaultValue: "Saved" }),
          "success",
        );
        onClose();
      },
      onError: (err) => addToast(toastErr(err, t("common.error")), "error"),
    });
  };

  return (
    <DrawerPanel isOpen onClose={onClose} size="lg" hideCloseButton>
      <div className="h-2 bg-linear-to-r from-brand via-brand/60 to-brand/30" />
      <div className="p-6 border-b border-border-subtle flex items-center justify-between">
        <h2 className="text-xl font-black">{channel.display_name || channel.name}</h2>
        <button onClick={onClose} className="p-2"><X className="w-5 h-5" /></button>
      </div>
      <div className="p-6 space-y-3">
        {visible.map(f => (
          <div key={f.key} className="space-y-1">
            <label className="text-xs font-bold">
              {f.label}
              {f.required && <span className="text-error">*</span>}
            </label>
            <Input
              type={f.type === "secret" ? "password" : "text"}
              value={values[f.key] ?? ""}
              placeholder={f.placeholder}
              onChange={e => setValues(v => ({ ...v, [f.key]: e.target.value }))}
            />
          </div>
        ))}
        {advanced.length > 0 && (
          <button className="text-xs text-text-dim underline"
                  onClick={() => setShowAdvanced(s => !s)}>
            {showAdvanced ? t("common.hide_advanced") : t("common.show_advanced")}
          </button>
        )}
      </div>
      <div className="p-4 border-t flex justify-end gap-2">
        <Button variant="ghost" onClick={onClose}>{t("common.cancel")}</Button>
        <Button variant="primary" onClick={handleSubmit} disabled={saveMut.isPending}>
          {saveMut.isPending ? t("common.saving") : t("common.save")}
        </Button>
      </div>
    </DrawerPanel>
  );
}
```

(Match the existing components' import patterns — `Input`, `Button`, `DrawerPanel`, `useUIStore`, `toastErr` are all defined elsewhere in the dashboard. Re-use, don't re-implement.)

**Step 2: Route picker pick to the form**

Modify `handlePick`:
```tsx
const [sidecarFormChannel, setSidecarFormChannel] = useState<Channel | null>(null);

const handlePick = (ch: Channel) => {
  setPickerOpen(false);
  if (ch.category === "sidecar") setSidecarFormChannel(ch);
  else if (ch.setup_type === "qr") setQrLoginChannel(ch);
  else setConfiguringChannel(ch);
};
```

Render the form near the other modals:
```tsx
{sidecarFormChannel && (
  <SidecarForm
    channel={sidecarFormChannel}
    onClose={() => setSidecarFormChannel(null)}
    t={t}
  />
)}
```

**Step 3: Typecheck + tests + build**

```
pnpm typecheck
pnpm test --run
pnpm build
```
Expected: clean.

**Step 4: Manual smoke (operator does this — not a hard gate)**

- Open dashboard → Channels page → click Add → pick Telegram
- Form appears with TELEGRAM_BOT_TOKEN (secret), ALLOWED_USERS (text), Show Advanced toggle
- Submit with token → toast "Saved"
- Page refreshes; Telegram now appears as configured

**Step 5: Commit**

```bash
git add crates/librefang-api/dashboard/src/pages/ChannelsPage.tsx
git commit -m "feat(dashboard): SidecarForm — schema-driven config form for sidecar channels"
```

---

## Phase 6 — Integration + finalize (~0.5 day)

### Task 6.1: Full workspace verification

**Steps (no code, just commands):**

```
cd /tmp/librefang-sidecar-configure
cargo check --workspace --lib --tests
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p librefang-api --test channels_routes_test
cargo test -p librefang-api --test sidecar_describe_test
cargo test -p librefang-api --test sidecar_toml_test
cargo test -p librefang-api --test secrets_env_test
cargo test -p librefang-kernel --lib config_reload
cd crates/librefang-api/dashboard
pnpm typecheck
pnpm test --run
pnpm build
cd ../../..
cd sdk/python && python -m pytest tests/ -v
```
Expected: every command exits 0.

If anything fails, fix at the source — do not skip tests or add `#[ignore]`.

### Task 6.2: Push + open PR

```
git push -u origin feat/sidecar-channel-configure
gh pr create --title "feat(channels): configure sidecar adapters (telegram/ntfy) from dashboard" --body "$(cat <<'EOF'
## Summary

After #5249 / #5250 the dashboard could **discover** telegram and ntfy sidecar adapters but had no save button — operators still had to hand-edit `~/.librefang/config.toml`. This PR closes that gap end-to-end:

1. **SDK self-description**: sidecar adapters now expose `python -m <module> --describe` returning a JSON schema (`name`, `display_name`, `description`, `fields[]`). telegram and ntfy ship with field declarations.
2. **Daemon schema cache**: at boot the daemon runs `--describe` for every entry in `SIDECAR_CATALOG`, caches the result, and surfaces `fields[]` on `/api/channels` discovery rows. SDK-absent dev envs fall back to empty fields (logged at WARN).
3. **Save endpoint**: `POST /api/channels/sidecar/{name}/configure` accepts `{ values: {key: value} }`, splits the payload across `~/.librefang/secrets.env` (every `type=secret` field) and `~/.librefang/config.toml` `[[sidecar_channels]]` block (every non-secret field + boilerplate), then triggers `reload_config()`.
4. **Hot-reload**: `sidecar_channels` is now diffed in `config_reload.rs` and reuses the existing `HotAction::ReloadChannels` path (`channel_adapters.clear()` → bridge re-init).
5. **Dashboard**: picker click on a sidecar row opens a new `SidecarForm` (schema-driven) instead of the read-only details modal. Save → toast → channels query invalidated → configured row appears.

Reuses two pieces of existing infrastructure:
- `crates/librefang-extensions/src/dotenv.rs` already loads `~/.librefang/secrets.env` into `std::env` at startup — sidecar children inherit it automatically; no new env-passing code.
- `HotAction::ReloadChannels` already clears `mesh.channel_adapters`; bridge cycle re-spawns sidecars from the live config.

## Tests

- `sdk/python/tests/test_describe_schema.py` — Field/Schema shape (4 tests)
- `sdk/python/tests/test_describe_main.py` — `--describe` CLI (2 tests)
- `sdk/python/tests/test_first_party_describe.py` — telegram/ntfy SCHEMA contracts (2 tests)
- `crates/librefang-api/tests/sidecar_describe_test.rs` — Rust schema-fetch helper (2 tests)
- `crates/librefang-api/tests/secrets_env_test.rs` — secrets.env upsert atomicity + 600 perms (4 tests)
- `crates/librefang-api/tests/sidecar_toml_test.rs` — config.toml block upsert (3 tests)
- `crates/librefang-api/tests/channels_routes_test.rs` — full save flow + validation (3 new tests)
- `crates/librefang-kernel/src/config_reload.rs` — sidecar_channels diff (1 new unit test)

## Verification

- `cargo check --workspace --lib --tests`: clean
- `cargo clippy --workspace --all-targets -- -D warnings`: clean
- All targeted tests pass
- `pnpm typecheck` / `pnpm test --run` / `pnpm build`: clean

## Test plan

- [ ] Fresh `config.toml` with no `[[sidecar_channels]]`: dashboard Add picker shows Telegram + ntfy as unconfigured catalog rows with form fields
- [ ] Click Telegram → form shows TELEGRAM_BOT_TOKEN (password input), ALLOWED_USERS / CLEAR_DONE_REACTION under "Show advanced"
- [ ] Submit with token → toast "Saved", drawer closes
- [ ] `~/.librefang/secrets.env` contains `TELEGRAM_BOT_TOKEN=...` with mode 600; `config.toml` `[[sidecar_channels]]` exists with `name = "telegram"`, no token
- [ ] Dashboard refreshes; Telegram now appears as configured (online)
- [ ] Telegram sidecar comes up automatically via hot-reload (no daemon restart)
- [ ] Save again with new token → secrets.env replaces in place, no duplicate lines

EOF
)"
```

---

## Risks / Watch-outs

1. **`describe_sidecar` blocking boot**: 5s timeout per adapter × N adapters. With 2 today this is 10s worst case. If catalog grows past 5, switch to parallel `tokio::join_all`.
2. **Stale schema cache**: `populate_sidecar_schema_cache` only runs at boot. If the operator `pip install -e sdk/python` *after* daemon start, dashboard still shows empty fields. Acceptable for v1; surface a "Reload schemas" button later if needed.
3. **Token leak via response**: the save endpoint returns `{ status, hot_actions_applied, restart_required }` — does NOT echo `values`. Verify with a manual `curl` post-merge.
4. **`channel_adapters.clear()` race**: if a message is mid-flight when clear fires, the bridge cycle re-init may double-spawn briefly. The existing supervisor handles duplicate spawns via the same circuit-breaker, but log volume could spike. Audit `channel_bridge.rs` (Task 4.2) catches this.
5. **`secrets.env` and `LIBREFANG_VAULT_KEY`**: `dotenv.rs` priority is `system env > vault > .env > secrets.env`. A token already set in the operator's shell env will WIN over our secrets.env write — the form will appear to "not save". Acceptable: telling the operator to unset the shell var is a one-line README addition (out of scope here).
6. **TOML comment loss**: `toml_edit` preserves comments on UNCHANGED sections but our `Table::clone()` for the replaced block discards comments inside it. Acceptable: operator-set comments in `[sidecar_channels.env]` are uncommon; document the limitation in the endpoint docstring.

---

## Execution

Plan complete and saved to `docs/plans/2026-05-19-sidecar-channel-configure.md`. Two execution options:

**1. Subagent-Driven (this session)** - dispatch fresh subagent per task, review between tasks, fast iteration.

**2. Parallel Session (separate)** - open new session with `executing-plans` skill, batch execution with checkpoints.

Which approach?
