`GET /api/skills` and `GET /api/skills/{name}` now report what a skill needs, not only what it provides: `required_tools` and `required_capabilities` echo the manifest's `[requirements]` table, and the list route adds `required_tools_count` beside `tools_count`.
Before this an operator could assign a skill to an agent whose tool grants did not cover it, and the mismatch surfaced only when the skill ran and its tool call was refused.
The `required_` prefix keeps the direction unambiguous, since `tools` on the detail route already means the tools the skill provides and keeps that meaning.
Both lists are always present, so a skill that declares nothing reports empty arrays rather than a missing key, which a client can tell apart from "unknown". (#8445) (@houko)
