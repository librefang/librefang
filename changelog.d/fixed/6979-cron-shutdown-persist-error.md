The cron scheduler's final persistence attempt during kernel shutdown discarded its result with `let _ = …`, so a failed flush of execution state (a full disk, an unwritable data dir) left no trace anywhere.
`run_cron_scheduler_loop` now logs a structured `warn!` with the underlying I/O error when the shutdown-time persist fails, while still letting shutdown proceed (#6979) (@houko)
