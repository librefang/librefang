Agent instructions now cover two failure modes that produced real incidents during a large backlog sweep.
A private `CARGO_TARGET_DIR` is forbidden: a second target directory is invisible to whoever is watching disk, and two of them do not fit — one such directory reached 24 GB unnoticed while the shared one was being pruned.
The merge-sweep guidance gains the mechanism by which two green pull requests break `main` together, which is a shared *type* rather than a shared file, and the cheap guard of merging `main` into each queued branch before merging it.
It also records that `CI Gate` reports a `cancelled` job as a failure and never re-evaluates it, so a red check is not always a failed test.
(#PR) (@houko)
