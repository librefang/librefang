`update_manifest` no longer reprojects an agent's tags twice per manifest PATCH.
It routed a tags change through `registry::update_tags` and then, on the very next line, called `replace_manifest_and_retag` — which already reprojects `entry.tags` and the `tag_index` from `manifest.tags` as part of the same call (#7742).
Both calls fired `notify_changed()`, so every `AgentRegistry` watcher woke twice per PATCH that changed tags — the observable regression this fixes.
In the narrow case where the agent is removed from the registry between the two calls, the redundant call also left `entry.tags` ahead of `entry.manifest` until the next successful write; this fix closes that window too, though it was not reachable in the ordinary PATCH path.
The redundant call is gone; `replace_manifest_and_retag` alone was already doing the whole job. (#7835) (@DaBlitzStein)
