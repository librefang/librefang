The boot recovery sweep for goal runs can no longer take over a goal run that is live.
`GoalRunner::recover_stale_runs` demoted every stale-looking `Running` row to `Stopped` and wrote a task-less placeholder into the run registry without taking the start/stop lock or looking at what the registry already held.
A run started while the sweep was in flight, or a resumed run whose `started_at` comes from its old checkpoint, could have its durable row demoted and its registry entry replaced, so `stop()` removed only the placeholder and the loop kept issuing agent turns until its iteration cap while `GET /api/goals/{id}/run` reported it stopped.
The sweep now holds the same lock as `start()` and `stop()` and skips any goal whose registry entry still owns a loop (#8472, #8429) (@houko)
