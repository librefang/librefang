# API conventions

LibreFang exposes a typed HTTP surface (`librefang-api`) with utoipa-generated
OpenAPI and four downstream SDKs. The conventions below keep that surface
unambiguous for clients and let typed-language SDKs exhaustively handle every
shape.

Refs: [#3302](https://github.com/anthropics/librefang/issues/3302).

## Discriminated unions and sentinel values

This section defines the wire-shape contract for sum types and absent-value
fields. Adding new endpoints? Pick from the **good patterns** below. Touching
existing code? The lint script `scripts/check-no-empty-string-sentinels.sh`
flags the most common offenders.

### Rule 1 — Discriminated unions use an explicit `type` tag

For any enum that crosses the API boundary as JSON, the wire shape MUST carry
an explicit discriminator field. Prefer the serde-internal-tag or
adjacently-tagged form so utoipa can emit the discriminator as `required` and
enumerate valid values:

```rust
// GOOD — internal tag, snake_case variant names match the wire literal.
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CronDeliveryTarget {
    Channel { channel_type: String, recipient: String },
    Webhook { url: String },
    LocalFile { path: String, append: bool },
}
// Wire shape:  {"type": "channel", "channel_type": "slack", "recipient": "..."}
//              {"type": "webhook", "url": "..."}
```

```rust
// GOOD — adjacently tagged when the variant payload is a single value.
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", content = "value")]
pub enum EventTarget {
    Agent(AgentId),
    Broadcast,
    Pattern(String),
    System,
}
// Wire shape:  {"type": "agent", "value": "agt_..."}
//              {"type": "broadcast"}
```

The following form is **forbidden** for new types and discouraged for
existing ones:

```rust
// BAD — `untagged` defers discrimination to structural matching, which
// every SDK has to re-implement and which utoipa cannot describe as a
// discriminated schema. The OpenAPI output ends up as a free `oneOf` with
// no required discriminator, and the typed SDK loses exhaustiveness.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub enum SkillSource {
    Native,
    ClawHub { slug: String, version: String },
    // …
}
```

`#[serde(untagged)]` is acceptable **only** when the wire shape is dictated
by an external contract that legitimately accepts both a scalar and an
object form (e.g. OpenAI's `content: string | array`, Anthropic's
`content: string | block[]`). Mark such cases with a `// EXTERNAL CONTRACT —`
comment naming the upstream spec so future readers know the constraint is
not local.

### Rule 2 — Don't conflate "unset" with "empty string"

`String::is_empty()` is a runtime check — it cannot tell apart "the user
provided an empty value" from "the value was never set". Conflating them
forces every client to special-case the empty string and breaks round-trip
serialization.

```rust
// BAD — readers can't tell "not configured" from "explicitly cleared".
#[derive(Serialize)]
struct ProviderConfig {
    pub api_key_env: String,  // sentinel: "" means unset
}

// In a handler:
if cfg.api_key_env.is_empty() {
    // unset path
} else {
    // set path
}
```

```rust
// GOOD — Option<T> is the type-level encoding of "may be unset".
#[derive(Serialize)]
struct ProviderConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
}

// In a handler:
match cfg.api_key_env.as_deref() {
    Some(env) => { /* set path */ }
    None => { /* unset path */ }
}
```

Likewise, the literals `"<unknown>"`, `"<empty>"`, and `"none"` are not
acceptable as sentinel values in JSON responses. If you need to return
"information not available", use `null` (`Option<T>` with `None`) or a
typed enum variant.

### Rule 3 — Use `#[serde(skip_serializing_if = ...)]` for genuinely
optional output

For response shapes where omitting a field is preferable to emitting `null`
(common when a downstream consumer treats `null` and missing differently),
combine `Option<T>` with `skip_serializing_if`:

```rust
#[derive(Serialize)]
struct AgentSummary {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_active_at: Option<DateTime<Utc>>,
}
```

This keeps the field absent when unset, present when set, and never collapses
both states onto a single string.

## Lint enforcement

`scripts/check-no-empty-string-sentinels.sh` greps the routes and channel
adapters for the patterns above and reports candidates. It runs in **warn
mode** in the `Quality` CI job — it surfaces hits without failing the
build. Pass `--strict` to fail on any hit; once the existing inventory is
cleared (tracked under #3302) CI will flip to strict.

To suppress a verified-benign hit (e.g. an `is_empty()` check that's a
length validation, not a sentinel), append the marker `// allow-empty-sentinel: <reason>`
on the same line. The reviewer evaluates each suppression.

## Migration policy for existing types

The 1/N PR landing this convention adds the lint and the docs only. Existing
`#[serde(untagged)]` enums that are *not* governed by an external contract
(e.g. internal sum types whose JSON shape happens to be unambiguous by
construction) will migrate variant-by-variant in follow-up PRs under #3302.
Each migration must verify wire compatibility — ideally with a round-trip
test asserting `serde_json::to_string(&v)` matches the historical shape
byte-for-byte — before the variant flips to `#[serde(tag = "type")]`.
