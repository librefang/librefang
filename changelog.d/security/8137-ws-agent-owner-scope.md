Closed a cross-user hole in the agent chat WebSocket: it authenticated the bearer but never checked that the caller owned the agent, so any `User`-role token could open a socket on an agent authored by someone else and drive full LLM turns on it — tool execution and provider spend included — by guessing or enumerating its UUID.
The REST twin `POST /api/agents/{id}/message` has carried that ownership check since #6753; the upgrade path had only an existence check, and now runs the same one, refusing with the same not-found shape so a non-owner cannot use the status to confirm the id exists.
Admins and the agent's own author are unaffected, as are loopback and no-auth deployments.
(#8137) (@houko)
