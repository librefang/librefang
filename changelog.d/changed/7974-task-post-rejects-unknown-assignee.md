`POST /api/tasks` and `POST /api/comms/task` no longer accept an assignment to an agent that is not registered: both routes refuse it with a 400 naming the assignee, instead of queueing a row that nothing could ever claim.
  The `task_post` agent tool performs the same kernel check, but an agent has no HTTP status — the tool returns an error naming the unknown assignee rather than a result saying the task was created.
  Workflows that post work for an agent that does not exist yet must create the agent first — the old queue-anything behaviour let such a task sit `pending` forever, and nothing woke the assignee.
  A task for a registered agent that is currently stopped is still accepted and waits for it to come back (#7974) (@DaBlitzStein)
