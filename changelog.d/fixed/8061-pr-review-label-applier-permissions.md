The `ready-for-review` / `needs-changes` labels are applied again when a review is submitted.
Splitting the applier's permissions per job in #7546 narrowed the mutating job to `pull-requests: read`, and since the labels endpoint is gated on the pull_requests scope when the issue number resolves to a pull request, every run since has failed with `Resource not accessible by integration` — 24 consecutive failures, and no PR has been labelled from a review since 2026-08-15.
The guard script that was supposed to catch this was never wired into any workflow, so it is now run by CI's workflows lane alongside an assertion on the permission itself.
(#8061) (@houko)
