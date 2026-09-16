The daily stale-PR reconciliation no longer makes unrelated pull requests look like they are failing.
Its concurrency group was the fixed string `stale-pr-reconciliation`, so every push in the repository contended for one slot; with `cancel-in-progress: false` GitHub cancels the older pending run when a newer one queues behind the one in flight, and `gh pr checks` renders a cancellation as `fail`.
During any busy window that left a spread of open PRs sitting at `UNSTABLE` over a housekeeping job that had never looked at them — eight of them at once while a dependency sweep was landing.
The group is keyed per pull request now, and the scheduled run still serializes against itself because `github.ref` is the default branch for a `schedule` event (#8390) (@houko)
