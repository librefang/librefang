`[task_board] claim_ttl_secs = 0` now disables only the global stuck-task clock: a task that carries its own `timeout_secs` is still reclaimed when that deadline passes.
  Previously the sweeper skipped every row when the global TTL was `0`, so a per-task timeout had no effect on a board configured that way.
  To keep a board manual-only, do not set per-task timeouts: after upgrading, a task that carries one expires on its own deadline and a human's claim can be reclaimed from under them (#7974) (@DaBlitzStein)
