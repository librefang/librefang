A goal run's captured `GOAL_LEARNED:` lessons are no longer queued as a pending skill draft for an agent that never opted into the skill workshop.
The workshop is default-off and opted into per agent (`agent.toml: [skill_workshop] enabled = true`), but the goal runner's learnings hook queued a draft regardless, because nothing in that path read the setting.
It now checks `enabled` and `auto_capture`, the same gate every other automatic capture path in the workshop already applies. (#7973) (@DaBlitzStein)
