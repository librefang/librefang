# Trajectory format

Wire format for the dataset export pipeline landing under #3330
(`librefang datagen run`) and consumed by the downstream W&B / Tinker /
Atropos integrations queued in #3331. This RFC locks the four
decisions a recorder and a replayer must agree on byte-for-byte so the
batch-runner PR and the integration PRs can be cut in parallel without
a v1 → v2 migration six weeks in.

Status: draft. Lock pending review and a smoke export against a real
session.

## TL;DR

- Container: **msgpack-NDJSON** (one `rmp-serde`-encoded record per
  line, framed by a 4-byte big-endian length prefix). Parquet is the
  eventual end-state; v1 is not it.
- Compression: **zstd**, level 3, framed per-file. Uncompressed `.msgpack`
  is a supported debug variant.
- Schema versioning: per-record `format_version: u16` at field position 0,
  additive-only inside v1. Range 1..=99 reserved for v1; bump to 100 on
  any rename or removal.
- Tool-call encoding: **pass through the upstream provider's wire
  shape** tagged with `provider`. No normalization at record time;
  normalize in the replay tools.
- Sidecar `toolset.toml` enumerates tools available at recording time
  with version + capability hash. Not embedded in the stream.

## Why we need a locked format

The batch runner (#3330) writes. The W&B / Tinker / Atropos
integrations (#3331) read. Those PRs land separately. If the wire
shape drifts between writer and reader by a single field — a renamed
key, a moved `provider` tag, a `u32` that became a `u64` — the symptom
is silent data corruption: replays succeed, RL scores converge, and a
month later you find out half the tool-result payloads were
truncated because `rmp-serde` parsed the new field as a `bin` instead
of a `str`. No checksum catches semantic drift.

The other forcing function is reproducibility. An RL eval that says
"replay this trajectory against the new checkpoint and tell me the
reward delta" only works if the original trajectory is still parseable
by both the recorder that wrote it and the replayer running today. We
do not get to pretend we will never bump fields. We get to pretend
we know which bumps are safe. This RFC defines what counts as safe.

DECISION: lock the format before either #3330's runner or #3331's
exporters merges. No "we'll figure it out in v2".

## Container format

| Container       | Query | Debug ergonomics | Streaming append | Random access | Deps to add                |
|-----------------|-------|------------------|------------------|---------------|----------------------------|
| JSONL           | grep  | excellent        | trivial          | none          | none (`serde_json` in tree)|
| msgpack-NDJSON  | weak  | needs a CLI tool | trivial          | none          | none (`rmp-serde` in tree) |
| Parquet         | SQL   | poor             | poor (row groups)| native        | `arrow` + `parquet` (large)|
| SQLite blob     | SQL   | medium           | medium (WAL)     | native        | `rusqlite` (already in tree)|

JSONL loses on payload size: a 200-turn trajectory with image blobs
inflates 3-5x over msgpack because every base64 tool-result
round-trips through string-escape. Parquet wins on query, loses on
streaming — the writer has to buffer a row group before flushing,
which fights the crash-checkpoint model the batch runner needs.
SQLite is fine for single-machine but a poor fit for the rsync / S3
distribution path implied by the issue's acceptance criteria.

msgpack-NDJSON sits in the middle. The append model is one
`rmp_serde::encode::to_vec_named(&record)?` plus a length prefix and
a write — exactly what a checkpointed batch runner wants. Inspection
requires `msgpack-tools` (`msgpack2json`), one `cargo install` away.
`rmp-serde` is already in workspace `Cargo.toml` (line 50, version
`1`), so the recorder adds zero new direct deps.

Wire framing: `[u32 BE length][msgpack record bytes]` repeated.
Length-prefix instead of newline-delimited because msgpack contains
arbitrary bytes; chasing a `\n` sentinel across a binary stream
generates `InvalidUtf8` panics. The length prefix also lets a
corrupted record be skipped without losing the rest of the file.

The Parquet migration path: when the dataset volume crosses a
threshold where SQL-style queries become operational, we add a
`librefang datagen convert --to parquet` pass that reads the
msgpack-NDJSON and emits Parquet. The trajectory shape does not
change. The container does. The `format_version` field carries
across unchanged.

DECISION: msgpack-NDJSON with `[u32 BE length][bytes]` framing. No
Parquet in v1. No SQLite. No JSONL.

## Compression

| Codec | Ratio (rough) | Decode speed | Encode speed | Crate         |
|-------|---------------|--------------|--------------|---------------|
| none  | 1.0x          | n/a          | n/a          | n/a           |
| lz4   | 2.0-2.5x      | very fast    | very fast    | `lz4_flex`    |
| zstd  | 4.0-6.0x      | fast         | medium       | `zstd`        |

Trajectory streams compress extremely well. The dominant content is
LLM prompt scaffolding (skill manifests, tool definitions, system
prompts) that repeats across every turn of every run — zstd's home
territory. A typical SWE-bench-style export of 500 runs ranges from
2-4 GB raw and lands at 400-800 MB under zstd level 3, the difference
between "fits on a USB stick" and "doesn't". Decode is parser-bound,
not codec-bound: zstd-3 decodes faster than `rmp-serde` parses.

lz4 would shave ~30% off decode latency but cost ~2x in storage. We
have no low-latency replay constraint that justifies that trade. If
one shows up — an in-loop RL trainer streaming trajectories live —
lz4 can be added as a per-file choice signalled by file extension
(`.msgpack.lz4`); framing unchanged.

`zstd` is not currently a direct workspace dependency — it appears
in `Cargo.lock` as a transitive dep of `wasmtime-internal-cache`
(verified against `Cargo.lock` at HEAD). Adding it as a direct dep
in `librefang-datagen`'s `Cargo.toml` is therefore a one-line commit;
its `zstd-sys` and `zstd-safe` are already compiled into every
LibreFang build, so the binary-size delta is negligible.

Uncompressed variant: `<run_id>/trajectory.msgpack` (no `.zst`
suffix). Supported for local debugging — `msgpack2json < file` works
without a decompression step. The replayer dispatches on the
extension; recorder default is compressed.

DECISION: zstd level 3, framed per-file (`.msgpack.zst`).
Uncompressed `.msgpack` accepted for debug. No lz4 in v1.

## Schema versioning

Per-record `format_version: u16` at field position 0 in the msgpack
struct. NOT a per-file header. Real datasets get concatenated across
runs recorded by different recorder versions; a per-file header lies
the moment somebody runs `cat a.msgpack b.msgpack > c.msgpack`.
Per-record lets concatenation just work, with the replayer
dispatching per record.

Position 0 because msgpack-named-struct encoding is order-stable: a
parser reading `format_version` first can dispatch to the right
deserializer without parsing the rest. A v1 replayer seeing
`format_version = 50` (in-range, unknown to it) can warn and skip;
seeing `100` (the v2 sentinel) it errors out.

Range reservation:

| Range     | Meaning                                       |
|-----------|-----------------------------------------------|
| 0         | reserved, never emitted                       |
| 1..=99    | v1 schema, additive-only changes              |
| 100..=199 | v2 schema (reserved; not yet defined)         |
| 200..=    | reserved                                      |

The upgrade rule inside v1: adding a new field is allowed and bumps
the recorder's emitted `format_version` by 1 (so v1.4 reader can
detect "this record uses a field I don't know about" and pick its
policy: ignore unknown, or warn). Renames are not allowed. Removals
are not allowed. Type changes are not allowed. Any of those bumps
the major and lands at `format_version = 100`, at which point a v1
replayer refuses to parse and the migration tool runs.

`rmp-serde` deserializes named structs with unknown fields gracefully
when the reader struct uses `#[serde(default)]` on additions, which
is the contract every v1.N → v1.M (N < M) upgrade pays. That cost is
two lines per added field in the reader-side struct. Worth it.

DECISION: per-record `format_version: u16` at struct position 0,
1..=99 for v1, additive-only inside the range, 100 for v2.

## Tool-call payload encoding

The tempting move is to normalize at recording time: pick one shape,
translate every provider's calls into it, emit only that. We are not
going to do that.

Three reasons. The upstream wire shape is the only thing we can audit
later — an Anthropic `tool_use` block normalized to OpenAI's
`function` shape loses provider-defined metadata in `arguments`
(which is a JSON-encoded string in one and a structured object in the
other), and round-tripping cannot recover it. Normalization is a
moving target — Atropos and Tinker have their own opinions about the
canonical form, and we do not want to ship one that either rejects.
The cost of skipping normalization is a provider-aware unpacker in
the replay tools, which is ~50 lines per provider; the cost of
normalizing wrong is a re-export of every dataset we ever shipped.

Concrete shape:

```
record_kind = "tool_call"
provider    = "anthropic"  (or "openai", "gemini", "tinker", ...)
payload     = <provider's exact wire shape, msgpack-encoded as the
               provider would have shipped it as JSON>
```

The `provider` tag is required for `tool_call` and `tool_result`
records. It is optional for `assistant_message` / `user_message`
because those carry text that is already provider-invariant.

If a replayer hits a `provider` it doesn't have an unpacker for, it
falls back to the raw payload — the trajectory is still streamable
and trainable; only tool-aware analyses break. That degradation is
correct: we want unknown providers to be inspectable, not to abort
the whole load.

DECISION: pass through upstream wire shape verbatim, tagged with
`provider`. Normalization belongs in the consumer.

## Record shape

Pseudo-schema, expressed as the Rust-side struct the recorder writes
and the replayer reads:

```
TrajectoryRecord {
    format_version: u16,           // 1..=99 in v1
    record_kind:    RecordKind,    // enum below
    turn_idx:       u32,           // monotonic per run, starts at 0
    timestamp_ns:   u64,           // nanos since UNIX epoch
    provider:       Option<String>, // required for tool_call/tool_result
    payload:        Payload,        // kind-specific
}

RecordKind = "system_prompt"
           | "user_message"
           | "assistant_message"
           | "tool_call"
           | "tool_result"
           | "metadata"
```

`turn_idx` is monotonic per run, not per session — a single run can
fork (auto-dream, planning) and the fork's records share the parent's
`turn_idx` floor; the fork's records are tagged in the `metadata`
record kind, not by re-numbering. `timestamp_ns` is the recorder's
wall clock at emit time, not the LLM provider's response time;
clock skew is the replayer's problem to handle (it should sort by
`(run_id, turn_idx)` not by timestamp).

