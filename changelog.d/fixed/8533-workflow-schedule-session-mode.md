Workflow schedules now reject `session_mode` with a 400 instead of storing a setting the dispatcher never reads.
The message points to `WorkflowStep.session_mode`, which is where per-step session behaviour is configured; a value persisted by an older daemon is dropped on load so the job keeps firing as before (#8587, #8533) (@houko)
