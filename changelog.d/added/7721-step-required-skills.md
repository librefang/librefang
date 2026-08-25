Workflow steps can now declare `required_skills`, and a step whose agent cannot actually use one of them fails before dispatch with an error that names the step, the agent, the skill, and the fix.
  The check resolves every required name against the loaded skill registry independently of the agent's allowlist mode, which matters because the default `skills = []` means "every skill that is loaded", not "every name you can type" — a requirement for a skill nobody installed would otherwise sail through validation and surface deep in the agent loop as a generic tool error.
  Three failure classes are reported separately because each has a different fix: not declared by the agent (widen its `skills` list), declared but not loaded (install the skill and reload the registry), and no such skill on this instance (a typo).
  A dry run reports the same text, so the mismatch is visible before a run burns its earlier steps.
  (#7863) (@houko)
