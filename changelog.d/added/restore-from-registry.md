Agent types can be restored to their original registry definition from the dashboard.
A diff view shows every changed field before overwriting, so the operator can review what diverged.
New endpoints: `GET /api/templates/{name}/registry-diff` and `POST /api/templates/{name}/restore`. (#8042) (@DaBlitzStein)
