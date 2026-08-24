Every GitHub Actions job now declares an explicit `timeout-minutes`, so a wedged job dies on its own instead of holding a runner slot until GitHub's 360-minute default expires.
  Two `main` CI runs recently hung `in_progress` for over nine hours and, because the `main` concurrency group is keyed per-sha and deliberately never cancels an older run, they held the repository's only two concurrent execution slots while more than 80 runs queued behind them and every open PR stalled.
  Caps are sized from each job's observed duration, generously enough that a cold cache or normal variance never trips one — most sit an order of magnitude above their slowest recorded run.
  The desktop release build is the deliberate exception: its slowest successful run took 206 minutes, and since GitHub caps any job at 360 minutes there is no room for a wide margin, so it is capped at 345 to preserve the guarantee that no job can occupy a slot for a full six hours.
  A new `Workflow Job Timeouts` gate keeps the coverage from regressing, because a job added without the key silently inherits the six-hour default again.
  (#7836) (@houko)