`payload` is one of:

- `system_prompt`: `{ text: String, hash: [u8; 32] }` — `hash` is
  blake3 of `text` so the replayer can dedupe across runs without
  comparing full prompt bodies.
- `user_message` / `assistant_message`: `{ content: ContentBlock[] }`
  where `ContentBlock` is `{ kind: "text"|"image", body: bytes|string }`.
- `tool_call`: `{ raw: msgpack-of-provider-payload }` — see above.
- `tool_result`: `{ tool_call_id: String, raw: msgpack-of-provider-payload }`.
- `metadata`: `{ key: String, value: msgpack-any }` — open-ended
  bucket for run-level annotations the runner wants to attach
  (fork parent id, reward, replay seed, anything that doesn't fit
  the above five kinds without inventing a sixth).

Image bytes are inline in `body` as msgpack `bin`. A
trajectory with screenshots compresses well under zstd despite
the binary payloads — the surrounding scaffolding text dominates.

DECISION: the schema above is v1. Field additions inside a record
kind are an in-range bump (1 → 2 → …). New record kinds are an
in-range bump. Anything else is v2.

## Toolset distribution metadata

The toolset shipped to the agent at recording time is part of the
training signal — a trajectory where `web_search` was available and
the agent didn't use it is a different teaching example from one
where `web_search` was not available. Without recording the toolset,
the replayer cannot tell those two apart.

