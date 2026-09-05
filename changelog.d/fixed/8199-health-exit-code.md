`librefang health` no longer reports a broken daemon as healthy.
`/api/health` is a liveness probe and deliberately answers HTTP 200 with `status = "degraded"` when a subsystem check fails, so the command printed a green "Daemon is healthy" line directly above `Status: degraded` and exited 0 while the SQLite substrate was unreachable; `--json` returned before inspecting the payload at all.
Both paths now key the success line and the exit code off `status`, exit 1 when it is anything other than `ok`, and name the `checks[]` entries that failed (#8199) (@houko)
