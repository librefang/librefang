# GitHub collaboration & wait policy

LibreFang is an open-source project with heavy AI-assistant traffic.
These rules keep maintainers in control of their own PRs and issue threads.
[`AGENTS.md`](../../AGENTS.md) carries the single-page summary and [`CLAUDE.md`](../../CLAUDE.md) the enforced short form; this page is the full policy with the incidents that produced it.

## Touching other people's work

- **Don't close PRs or issues opened by others** unless the maintainer directly instructs you to.
  By default, post a comment recommending closure with the linking commit or PR and let the maintainer pull the trigger.
  When directed to close, the close comment must state the substantive reason — review bugs, superseded by, scope mismatch — so the original author understands what went wrong.
  Do not attribute the close to "AI" / "Claude"; the reason itself is what matters.
- **Force-push only to your own branches, only before review.**
  Once a reviewer has loaded the diff, prefer fixup commits or a follow-up PR over rewriting history.
  Force-push to `main` / `master` is blocked by `guard-bash-safety.sh` and requires explicit user OK regardless.
- **Don't reassign, re-label, or re-milestone** issues or PRs you did not open unless directed.
  Self-assigning a triage label or adding `needs-review` is auto-OK; flipping `priority` / `release` labels is not.

## Commit & PR hygiene

- **No Claude / Anthropic / AI attribution** in commit messages, PR bodies, or issue comments.
  The `commit-msg` git hook rejects matching strings and the PreToolUse Bash hook catches the inline-flag form.
  Don't route around either — the rule exists because attribution pollutes `git log` and signals provenance the project does not want to imply.
- **One PR ↔ one issue**, or one tight cluster.
  Don't bundle unrelated refactors with the requested change.
  A real out-of-scope problem gets a separate issue or follow-up PR, mentioned in the current PR's "Out-of-scope follow-ups" section.
- **Fix what you found — don't punt it to a "follow-up".**
  Anything you noticed while reading or writing the code in this PR is in-scope by definition: review nits, mismatched HTTP status codes, missing log fields, redundant lookups, stale comments, type-shape inconsistencies, small clippy noise.
  Treat the phrases "follow-up", "long-term improvement", "next PR", "future cleanup", "out-of-scope follow-up issue", and "leave for later" as red flags in your own review or summary output — they almost always mean "I saw the problem and decided to defer the work".
  The bar to defer: would fixing it require touching a *different* crate or domain than the one you're already in?
  If no, fix it in this PR.
  If yes, surface the question to the human reviewer with the concrete trade-off and ask before deferring; don't decide unilaterally.
  The same rule applies when you re-evaluate a deferred item and decide it's a "non-issue" — back that decision with the file/line evidence that contradicts the original concern, in the same response.
  "I looked again and it's fine" without evidence is another form of punting.
- **PR body must enumerate** the substantive changes, the verification performed (integration test names, `cargo check --workspace --lib` output, scoped `cargo test -p <crate>` runs), and any deferred work.
  Bullet form, no marketing prose.

## CI wait policy

CI is shared infrastructure and frequently slow.
Polling it from an AI session burns turns without producing information.

- **Total polling budget: about 5 minutes, in 60–270 s chunks.**
  Anthropic's prompt cache TTL is 5 minutes, so keep each wake-up inside that window to keep the cache warm; ~300 s is the worst case (cache miss without amortizing).
  Don't reach for 1200 s+ "save my turns" waits here — that violates the 5-minute total cap and reintroduces the long `gh run watch` / sleep behaviour the policy exists to prevent.
  After the budget is spent, push, leave the run URL in the PR or report, and **stop**.
  Long waits *are* appropriate elsewhere, such as an autonomous-loop tick polling for an external job — just not for in-session CI polling.
- **Don't pre-emptively re-run a check** that has not yet failed.
  Only retry after a recorded failure, and only once.
- **Don't open follow-up issues or pivot the plan** while waiting for CI or review.
  If you cannot make further progress without information you do not have, report status and yield — don't invent more work.
- **Don't add reviewers, flip `ready-for-review`, or `gh pr ready`** on someone else's behalf, and don't re-request review on your own PR unless a maintainer has explicitly asked you to ping them.
  Maintainers pull work into their queue; AI agents do not push it onto theirs.

## Batch merging: the Actions runner pool is the bottleneck, not the merge

The `librefang` org is on the **free** GitHub plan and every job targets the hosted `ubuntu-latest` pool, so concurrent runners are capped org-wide.
Every push to `main` re-triggers the full workflow set, and so does every push to a PR branch.
Merging a backlog back-to-back therefore spends the whole concurrency budget on runs nobody will ever read: after 86 consecutive merges the queue held 657 jobs with **zero** executing, the oldest waiting 67 minutes, and the only run whose result mattered — CI Gate on the current `main` HEAD — was stuck behind 85 superseded ones.

When clearing more than roughly 10 PRs:

- **Merge in batches and let the queue drain between them.**
  Check `gh api 'repos/librefang/librefang/actions/runs?status=in_progress&per_page=1' --jq .total_count` — if it reads 0 while jobs are queued, the pool is saturated and nothing will progress until something is cancelled or times out.
