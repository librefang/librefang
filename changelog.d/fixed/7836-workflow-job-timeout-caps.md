Every GitHub Actions job now declares an explicit `timeout-minutes`, so a wedged job dies on its own instead of holding a runner slot until GitHub's 360-minute default expires.
  Two `main` CI runs recently hung `in_progress` for over nine hours and, because the `main` concurrency group is keyed per-sha and deliberately never cancels an older run, they held the repository's only two concurrent execution slots while more than 80 runs queued behind them and every open PR stalled.
  Caps are sized from observed job durations — roughly a 3x margin over the slowest recent run — so a cold cache or normal variance never trips one, and tag-driven release builds keep the headroom their real cost demands.
  (#7836) (@houko)
