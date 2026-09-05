The TUI agent-types screen gains two registry-backed actions on manifest-backed rows: `Shift+R` restores the agent type to its original registry definition through `POST /api/templates/{name}/restore`, and `v` opens a version-history table fed by `GET /api/templates/{name}/history`.
A failed history fetch now reports the daemon's status and reason in place, instead of rendering every failure — a 404, a 500, a connection refused — as an indistinguishable empty list.
Both keys are advertised in the screen's hint bar (#8073) (@DaBlitzStein)
