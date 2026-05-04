# Security Policy

## Supported Versions

| Version | Supported          |
|---------|--------------------|
| `main`  | :white_check_mark: |
| Latest LibreFang release | :white_check_mark: |

## Reporting a Vulnerability

If you discover a security vulnerability in LibreFang, please report it privately.

**Do NOT open a public GitHub issue for security vulnerabilities.**

### How to Report

1. Use GitHub's private vulnerability reporting flow:
   `https://github.com/librefang/librefang/security/advisories/new`
2. Include:
   - Description of the vulnerability
   - Steps to reproduce
   - Affected versions
   - Potential impact assessment
   - Suggested fix (if any)

### What to Expect

- **Acknowledgment** within 48 hours
- **Initial assessment** within 7 days
- **Fix timeline** communicated within 14 days
- **Credit** given in the advisory (unless you prefer anonymity)

### Scope

The following are in scope for security reports:

- Authentication/authorization bypass
- Remote code execution
- Path traversal / directory traversal
- Server-Side Request Forgery (SSRF)
- Privilege escalation between agents or users
- Information disclosure (API keys, secrets, internal state)
- Denial of service via resource exhaustion
- Supply chain attacks via skill ecosystem
- WASM sandbox escapes

## Security Architecture

LibreFang implements defense-in-depth with the following security controls:

### Access Control
- **Capability-based permissions**: Agents only access resources explicitly granted
- **RBAC multi-user**: Owner/Admin/User/Viewer role hierarchy
- **Privilege escalation prevention**: Child agents cannot exceed parent capabilities
- **API authentication**: Bearer token with loopback bypass for local CLI

### Input Validation
- **Path traversal protection**: `safe_resolve_path()` / `safe_resolve_parent()` on all file operations
- **SSRF protection**: Private IP blocking, DNS resolution checks, cloud metadata endpoint filtering
- **Image upload validation**: exact-match MIME allowlist on
  `/api/agents/{id}/upload` covers `image/png`, `image/jpeg`, `image/gif`,
  `image/webp`; scriptable formats like `image/svg+xml` are rejected.
  Upload size is capped by `KernelConfig.max_upload_size_bytes` (default
  10 MiB — tighten it in `config.toml` if your threat model demands a
  smaller limit).
- **Prompt injection heuristics** *(best-effort, not a security boundary)*: Skill content is
  scanned for a short hard-coded list of English override phrases and exfiltration keywords
  (`ignore previous instructions`, `exfiltrate`, `post to https`, …) via case-insensitive
  substring match. Matches emit warnings and block installation of ClawHub skills whose
  `prompt_context` contains a *critical* pattern. This is a warning layer for obviously
  malicious content, **not** a defence against a motivated attacker: Unicode homoglyphs,
  zero-width separators, line-split keywords, Base64/other encodings, markdown/link
  obfuscation, and non-English phrasing all bypass it. The actual runtime safety of
  installed skills comes from the capability system and the WASM / subprocess sandbox
  (see **Runtime Isolation**), which bound what a skill can do regardless of what its
  prompt text says.

### Cryptographic Security
- **Ed25519 signed manifests**: Agent identity verification
- **HMAC-SHA256 wire protocol**: Mutual authentication with nonce-based replay protection
- **Secret zeroization** *(scoped to the credential vault)*: the encrypted
  credential vault in `librefang-extensions/src/vault.rs` stores every
  entry as `Zeroizing<String>`, so individual vault reads drop plaintext
  immediately after use and the vault master key is also held in
  `Zeroizing<[u8; 32]>`. The embedding driver re-wraps its API key with
  `Zeroizing::new` after reading it from config. Other secret-carrying
  fields on `KernelConfig` (`api_key`, `dashboard_pass`,
  `dashboard_pass_hash`) are still plain `String`: adding a destructor to
  `KernelConfig` breaks partial-move patterns across ~700 call sites,
  and switching every field to `Zeroizing<String>` requires a
  serde-compatible newtype rollout that is not yet in place. The
  in-process copy of these fields therefore persists in heap memory
  until the owning `Arc<KernelConfig>` is dropped, which is
  good-enough against post-exit forensics on most platforms but is
  **not** the "wiped on every drop" guarantee the previous bullet
  implied. If you need stronger memory hygiene, run the daemon inside
  a memory-encrypted VM or disable core dumps for the process.

### Runtime Isolation
- **WASM dual metering**: Fuel limits + epoch interruption with watchdog thread
- **Subprocess sandbox**: Environment isolation (`env_clear()`), restricted PATH
- **Tool-sink heuristics** *(pattern match, not full information-flow tracking)*:
  `crates/librefang-types/src/taint.rs` defines `TaintLabel`, `TaintedValue`,
  and `TaintSink`, and the LLM tool runner checks two sinks before
  executing risky tool calls:
  `check_taint_shell_exec` refuses commands matching `curl `, `wget `,
  `| sh`, `| bash`, `base64 -d`, `eval ` (plus the shell-metacharacter
  denylist), and `check_taint_net_fetch` refuses URLs whose query string
  or percent-decoded parameter names contain `api_key`, `apikey`,
  `token`, `secret`, `password`, or an `authorization:` header fragment.
  Values that hit a pattern are wrapped in `TaintedValue` and run
  through `check_sink`, which is where the refusal originates.

  This is **not** a general information-flow control system: labels are
  attached at the call site the moment a pattern matches, they do not
  propagate across function boundaries, LLM tool outputs are not
  automatically labelled, and there is no compiler/type-level
  enforcement — code that never constructs a `TaintedValue` is
  entirely outside the check. Treat it as a targeted denylist for two
  specific exfiltration / injection shapes in the tool runner, not as
  a lattice that covers all untrusted data in the process.

