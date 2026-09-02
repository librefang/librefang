Stop pinning one of the five concurrent-LLM-call slots for the whole retry backoff.
Both retry loops acquired the permit with `let _permit = …` at loop-body scope, which reads like a scoped guard but keeps it alive past the backoff sleep, so a single rate-limited agent could hold a slot for minutes with no request in flight on it and starve everyone else behind the cap.
The permit now covers the driver round-trip only — the streaming path included, where it also spans the forwarding task's join — and is released before the sleep (#8136) (@houko)
