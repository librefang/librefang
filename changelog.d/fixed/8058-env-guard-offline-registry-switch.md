A test helper that overrode an environment variable erased it on cleanup instead of putting back what it held, which disarmed the `LIBREFANG_REGISTRY_OFFLINE=1` that CI sets for the whole macOS test job.
The macOS lane's Mach-port guard runs every kernel unit test as threads in one process, so every kernel booted after that point performed a live registry sync, and the synced `researcher` agent type shadowed the deliberately-broken templates three step-agent tests seed — turning their expected errors into successful spawns and reddening `main`.
Guards now restore the previous value, and the step-agent fixture freezes the registry sync in its own config rather than trusting the ambient environment.
(#8058) (@houko)
