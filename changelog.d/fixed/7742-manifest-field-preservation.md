Two agent-manifest fields that a running agent reported back but could never actually be changed.
`tags` was overwritten with the stored value on every write, so the dashboard, `PATCH /api/agents/{id}` with `manifest_toml` and the CLI all reported a successful save and changed nothing — the registry had no way to re-project a new tag list onto the runtime lookups, so pinning them was the only safe option available at the time.
It now has one, and the system-owned `hand:*` tags stay pinned on their own because they route an agent's workspace and decide whether its tool calls need an approval gate.
`tools_disabled` was returned by `GET /api/agents/{id}/tools` but forced back to `false` by every successful write to that route, which meant an operator editing a blocklist silently re-enabled every tool on an agent they had deliberately switched off.
It is now the fourth tri-state field of that request alongside the three lists: omit it and the stored value is left alone, send it and it is written.
(#7866) (@houko)
