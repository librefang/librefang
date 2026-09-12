Running the same goal a second time deleted the first run's captured `GOAL_LEARNED:` lessons, because they were stored under a key scoped to the goal rather than to the run.
Lessons are now keyed by the run's own start time as well as the goal id, so re-running a goal no longer costs the operator lessons they may never have read (#7785) (@DaBlitzStein)
