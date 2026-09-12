Loop engineering for autonomous goal runs, opt-in per goal.
A run ended the moment the agent wrote `GOAL_DONE`, which left the worker as the sole judge of its own work — the one check a long-horizon loop most needs, and the one it did not have.
Setting `loop_engineering` on a goal adds two judges that are not the worker.
A verifier agent (`verify_agent_id`) reads each iteration's output and returns `VERDICT: PASS|FAIL|NEEDS_REWORK`; a rejection goes back to the generator carrying the verifier's stated reason, up to `verify_max_retries` rework rounds, and until the verifier passes the work `GOAL_DONE` does not end the run.
An evaluator model (`evaluator_model`) makes one cheap yes/no read of the goal against the latest output, and can conclude the goal is met even when the agent never claimed it.
Because the verifier now stands between the agent and the end of the run, `GOAL_DONE` changes meaning: it is a request to finish, granted once the verifier passes the work it is attached to, rather than the finish itself.
The agent can also record a reusable lesson with `GOAL_LEARNED: <one sentence>`; captured lessons are replayed into later iterations' prompts, persisted to the shared store, and filed as a draft skill in the workshop's `pending/` queue.
Turning a run's lessons into a skill an agent actually loads still takes a human `librefang skill pending approve` — an autonomous loop proposes, it does not grant itself authorship.
All three are inert unless the goal opts in, so a goal that does not ask for them sends the same prompt and makes the same single LLM call per iteration it always did.
Sub-agents are delegated rather than provisioned: the prompt directs the agent to its own `agent_spawn` / `agent_send` tools, which run under the capabilities its operator granted it, and neither the runner nor the API ever creates an agent on a caller's behalf.
First of the five PRs splitting the closed #6505 (#7785) (@DaBlitzStein)
