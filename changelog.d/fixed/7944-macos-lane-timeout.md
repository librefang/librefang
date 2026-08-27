Stop `main` going red on a timeout no code change causes.
`Test / macOS` runs 43-57 minutes against a 60-minute cap, so a slow runner spends the whole 3-17 minute margin and the job is killed — three of the last ten pushes to `main` died that way, each within a minute of the cap (`f7130c1` at 60:28, `efa84ea` at 60:41, `2b871dd` at 60:23), every other lane in those runs green.
GitHub reports a timeout as `cancelled`, `CI Gate` fails on `cancelled` without re-evaluating, and re-running the merged commit cannot clear it, so each occurrence files a `[main red]` issue against a tree that is fine.
Raised to 90 minutes, the cap `Test / Windows` already carries — in one of those same runs Windows took 63 minutes and passed for no reason other than its more generous limit. (#7944) (@houko)
