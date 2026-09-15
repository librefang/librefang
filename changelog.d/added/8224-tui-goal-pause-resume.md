The TUI's Goals screen can now pause and resume a run with `p`, which it could already show but not act on.
`main` already colours a paused run yellow and ships `tui-goals-phase-paused` and `tui-goals-run-paused`, so before this an operator watching from a terminal saw a run somebody had paused from the dashboard, correctly labelled, with no key that touched it.
The key reads the live run phase rather than the goal document: a running goal pauses, a paused one resumes, and one doing neither is left alone rather than being started, because `s` is the key that starts a run and a pause key that quietly launched one would be a surprise on a screen where "stopped" and "paused" sit next to each other.
Resume sends no body, which is the daemon's "keep the cap the paused run was already under" path — re-budgeting a resumed run is a deliberate act and belongs to a surface that can ask for the number, not to a single keypress.
(#8224) (@DaBlitzStein)
