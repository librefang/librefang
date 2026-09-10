The TUI's Auxiliary settings tab now reports what went wrong instead of showing a result that quietly misdescribes the operator's config.
The list of available auxiliary tasks is served by the daemon as `x-aux-tasks` on the config schema, and a failed request or a schema without that field both fell through to an empty list with nothing on screen to distinguish "the daemon could not be reached" from "you have configured everything there is".
Those two cases are now reported separately, and the tasks the config document does carry are still rendered, so a transport hiccup does not blank the pane.
The messages are also visible now: the pane never drew its own status line, so every message it wrote went nowhere.
Beyond the task list, a config request that came back `401` or `500` used to be read as success and rendered every task as "not configured" — telling an operator with a rejected request that their chains were empty — and a save that the daemon refused (`403` on a non-writable path, `423` under a managed config, `400` on a chain that does not deserialise) was discarded silently, so a rejected edit looked exactly like one that did not take.
A failed fetch also left the whole Settings screen spinning, since none of its four panes clear the loading flag on error.
(#8059) (@DaBlitzStein)
