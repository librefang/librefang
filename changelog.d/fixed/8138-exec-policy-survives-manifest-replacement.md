Hot-reloading an agent's `agent.toml`, or saving its manifest from the dashboard, no longer clears the shell policy the agent was running under.
`exec_policy` is resolved once from your global `[exec_policy]` when the agent starts and is not written into the manifest file you edit, so both of those paths used to replace it with "unset" — and nothing downstream reads "unset" as deny, which handed a `deny` agent the `shell_exec` tool back and skipped the allowlist on every command until the daemon was restarted.
An `[exec_policy]` you write in `agent.toml` still takes precedence, as before.
(#8138) (@houko)
