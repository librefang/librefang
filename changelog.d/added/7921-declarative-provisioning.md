A deployment can now declare its agents in a directory it owns, the way it already declares its configuration.
Point `LIBREFANG_PROVISIONING_PATH` at a tree of `agents/*.toml` and the daemon reconciles it at boot, then refuses every API route that would rewrite one of those manifests with `423 Locked` and `code: "resource_provisioned"` — while agents created at runtime stay fully editable, because ownership is per resource rather than a global switch.
Operating a provisioned agent is untouched: suspend, resume, messages and sessions all still work, since none of that is something the next reconcile would overwrite.
Removing a declaration releases the agent back to runtime ownership by default rather than deleting it, so a removal is reversible and only the explicit `LIBREFANG_PROVISIONING_PRUNE=delete` destroys anything.
`GET /api/provisioning/status` reports where each resource came from, whether the tree has drifted from what is running, and every file the last reconcile refused — a malformed manifest never fails the boot, so that endpoint is where an operator finds out why an agent is missing.
(#7921) (@houko)
