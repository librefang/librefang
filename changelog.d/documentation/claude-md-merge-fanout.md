The agent instructions now record why a large merge sweep stalls CI, and how to read the queue without being misled by the tooling.
A sweep saturates the runner pool through the housekeeping workflows each merge fires, not through the merges themselves, and the required-check surface is narrow enough that cancelling those is safe while cancelling the secret scan is not.
Separately, `gh run list --limit N` silently truncates and `--status in_progress` counts runs rather than jobs, which between them produced three wrong diagnoses of one incident.
(#7926) (@houko)
