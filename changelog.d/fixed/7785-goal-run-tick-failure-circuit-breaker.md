A goal run no longer burns its whole iteration budget rediscovering a deleted agent.
Every failing tick was retried until the iteration cap, so a run pointed at an agent that had been removed, or at a provider whose key had been revoked, spent its full budget failing identically each time and then reported the cap as the reason it stopped.
Five consecutive failures that are not rate limits now end the run in `Stopped`, with the underlying error left on the run's `last_error` — the operator gets the fault instead of a healthy-looking exhausted budget.
Rate limits keep their own separate streak, so a provider throttling a run still ends it as `RateLimited`.
(#7785) (@DaBlitzStein)
