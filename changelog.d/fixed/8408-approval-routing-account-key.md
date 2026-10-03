Route approval notifications to a sidecar by the config `name` the router is seeded with, not by the optional `account_id` the sidecar reports in its `ready` event.
A sidecar that reported none fell back to the bare channel key, which no sidecar is ever registered under, so every approval missed every adapter, stayed queued, and filled the per-agent pending-approval cap until the agent could no longer call a tool at all.
The seed, the inbound `metadata["account_id"]` stamp and this lookup now all read the same identity (#8418) (@DaBlitzStein)
