A goal run's `verify_max_retries` now survives a pause the same way `max_iterations` already does.
A run started with an explicit retry budget reported the compiled default instead once paused, and a bodyless `/resume` re-budgeted it to that default rather than restoring the operator's own number — the checkpoint never carried the field. (#7973) (@DaBlitzStein)
