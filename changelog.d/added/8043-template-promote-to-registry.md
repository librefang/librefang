Agent types can be promoted to the public registry as a GitHub PR via `POST /api/templates/{name}/promote`, the dashboard Promote button, or the TUI `[p]` key.
The manifest is sanitized for publication before submission, and promotion is refused with a 409 while a privacy finding still sits inside a field the sanitizer keeps, reusing the same fork/push/PR machinery the skill propose flow uses.
(#8043) (@DaBlitzStein)
