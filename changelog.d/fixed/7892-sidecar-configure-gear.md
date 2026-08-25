The Configure gear now appears for sidecar channels in the dashboard.
It was gated on `category !== "sidecar"`, written when sidecars were config.toml-only and a configure POST would have 404'd — but `POST /api/channels/sidecar/{name}/configure` shipped in #5252 and every channel has reported `category: "sidecar"` since the in-process registry was removed, so the condition was never true and the button never rendered.
The endpoint was therefore unreachable from the UI for any already-configured sidecar, and the only way in was the add-channel picker, which is for types that are not configured yet.
Slack's channel scope, reply threading, reaction feedback and file forwarding also move out from behind "Show advanced", since every non-secret field was flagged advanced and the drawer opened showing only the two tokens.
(#7894) (@houko)