- **Cancel superseded runs rather than waiting them out.**
  A queued run is dead weight when its `head_sha` is no longer its branch's tip: on a PR branch a newer commit has replaced it, and on `main` only HEAD's checks are ever consulted.
  Resolve tips with `git ls-remote origin` and compare, rather than assuming the newest run per branch is the live one.

  ```bash
  # Queued runs whose head_sha is no longer the branch tip, cancelled.
  gh api --paginate 'repos/librefang/librefang/actions/runs?status=queued&per_page=100' \
    --jq '.workflow_runs[] | "\(.id)\t\(.head_branch)\t\(.head_sha)"' > /tmp/q.tsv
  git ls-remote origin | awk '$2 ~ /^refs\/heads\//{sub("refs/heads/","",$2); print $2"\t"$1}' > /tmp/tips.tsv
  awk -F'\t' 'NR==FNR{tip[$1]=$2; next} ($2 in tip) && tip[$2]!=$3 {print $1}' /tmp/tips.tsv /tmp/q.tsv \
    | while read id; do gh api -X POST "repos/librefang/librefang/actions/runs/$id/cancel" >/dev/null; done
  ```

  Leave runs whose branch no longer exists alone — a deleted branch means the PR already merged or closed, and its checks are nobody's gate.

  **Never cancel a run whose `head_sha` *is* its branch's tip, even to free capacity.**
  `CI Gate` fails on `cancelled` exactly as it does on `failure`, and it does not re-evaluate when the lane is later re-run: the gate stays red while every lane above it reads green.
  Cancelling live runs to unblock one urgent PR left 25 healthy PRs looking broken, each showing `CI Gate: one or more lanes failed or were cancelled` as its only red check, and each needing a fresh push to clear.
  The superseded-run rule above is safe precisely because it never touches a tip.
- **A stalled queue is not a CI failure.**
  Before diagnosing a PR as broken, confirm jobs are actually executing.
  `runner_name` empty across the board (`gh api repos/librefang/librefang/actions/runs/<id>/jobs --jq '.jobs[].runner_name'`) means unassigned, i.e. starved for capacity, not failing.
- **`CI Gate` red with every lane green means the gate saw a cancellation, not a defect.**
  Read the gate's own log before touching the branch: `CI Gate: one or more lanes failed or were cancelled` with no failing lane is a stale verdict, cleared by re-running CI (update-branch or an empty push), not by changing code.

## Two green PRs can still break `main` together

The `main` ruleset does not set `strict_required_status_checks_policy`, so a PR merges on the CI it ran against **its own base**, which may be many commits behind.
Two PRs that touch the same file on the same stale base are each validated in isolation and neither result says anything about the combination.

This is not hypothetical, and text-level conflict detection does not catch it — git sees edits to different lines and merges cleanly:

> #6814 added two `redact_metadata(&Value::String(…))` call sites while the function still took a reference.
> #6815 changed the signature to take the value by owner and updated the call sites *it* could see.
> Both branched from `37937950b`, both were fully green, and they were merged two seconds apart.
> The result did not compile, and because the break sits in `#[cfg(test)]` code it only surfaced on the one lane that builds test targets — turning `Build / Linux aarch64` red on `main` and failing the CI Gate of 30+ open PRs that had nothing to do with it.

So when merging several PRs in one sweep:

- **Group by file, not just by conflict status.**
  Before starting, map changed files to PRs (`gh pr view <n> --json files`) and treat any file touched by more than one PR as a serialization point: merge one, let the others pick up the new `main`, and re-check.
- **Second and later PRs in a group need a fresh check run, not just a clean merge.**
  `MERGEABLE` only means git can splice the text.
  Push the branch onto current `main` (or use the update-branch API) so CI re-runs against the combination that will actually land.
- **Watch for a signature or contract change anywhere in the group.**
  A PR that changes a function's parameters, return type, or ownership is the dangerous side of the pair — any *other* PR in the batch that calls it was written against the old contract.
- **After a batch, verify `main` itself rather than assuming.**
  Green on each PR does not imply green on the merge result; check the CI Gate on the new `main` HEAD before starting the next batch.

## Issue / PR comment etiquette

- **At most two follow-up comments** on the same thread without human input.
  After that, stop and wait — repeated AI-generated pings on a silent thread are noise, not progress.
- **Don't comment on threads you have no action on.**
  "Looks good" drive-bys from an AI account add nothing.
- **When you reply, link evidence:** commit SHAs, file paths, test names.
  No vibes-only comments.

## Conflict resolution

- **Latest maintainer intent wins.**
  When rebasing or resolving merge conflicts that touch a human-authored hunk, keep the maintainer's edit.
  If the two sides genuinely disagree, surface the conflict in the PR body and ask — don't silently pick the smaller diff.
- **Preserve both sides' intent** during conflict resolution.
  Dropping a hunk because "it'll be reapplied later" is how regressions land.
