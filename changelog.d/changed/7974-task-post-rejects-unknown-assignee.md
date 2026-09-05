`POST /api/tasks` (and the `task_post` agent tool) no longer accepts an assignment to an agent that is not registered: the post is refused with a 400 naming the assignee, instead of queueing a row that nothing could ever claim.
Workflows that post work for an agent that does not exist yet must create the agent first — the old queue-anything behaviour let such a task sit `pending` forever, and nothing woke the assignee.
A task for a registered agent that is currently stopped is still accepted and waits for it to come back.
(#7974) (@DaBlitzStein)
