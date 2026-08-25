The `release-tag` workflow parses again, and the workflow guard now rejects a duplicate key instead of letting one reach `main`.
Merging a branch that set `timeout-minutes: 10` on the tag job against a `main` that had just set `timeout-minutes: 15` on the same job kept both lines, which GitHub Actions refuses to start — it reported a red run with no jobs on every branch pushed thereafter, which reads like a failing test rather than a malformed file.
`yaml.safe_load` silently keeps the last of two identical keys, so every YAML check in the repository passed; the guard that already parses these files now refuses the duplicate outright.
(#7918) (@houko)
