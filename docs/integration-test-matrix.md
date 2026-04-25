# LibreFang Integration Test Matrix

This matrix defines what "100% integration coverage" means for LibreFang: every
public runtime surface is either covered by an executable scenario or explicitly
exempted in `crates/librefang-api/tests/fixtures/integration_matrix.json`.

Line coverage alone is not sufficient. Required coverage means the test proves
the HTTP/API layer, kernel behavior, persistence side effects, and business rule
contracts that users rely on at runtime.

## Coverage Policy

- Every OpenAPI path must be listed in the machine-readable matrix as `covered`
  or `exempt`.
- Every `covered` row must reference a scenario in this document.
- Every `exempt` row must include a reason and owner. Exemptions are temporary
  and should shrink over time.
- Critical agent, skill, workflow, memory, storage, auth, audit, and rate-limit
  paths must have executable integration coverage, not unit-only coverage.
- CI should fail on contract drift: a new OpenAPI path without a matrix entry, a
  covered scenario without a valid status, or an exemption without a reason.

## Scenario Matrix

| Scenario | Runtime surface | Required proof | Current status |
| --- | --- | --- | --- |
| `agent_lifecycle_http` | `POST /api/agents`, `GET /api/agents`, `GET/PATCH/DELETE /api/agents/{id}`, `POST /api/agents/{id}/message`, `GET /api/agents/{id}/session` | Spawn an agent from TOML, read it back, update mutable fields, send a deterministic message, verify session side effects, then delete it. | Covered by API integration tests and expanded lifecycle tests. |
| `agent_observability_http` | `GET /api/agents/{id}/metrics`, `GET /api/agents/{id}/logs`, traces and deliveries where stable | Mutating agent actions must be observable through metrics/log endpoints and preserve stable response shapes. | Partially covered; detailed trace and delivery assertions are follow-up. |
| `agent_clone_http` | `POST /api/agents/{id}/clone` | Clone an existing agent, verify the clone gets a distinct ID, inherits selected configuration, and rejects invalid clone names. | Covered by focused integration tests. |
| `agent_upload_http` | `POST /api/agents/{id}/upload`, `GET /api/uploads/{file_id}` | Upload a small attachment through the route that bypasses the global body limit, verify metadata and retrieval. | Exempt until multipart fixture support is complete. |
| `skill_lifecycle_http` | `GET /api/skills`, `GET /api/skills/registry`, `POST /api/skills/install`, `POST /api/skills/reload`, `POST /api/skills/uninstall`, `POST /api/skills/create` | Seed a local registry skill, install it, reload the registry, list it, reject invalid input, uninstall it, and verify it no longer appears. | Covered by fixture-driven integration tests. |
| `skill_assignment_http` | `GET/PUT /api/agents/{id}/skills` | Installed skills must be assignable to an agent allowlist; invalid skill names must be rejected by the kernel. | Covered by fixture-driven integration tests. |
| `workflow_lifecycle_http` | `POST /api/workflows`, `GET /api/workflows`, `POST /api/workflows/{id}/dry-run`, `POST /api/workflows/{id}/run`, `GET /api/workflows/{id}/runs` | Create a workflow over a deterministic agent, dry-run it, execute it, assert completed status/output/step results, and read the run history. | Covered by expanded workflow integration tests. |
| `memory_vector_http` | `GET /api/memory/search`, memory add/search/stats/config routes, agent-scoped memory routes | Add or seed memory, search with deterministic query terms, verify stats, and run feature-gated Surreal vector recall tests. | Partially covered; Surreal/vector tests run in a separate feature-gated lane. |
| `storage_migration_http` | `/api/migrate*`, `/api/storage/*`, kernel boot migrations | Boot with temp storage, verify operational migrations are idempotent, and ensure embedded Surreal operational and memory stores do not contend for locks. | Feature-gated integration lane. |
| `audit_integrity_http` | `GET /api/audit/recent`, `GET /api/audit/verify` | Mutating API calls must create auditable entries, and the audit chain must verify cleanly. | Covered by focused integration tests. |
| `config_contract_http` | `/api/config*` | Read config, apply safe changes, reload, and validate schema/error response shapes. | Covered by existing API tests; matrix gate prevents route drift. |
| `budget_runtime_http` | `/api/budget*` | Deterministic LLM calls should update usage/budget views when a real or mock driver is enabled. | Live daemon smoke path. |
| `daemon_smoke` | `/api/health`, `/api/status`, `/api/network/status`, `cargo xtask integration-test` | Start a daemon, hit health/status/basic runtime endpoints, optionally send an LLM message, and cleanly shut down. | Existing xtask smoke, expanded in CI plan. |
| `a2a_contract_http` | root A2A and `/api/a2a/*` | Verify well-known metadata, task send/status contract shapes, and invalid-peer error handling. | Covered by existing A2A tests with future workflow depth. |
| `mcp_contract_http` | `/mcp`, `/api/mcp/*` | Verify JSON-RPC envelope handling, auth-needed state, reload, and server list contracts. | Covered by smoke and OAuth integration tests; deeper catalog coverage deferred. |
| `openai_compat_contract_http` | `/v1/models`, `/v1/chat/completions` | Verify OpenAI-compatible response envelopes, auth behavior, and invalid model errors. | Contract matrix only; full execution deferred to provider fixture work. |

## Measurement Gates

- `cargo nextest run --workspace --no-fail-fast` remains the fast required Rust
  integration gate.
- `cargo xtask coverage --lcov` or `cargo llvm-cov nextest` should publish line
  and branch coverage. Critical crates should have stricter thresholds than the
  rest of the workspace.
- `cargo-mutants --test-tool=nextest` should run on critical crates in a
  scheduled or label-triggered lane. Survived mutants in agent, skill, workflow,
  audit, memory, or storage business rules block release until triaged.
- Dashboard Playwright should run for `crates/librefang-api/dashboard` changes.
- Feature-gated Surreal/vector tests should run on Linux with isolated temp
  directories and no shared embedded RocksDB paths.
