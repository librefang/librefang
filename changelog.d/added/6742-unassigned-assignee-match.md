Teach a task-board trigger to fire on unowned work via `pattern = { task_posted = { assignee_match = "unassigned" } }`.
Previously the only options were "every posted task" or a specific agent, so an agent that should pick up whatever nobody has claimed had to match everything and filter in the prompt.
The keyword matches both spellings of unowned that the system actually produces — an absent `assigned_to` and the empty string — because the stuck-task sweeper releases a claim by writing `assigned_to = ''` rather than NULL, on both the retry and the retries-exhausted path.
Recognising only the absent form would have made the trigger go permanently quiet for any task that had ever been claimed once, which is the opposite of what a pick-up-unowned-work trigger is for.
(#6742) (@houko)
