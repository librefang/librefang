Fixed a race in `CronScheduler::add_job` where concurrent creators could each pass the global and per-agent job-limit checks before any of them inserted, letting the total job count exceed the configured cap.
Capacity checks, validation, and insertion are now serialized on a dedicated lock so the whole add sequence is atomic (#6970) (@houko)
