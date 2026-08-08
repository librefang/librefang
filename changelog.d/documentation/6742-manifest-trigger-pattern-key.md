Correct the two task-board trigger snippets in the trigger-dispatch-concurrency guide, which documented a field that does not exist.
Both wrote `event = "task_posted"`, but `ManifestTrigger` has no `event` field — the key is `pattern` and the value is the externally-tagged enum form `pattern = { task_posted = {} }`.
Because `ManifestTrigger` derives `#[serde(default)]` the unknown key was dropped in silence, `pattern` fell back to JSON `Null`, and reconcile skipped the entry with a warning, so an operator copying either snippet got a manifest that parsed cleanly and registered no trigger whatsoever.
The guide now also states that the key is `pattern`, explains why a typo there fails quietly, and shows the filtered `assignee_match` form so the narrower shape is discoverable.
(#6742) (@houko)
