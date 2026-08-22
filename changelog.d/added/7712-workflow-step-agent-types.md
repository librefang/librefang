Workflow steps can reference an agent type with `{ type = "name" }` — find-or-spawn semantics: reuse the registered agent with that name, else spawn from the template manifest (templates/ then workspaces/agents/) with the canonical name-derived UUID, and fail the step with a ByType-specific error when neither exists.
 (@DaBlitzStein)
