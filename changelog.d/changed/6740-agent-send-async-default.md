`agent_send` now delegates non-blockingly by default when the calling agent is known, returning a `task_id` whose reply is delivered to the caller's session on completion.
  The previous blocking default required the model to predict in advance that a delegation would be slow and opt in to `"async": true`; a wrong guess spent the entire turn waiting for `tool_timeout_secs`.
  An unnecessary `task_id` costs one extra turn to collect, whereas an unnecessary block can lose the turn outright.
  Pass `"async": false` for a quick sub-question whose answer is needed within the same turn.
  Callerless system-initiated sends keep dispatching synchronously, because the async tracker requires a known caller agent to route a completion back to. (#6740) (@houko)
