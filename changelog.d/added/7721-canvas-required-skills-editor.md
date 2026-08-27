The workflow canvas step editor can now set a step's `required_skills`, and the dry-run panel shows the mismatch when an agent cannot satisfy them.
The gate itself shipped one release earlier with no editor surface at all, so the only way to require a skill was to hand-edit the workflow TOML or post the JSON yourself — and because a skill mismatch leaves `agent_found` true, the dry-run panel marked every step green while reporting the workflow as invalid, which is the least useful pair of signals it could have given.
Installed skills autocomplete the box without restricting it: requiring a skill that is declared but not yet installed is a real workflow, and naming that gap is the dry run's job rather than the editor's.
(#7871) (@houko)
