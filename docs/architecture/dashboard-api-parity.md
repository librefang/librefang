# Dashboard / API parity inventory

This is an inventory, not a contract: it lists which HTTP route groups the
dashboard source references and which it does not, so the UI work can be
prioritised instead of guessed. It was requested by the UI-architecture block of
the backlog (2.4).

## How it is produced

```bash
node scripts/dashboard-api-parity.mjs          # markdown table
node scripts/dashboard-api-parity.mjs --gaps   # group<TAB>path, one uncovered route per line
```

The script reads `openapi.json` and searches `crates/librefang-api/dashboard/src`
for the longest static prefix of each path (`/api/agents/${id}` counts as
`/api/agents`). Two limits follow from that method:

- **Covered means referenced, not rendered.** A group can be referenced by
  `api.ts` and still have no screen; the table answers "is there code that talks
  to it", not "can the operator use it".
- **A whole-group gap is reliable; a per-route gap inside a covered group is a
  lead.** Parameterised URLs built in pieces can hide a real caller, and the
  reverse also happens: `/api/logs` is "referenced" only by a comment in
  `LogsPage.tsx`, while the page actually reads `/api/audit/recent`.

Operations that are deliberately not dashboard surfaces (browser OAuth
callbacks, agent-facing authz checks, A2A transport, migration and provisioning
endpoints) are listed in the gap table but are not UI work. The backlog's
"webhooks, 9 routes" entry does not match this tree: there are no
`/api/webhooks` routes. The OpenAPI `webhooks` tag covers `/api/hooks/agent` and
`/api/hooks/wake`, which are in the gap table as surfaces the dashboard does not
have. The channel webhook helpers named in `routes/channels.rs` were removed;
the remaining mention is a comment explaining why the channel cards do not show
them.

## Current inventory

379 routes in 72 groups in `openapi.json`; 326 routes are referenced by the
dashboard source. 26 groups have at least one unreferenced route.

| Group | Routes | Referenced | Unreferenced |
|---|---:|---:|---:|
| `/.well-known/agent.json` | 1 | 0 | 1 |
| `/a2a/tasks` | 3 | 2 | 1 |
| `/api/agents` | 59 | 54 | 5 |
| `/api/approvals` | 11 | 8 | 3 |
| `/api/auth` | 17 | 10 | 7 |
| `/api/authz` | 3 | 2 | 1 |
| `/api/bindings` | 2 | 0 | 2 |
| `/api/budget` | 7 | 5 | 2 |
| `/api/catalog` | 2 | 0 | 2 |
| `/api/channels` | 6 | 5 | 1 |
| `/api/extensions` | 4 | 0 | 4 |
| `/api/hands` | 17 | 15 | 2 |
| `/api/hooks` | 2 | 0 | 2 |
| `/api/inbox` | 1 | 0 | 1 |
| `/api/marketplace` | 1 | 0 | 1 |
| `/api/memory` | 22 | 20 | 2 |
| `/api/migrate` | 3 | 0 | 3 |
| `/api/models` | 7 | 5 | 2 |
| `/api/pairing` | 5 | 3 | 2 |
| `/api/plugins` | 19 | 18 | 1 |
| `/api/profiles` | 2 | 0 | 2 |
| `/api/providers` | 10 | 8 | 2 |
| `/api/provisioning` | 1 | 0 | 1 |
| `/api/ready` | 1 | 0 | 1 |
| `/api/sessions` | 7 | 6 | 1 |
| `/api/versions` | 1 | 0 | 1 |

Every other group in `openapi.json` is referenced by at least one dashboard
module. `/api/logs`, `/api/vault` and `/api/registry`-adjacent routes, previously
unreferenced, are covered in this tree. The six `/api/context-engine` routes
(`health`, `chain`, `metrics`, `traces`, `config`, `sandbox-policy`) are now
covered by the read-only Context Engine page under Observability; it consumes
all six, including the 204 "no plugin engine" branches.

## Uncovered routes

Generated with `--gaps`; grouped by the decision they need.

### Needs a dashboard surface

| Group | Routes | What the operator cannot do today |
|---|---|---|
| `/api/bindings` | `GET/POST /api/bindings`, `DELETE /{index}` | Inspect or edit agent→channel bindings directly; the channel page only edits its own card. |
| `/api/extensions` | `GET`, `install`, `uninstall`, `{name}` | Install or remove an extension from the UI; today it is CLI/config only. |
| `/api/profiles` | `GET`, `GET /{name}` | Browse the tool profiles referenced from agent manifests. |
| `/api/hooks` | `agent`, `wake` | Trigger an agent turn or wake a sleeping agent from the UI. |
| `/api/catalog` | `status`, `update` | See or refresh the model-catalog snapshot. |
| `/api/marketplace` | `search` | Search the marketplace from the dashboard. |
| `/api/models/aliases` | `GET/POST /api/models/aliases`, `DELETE /{alias}` | Manage model aliases. |
| `/api/budget/agents` | `GET /agents`, `/{id}` | Per-agent budget view. |
| `/api/agents` | `bulk`, `bulk/start`, `bulk/stop`, `identities`, `identities/{name}/reset` | Bulk start/stop and agent identity reset. |
| `/api/approvals/session` | `{id}`, `{id}/approve_all`, `{id}/reject_all` | Approve or reject a whole session's pending requests. |
| `/api/pairing` | `complete`, `notify` | Finish or notify a pairing from the dashboard (mobile flow does it elsewhere). |
| `/api/providers` | `github-copilot/oauth/start`, `poll/{id}` | Start a Copilot OAuth flow (the dashboard supports the other providers). |
| `/api/memory` | `bulk-delete`, `user/{user_id}` | Bulk delete memories, or inspect one user's memories. |
| `/api/hands` | `install`, `reload` | Install or reload a hand (packages are handled elsewhere). |
| `/api/plugins` | `doctor` | Run the plugin doctor. |
| `/api/channels` | `registry` | Browse the channel-type registry. |
| `/api/inbox` | `status` | See the inbox status. |
| `/api/sessions` | `search` | Search across sessions. |

### Deliberately not a dashboard surface

`/api/auth/*` (OIDC login/callback/refresh/userinfo/introspect/providers),
`/api/authz/check` (agent-facing), `/api/migrate/*`, `/api/provisioning/status`,
`/api/ready`, `/api/versions`, `/a2a/tasks/send`, `/.well-known/agent.json`.
`/v1/chat/completions` and `/v1/models` are referenced by the dashboard and are
not gaps. `/api/provisioning/status` is a judgment call: the dashboard already
shows provisioning metadata, so a drift screen is arguable but not counted as a
UI gap here.

## Suggested order

1. Extensions (`/api/extensions`): install/uninstall is a real capability gap,
   not just a missing view.
2. Bindings and profiles: small editors that remove hand-editing of TOML.
3. The rest: aliases, catalog, marketplace, hooks, doctor, budget per agent,
   bulk agent actions, approval bulk actions, pairing, Copilot OAuth, memory
   bulk/user, hands install/reload, channel registry, session search.

Each item is a separate change; this document only records the state and the
order, so it does not gate the UI-architecture refactor.
