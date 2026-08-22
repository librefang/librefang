```release-note:added
Added ModelRouter — a tag-based dynamic model selection system (alpha, disabled by default). New `model_profiles.toml` with profiles maps task tags to specific models/providers. Complexity evaluator picks the best profile. New `agent_spawn` params: `profile` and `model_override`. (@DaBlitzStein)
```