### Network Security
- **GCRA rate limiter**: Cost-aware token buckets per IP
- **Security headers**: CSP, X-Frame-Options, X-Content-Type-Options, HSTS
- **Health redaction**: Public endpoint returns minimal info; full diagnostics require auth
- **CORS policy**: Restricted to localhost when no API key configured

### Audit
- **Hash-linked audit log**: Each entry's hash covers its fields plus the previous entry's hash, and `/api/audit/verify` recomputes the chain from the genesis sentinel. This detects **in-place edits** (flip a byte in one entry and the chain breaks at that row) and **row deletions** (the successor's `prev_hash` no longer matches).
- **External tip anchor (Tier 1)**: every audit append also writes the new tip hash to `~/.librefang/data/audit.anchor` outside the SQLite database (see `AuditLog::with_db_anchored` in `crates/librefang-runtime/src/audit.rs`). On startup and on every `/api/audit/verify` call, the in-DB tip is reconciled against the anchor file; if they diverge, verification **fails closed** (`valid: false`, `anchor_status: "diverged"`). An attacker rewriting the SQLite chain from genesis must now also forge the anchor file in lockstep, defeating the trivial DB-only forgery. The verify response surfaces `anchor_status` as `ok`, `diverged`, or `none` so the dashboard can show the anchor state alongside the chain check.
- **Threat model — what the anchor does and does not buy**: the anchor file lives next to the database by default. An attacker with full write access to `~/.librefang/data/` can still corrupt both files in lockstep — the anchor is meaningfully stronger only when operators sync `audit.anchor` to an append-only store they control (offsite cron rsync, signed systemd-journald mirror, transparency log). Tier-2 (journald mirror) and Tier-3 (Ed25519-signed offsite mirror, transparency log) are tracked as follow-up work in #3339. Until then, treat the audit log as tamper-evident against single-file tampering and cooperative against multi-file tampering by an attacker with full filesystem write.

## Supply-chain audit (CI gate)

PRs that touch `crates/librefang-skills/`, `crates/librefang-hands/`,
`crates/librefang-extensions/`, or `examples/` are gated by the
`supply-chain-audit` workflow
(`.github/workflows/supply-chain-audit.yml`), which runs the static
checker `scripts/check-skills-supply-chain.py`. The checker enforces:

- **No `.pth` files** anywhere in skill / hand / extension bundles.
  Python's `site-packages` loader auto-executes `.pth` content at
  interpreter start, so a single shipped `.pth` is a full-process RCE.
- **No `eval` / `exec` / `compile(..., 'exec')`** in embedded `.py`
  files (AST-grep, not regex), and a dedicated rule for
  `eval(base64.b64decode(...).decode())` shapes.
- **No `sys.path` mutation** or `importlib.util.spec_from_file_location`
  in shipped Python — both let a skill load attacker-controlled code
  outside the bundle.
- **No `eval` / `Function(...)` / `setTimeout(<string>, ...)`** in
  embedded `.js`.
- **Curated jailbreak phrase regex** on `.toml` / `.md` / `.prompt`
  prompt-bearing content: `ignore previous instructions`, `exfiltrate`,
  `post … to webhook`, `bypass safety`, `override system prompt`,
  `disregard … rules`, `reveal/leak system prompt`. This is stricter
  than the runtime warning layer in
  `crates/librefang-skills/src/verify.rs` because PR review is the
  right time to bounce content, not install time. False positives can
  opt out per-file with the literal marker `supply-chain-audit: allow`
  (every use is reviewable in `git diff`).

The workflow runs a `--self-test` job first, replaying embedded
clean + malicious fixtures through every rule and asserting each is
caught with the expected rule name. If a rule is weakened or removed
without updating the fixtures, the self-test fails and the audit job
is gated behind it. The companion checked-in fixtures live at
`crates/librefang-skills/tests/fixtures/supply-chain/` and are
documented in the README there.

The script is pure-stdlib Python 3.10+ (no third-party imports), so
it adds no new dependency surface to the security tooling itself.

Cargo-side dependency advisories (`cargo-deny` / `cargo-audit`) are
tracked separately under #3305. A runtime install-time guard that
re-runs these checks at marketplace install time is the next stage of
#3333 and is tracked there.

## Dependencies

Security-critical dependencies are pinned and audited:

| Dependency | Purpose |
|------------|---------|
| `ed25519-dalek` | Manifest signing |
| `sha2` | Hash chain, checksums |
| `hmac` | Wire protocol authentication |
| `subtle` | Constant-time comparison |
| `zeroize` | Secret memory wiping |
| `rand` | Cryptographic randomness |
| `governor` | Rate limiting |
