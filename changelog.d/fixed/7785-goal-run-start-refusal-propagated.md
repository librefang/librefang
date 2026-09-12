Starting an autonomous goal run always reported success even when the runner itself refused, because the kernel discarded `GoalRunner::start`'s return value and hardcoded `true`.
A goal that vanished between the API handler's read and the runner's own load — a delete racing a start — made `/goal` from the dashboard chat, a channel bridge or the TUI print "Goal created and started" for a run that never started, and left the API's own `started` check on that path permanently dead.
The refusal now reaches the caller (#7785) (@DaBlitzStein)
