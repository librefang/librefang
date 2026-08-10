Harden the discussion-to-issue and weekly-report workflows against partial failures and unsafe assumptions.
The discussion backfill now serializes through a concurrency group, bounds every job with a timeout, and records per-discussion failures instead of continuing past them silently.
The manual `/to-issue` command now requires an exact token match instead of a substring match, so a comment that merely contains that text can no longer trigger a promotion.
The weekly report now fails closed on any command error, resolves the repository from `github.repository` instead of a hardcoded name, and surfaces Discord delivery failures instead of swallowing them (#6904) (@houko)