Toolset metadata lives in a sidecar `toolset.toml` next to the
stream, not embedded in records. Two reasons. First, the toolset is
fixed for the duration of a run — embedding it in every record would
inflate the stream by ~30% (tool definitions are large) for zero new
information. Second, sidecar means a replay tool can pre-validate
("does my replayer have unpackers for every provider used in this
run?") before opening the stream at all. Embedded metadata requires
a full scan.

Sidecar shape:

```toml
# toolset.toml
run_id = "20260514-153022-a3f1"
recorded_at = "2026-05-14T15:30:22Z"
recorder_version = "0.41.0"

[[tools]]
name = "bash_exec"
version = "1.2.0"
capability_hash = "blake3:7b3f...e21"
provider = "librefang.runtime"

[[tools]]
name = "web_search"
version = "0.9.1"
capability_hash = "blake3:9c41...0a3"
provider = "anthropic.builtin"
```

`capability_hash` is a blake3 over the tool's input-schema JSON. A
replayer comparing two runs of the same tool name with different
hashes knows the tool changed semantics between them; that's the
signal RL eval needs to refuse a stale replay.

DECISION: sidecar `toolset.toml`, not embedded in the record stream.

## Directory layout

A single exported run lives at:

```
<export_root>/<run_id>/
    trajectory.msgpack.zst    # the record stream
    toolset.toml              # the sidecar (above)
    metadata.json             # human-readable run summary (input
                              # prompt, exit reason, final reward,
                              # checkpoint cursor for resume)
```

`<run_id>` is the recorder-assigned id (`<YYYYMMDD-HHMMSS-<short-hash>>`).
`metadata.json` is JSON, not msgpack or TOML, because it's the file
operators eyeball with `cat` when triaging — it never grows past a
few KB and never needs to be parsed by a hot path.

Multiple runs concatenate by adding sibling directories:

```
<export_root>/
    20260514-153022-a3f1/
        trajectory.msgpack.zst
        toolset.toml
        metadata.json
    20260514-154110-b8d2/
        trajectory.msgpack.zst
        toolset.toml
        metadata.json
    ...
```

This layout is the rsync-friendly shape the batch runner's
checkpoint store wants — incomplete runs can be detected by the
absence of `metadata.json` (which the runner writes last, after
`trajectory.msgpack.zst` is closed), and a resumed run rewrites its
own directory atomically via a `.tmp` suffix and `fs::rename`,
mirroring the skill-workshop atomic-write pattern in
`docs/architecture/skill-workshop.md`.

DECISION: `<export_root>/<run_id>/{trajectory.msgpack.zst, toolset.toml,
metadata.json}`. Run completion is signalled by the presence of
`metadata.json`.

## Open questions

These are NOT decided by this RFC. Each will be picked up in the
implementation PR or in a follow-up RFC, with the trade-off written
into the same docs/architecture/ tree.

- **PII redaction policy.** Trajectories captured against user
  sessions may contain secrets the user typed into the agent. The
  recorder needs an opt-in redaction pass before flush. This RFC
  does not pin the redaction schema; #3330's implementation PR will
  propose one.
- **Cross-record dedup of system prompts.** The `system_prompt`
  hash field anticipates dedup. The replayer-side store (a hash →
  body map) is not specified here. Likely a sibling
  `system_prompts/<hash>.txt` next to the run directories, but
  deferred to the integration PR that needs it.
- **Streaming partial-record recovery.** A crash mid-write leaves a
  truncated final record. The replayer currently aborts on the
  truncation. Should it skip-and-continue? The trade-off is
  silent-data-loss vs operator-visible-error; the conservative
  default is "error". Revisit when we have a crash log.
- **Reward annotation timing.** RL pipelines attach reward at end-of-
  run; the `metadata` record kind is the right home, but whether
  reward writes happen inside the runner or in a post-processing
  pass is a runner-design question, not a format question.
- **Image-blob externalization threshold.** Inline `bin` is fine up
  to a few MB per record. Beyond that we want a sidecar
  `blobs/<hash>.png` and a record reference. Threshold is not
  picked yet; depends on real-dataset profiling.
- **Schema diff tooling.** A `librefang datagen schema-diff <old> <new>`
  command that classifies a proposed change as "additive (in-range
  bump)" vs "breaking (v2)" would catch most accidental schema breaks
  in CI. Worth building, not in v1's critical path.

Status: draft. Lock pending review and a smoke export against a real
session.
