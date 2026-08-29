The checksum on `GET /api/config/status` covered only the primary `config.toml`, so a deployment using `include = [...]` could edit an included file, change the effective configuration, and see the checksum stay identical.
An operator comparing it against a Kubernetes `checksum/config` annotation to confirm a rollout had landed was told nothing had happened.
It now covers every file that contributes, with a new `includes` field listing them; a deployment with no includes keeps the exact digest it had before, so existing rollout annotations continue to match.
The Kubernetes manifest checker consequently stops banning `include` outright and instead verifies that each included file is another key of the same ConfigMap.
(#7921) (@houko)
