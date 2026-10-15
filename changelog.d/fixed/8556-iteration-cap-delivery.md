A turn that exhausts its iteration budget now delivers what it has, marked as cut off, instead of returning nothing.
Previously, when the per-turn cap (`agent_max_iterations`, default 50) was reached, the loop returned `MaxIterationsExceeded` and the user saw silence indistinguishable from a hang.
The non-streaming and streaming loops now return the prose accumulated across intermediate tool-use iterations followed by a `[Stopped after reaching the iteration limit (N) before finishing …]` notice, or the notice alone when the turn only ran tools, together with a new `AgentLoopResult::hit_iteration_cap` flag.
On the streaming path the notice is emitted through the stream pipe so channel and WebSocket users see it, and the WebSocket `response` frame carries `hit_iteration_cap: true`.
The kernel pushes a `max_iterations` notification, which routes to `alert_channels` by default and is accepted in `[[notification.agent_rules]]`, so operators keep the alert the old `task_failed` push gave them; autonomous agents no longer also report the capped turn as `task_completed`.
A cron job whose run hits the cap is recorded as a failed run, so a job that caps on every fire is still auto-disabled after repeated failures.
A capped turn never triggers a background skill review.
The `AgentLoopEnd` hook keeps `reason = "max_iterations_exceeded"` and adds `delivered: true` (#8556, #8557) (@DaBlitzStein)
