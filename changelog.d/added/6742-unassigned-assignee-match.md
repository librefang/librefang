Teach a task-board trigger to fire on unowned work via `pattern = { task_posted = { assignee_match = "unassigned" } }`.
Previously the only options were "every posted task" or a specific agent, so an agent that should pick up whatever nobody has claimed had to match everything and filter in the prompt.
The keyword matches both spellings of unowned that reach the event — an absent assignee and the empty string — because neither the `task_post` tool nor `POST /api/tasks` normalises the field, while both do reject an empty title and description.
A client that sends an empty assignee means "nobody", and a filter that only understood the absent form would silently ignore it.
(#6742) (@houko)
