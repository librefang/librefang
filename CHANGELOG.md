# Changelog

All notable changes to LibreFang will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project uses [Calendar Versioning](https://calver.org/) (YYYY.M.DD).

## [Unreleased]

## [2026.8.19] - 2026-08-19

_474 PRs from 5 contributors since v2026.7.31._

### Highlights

- **Security hardening** — dozens of fixes closing SSRF vectors, path traversal in skill/channel IDs, XSS in canvas and OAuth callbacks, credential redaction gaps, and durable atomic writes throughout the daemon to prevent partial-state corruption.
- **Managed configuration mode** — new `managed` mode locks provider config routes so self-hosted deployments can enforce a fixed LLM setup; pairs with opt-in model discovery and API key support for custom and local providers.
- **Long-form audio/video transcription** — recordings are now processed in sliding windows and written directly to a file, removing the previous length cap on transcription.
- **Non-blocking agent messaging and smarter task waking** — `agent_send` is now non-blocking by default when called from within an agent turn; posted tasks automatically wake their assignee without requiring a separate trigger declaration.
- **Polish localization and i18n fixes** — Polish (`pl`) added as a supported language; Japanese, Spanish, and French error message translations restored and completed.

### Added

- Scan the runtime container image for vulnerabilities with Trivy, on `main` and on every native release digest, so OS packages and the bundled Node.js / Python dependencies stop sitting outside the source and dependency checks.
  In the release pipeline the scan sits between the per-architecture `push-by-digest` build and the manifest publish, and `docker-manifest` now depends on it — no user-facing tag (`:VERSION`, `:latest`, `:lts`) can be created or moved onto a digest that failed the gate, and nothing downstream of the manifest (`publish_arch_repo`, `deploy_fly`, `deploy_render`, `sync_aur_docker`) consumes one either.
  Each run publishes a job summary naming the platform digest, scanner and database versions, and every CRITICAL / HIGH finding with its package, installed version, fixed version, CVE, and severity; the raw JSON, the SARIF, and a machine-readable verdict are retained as artifacts and the SARIF is uploaded to code scanning under a per-image category.
  The gate ships **report-only**: issue #6694 measured a 10-critical / 95-high backlog with Trivy 0.57.0, so arming it today would fail `main` on the first run.
  The enforcement threshold is a single default — the `fail-on` input of `.github/actions/trivy-image-scan` — that a maintainer moves `off → critical → high` once the backlog is remediated, and neither workflow overrides it.
  Nothing is suppressed to achieve that: no `--ignore-unfixed` and no severity filtering, so the report is complete either way and only the pass/fail decision is narrowed to fixable findings.
  A vulnerability-database download failure is kept distinct from a clean scan — refreshed in its own retried step, with the scan then running `--skip-db-update` — so a scanner outage fails loudly instead of passing as a finding-free image (#6694, #6712) (@houko)
- Add a managed configuration mode so a deployment can own `config.toml` instead of treating it as application state.
  `LIBREFANG_CONFIG_PATH` relocates the file — useful on its own, for a Compose bind mount or a ConfigMap mounted outside `LIBREFANG_HOME` — and `LIBREFANG_CONFIG_MODE=managed` locks it.
  The two are deliberately independent: relocating a file is not a statement about who owns it, and inferring the lock from the path would hand a read-only dashboard to an operator who only wanted the file somewhere else.
  The mode is read from the process environment and never from the config file, so a write through the API cannot unlock the very file it is being refused access to.
  When managed mode is active, every API surface that persists deployment configuration answers `423 Locked` with `{"code": "config_managed", "source": "<path>"}` and leaves the file untouched — enforcement lives in the handlers rather than relying on a read-only mount, because a filesystem `EACCES` surfaces as a 500 with an errno and tells an operator nothing about why.
  `GET /api/config/status` reports the mode, the source path, writability, a SHA-256 over the file's bytes, and its last-modified time, so the dashboard can present managed settings as read-only from server-supplied metadata rather than by attempting a save and reading the refusal back.
  Boot-time schema migration no longer tries to write the migrated config back when the file is managed; it logs a single targeted warning instead.
  That write previously failed against a read-only mount with nothing but a `warn!`, so the migration re-ran silently on every boot forever.
  Mutable mode remains the default and is unchanged (#6695, #6717) (@houko)
- Polish (pl) is now a supported UI language across the dashboard SPA, the backend Fluent error catalogue, and the webchat widget.
  The channel bridge also emits a Polish failure suffix for tool-failure progress lines.
  (#6696) (@leszek3737)
- Opt-in live model discovery for custom OpenAI-compatible providers, via a `discover_models` flag on the provider and a toggle in the dashboard's Add / Configure Provider dialogs.
  Discovery was gated on a hard-coded id allowlist (`ollama | vllm | lmstudio | lemonade`), so a self-hosted endpoint registered under any other id was never probed: its model list stayed empty forever and the only recourse was to register every model by hand, or to squat the built-in `vllm` id and override its base URL.
  A provider that opts in joins exactly the paths a built-in local one already walks — the 60-second probe loop, the `POST /api/providers/{name}/test` refresh, and the live-model filter on `/api/models`.
  The predicate ORs the flag with the id check rather than replacing it, so the built-in ids keep discovering regardless of the flag and an existing install sees no change.
  `PUT /api/providers/{name}/discovery` toggles it and persists the value into the provider's own TOML, so the opt-in survives a restart — for a provider you created, which is the case the feature exists for; on a registry-shipped file the boot-time sync still reverts any local edit (#6702, #6714) (@houko)
- Run the Python SDK test suite in CI.
  `sdk/python/tests/` held roughly 1900 pytest cases covering the HTTP client and every stdlib-only sidecar channel adapter — slack, discord, telegram, mastodon and the rest — and no workflow ran a single one of them, so the production code path for every sidecar channel shipped with CI fully green regardless of what broke.
  The `sdk/` prefix was already routed to the Rust lane, but only as an openapi codegen drift guard, which runs cargo and never pytest.
  The new lane installs the package with its `dev` extra and runs the suite on any `sdk/python/**` change, in under a minute (#6741) (@houko)
- Teach a task-board trigger to fire on unowned work via `pattern = { task_posted = { assignee_match = "unassigned" } }`.
  Previously the only options were "every posted task" or a specific agent, so an agent that should pick up whatever nobody has claimed had to match everything and filter in the prompt.
  The keyword matches both spellings of unowned that reach the event — an absent assignee and the empty string — because neither the `task_post` tool nor `POST /api/tasks` normalises the field, while both do reject an empty title and description.
  A client that sends an empty assignee means "nobody", and a filter that only understood the absent form would silently ignore it.
  (#6742) (@houko)
- `media_transcribe` can now transcribe a recording in bounded windows and write the transcript to a workspace file, which is what a recording longer than a few minutes needs to reach an agent at all.
  Previously the tool transcribed whole files and returned the transcript inline, so two limits unrelated to file size decided how long a usable recording could be: a single transcription request is bounded by a wall-clock timeout that does not scale with the input, and the kernel spills any tool result over `[tool_results] spill_threshold_bytes` (16 KB by default) to the artifact store and hands the agent a stub instead.
  Both are reached around ten minutes of speech, at roughly 2.5 MB of extracted audio — far below `MAX_AUDIO_BYTES`, and further still below `MAX_VIDEO_BYTES`, so no size limit is anywhere near being involved.
  `start_sec` and `max_secs` bound the request to one window and the response carries `has_more` / `next_start_sec` to walk the rest; `out_path` writes the transcript as UTF-8 and returns only the path, byte count, sha256 and a 200-character preview, following the contract `web_fetch_to_file` already established.
  Windows starting at `0` begin a new file and later windows append, so repeated calls assemble one transcript without any of it passing through the agent's context.
  Both mechanisms are needed rather than either alone: window size varies with how much was said, so a fixed window straddles the spill threshold instead of staying under it, and `out_path` is what makes the outcome independent of that.
  Callers advance by the produced window length rather than the requested one, read back from the Ogg granule position — a seek lands on a keyframe and a window overlapping the end of the recording is short, so an assumed edge drifts and eventually skips audio.
  Consecutive windows are separated by a newline: a boundary lands mid-sentence by design and each window's transcript arrives trimmed, so concatenating them directly would fuse the last word of one window to the first of the next at every boundary.
  A call that names neither window field keeps its previous behaviour exactly, including adding no ffmpeg pass. (#6748, #6773) (@nevgenov)

### Fixed

- Escape every TOML control character when the dashboard serializes agent manifest strings, preserving carriage returns, tabs, and other control bytes as valid round-trippable TOML instead of producing a manifest the daemon cannot parse. (@TechWizard9999)
- `browser_read_page` no longer drops the destination of every link nested inside a list item, and no longer returns a single card from a feed or search-results page.
  The extraction script's `li` branch flattened the item to `textContent` and returned before descending, so a nested anchor reached the model as bare text with no URL — 1,100 of 1,723 anchors on the Rust Wikipedia article and 11 of 11 on a DuckDuckGo results page.
  Clicking the text was not a fallback for those: 408 of the 1,100 do not resolve to themselves under `browser_click`'s substring matcher, so there was neither a URL nor a working text handle.
  The branch now recurses and folds its children back onto one line, so a bullet still renders as a bullet and the links inside it keep their identity.
  Root selection used `querySelector('main, article, [role="main"], .content, #content')`, which returns the *first* match — on a page built from sibling `<article>` cards that is one card, measured at 13.7% of a DuckDuckGo results page.
  Selection now climbs to the ancestor holding repeated sibling `article` elements, the way Readability resolves the same shape by walking to the common ancestor of its close-scoring candidates.
  Selection tests a node's direct children rather than everything below it: at least three of them must each carry an article of their own, so an ordinary post page whose "related posts" widget is built out of `article` keeps selecting the post instead of widening to the document and taking the widget with it.
  The tree is searched rather than climbed from the first article's ancestors, since the container is not always an ancestor of whichever article comes first — a featured card above a grid puts that article in a branch the grid does not sit under.
  A candidate that contains another loses to it, so a feed flanked by single-article widgets selects the feed rather than the ancestor holding all three, and between candidates that do not contain one another the one with the most article-carrying children wins, so a small widget nested deeper than the feed does not beat it for being deeper.
  A page with a `main` or `[role="main"]` landmark is unaffected, and a page with a single `article` still selects it (#6624, #6745) (@nevgenov)
- Fix four ways the release pipeline mishandled a large changelog, all of which fired on v2026.7.31 and left the release stuck.
  `cargo xtask release` passed the whole changelog section as the PR body, which GitHub rejects over 65,536 characters with `GraphQL: Body is too long` — the version bump committed and pushed, but no PR opened, so `tag_on_merge` never got the `<!-- release-tag: -->` marker it tags from and the release simply stopped.
  The body is now capped with a prefix-preserving truncation (the marker sits at position 0 and the Highlights lead, so a tail cut drops only the least-important bullets), cut on a character boundary, closing a code fence the kept prefix left open, and handed to `gh` via `--body-file` rather than an argv entry.
  `release.yml` fed the same section to `gh release create --notes-file`, where the API's own 125,000-character ceiling would have failed the job *after* the tag was already pushed; it now truncates the same way.
  Separately, `tauri-action` was invoked with `tagName`, whose `getOrCreateRelease` path PATCHes an already-published release with whatever `releaseBody` holds — and the desktop matrix passes none, so every desktop build overwrote the release notes with an empty string.
  That is why v2026.7.21 and v2026.7.27 both shipped with a blank body despite `create_release` writing one correctly.
  Both `release.yml` and `release-desktop.yml` now address the release by `releaseId`, which only uploads assets and leaves the notes alone.
  `release-notify.yml` bounded its Discord announcement by line count (`head -20`) but not by length, and a single bullet here runs past 1,800 characters, so the webhook would answer 400 `Must be 2000 or fewer in length`; it now truncates to a character budget that leaves room for the build-status block.
  The release commit also stages `xtask/baselines/` now: the run regenerates the schema baselines just before committing, so staging `openapi.json` without them left a drifted `openapi.sha256` in the working tree that would fail `schema-check check` on every subsequent PR.
  (#6689) (@houko)
- Fix the two `xtask` changelog tests that run against the repo's own `CHANGELOG.md` failing on any release branch, which blocked the v2026.7.31 release PR (#6688) on a state the release flow itself creates.
  `cargo xtask release` drains `## [Unreleased]` into the dated section it cuts, so on a `chore/bump-version-*` branch the section is empty — and `drains_the_repos_own_unreleased_section_without_tripping_the_guard` opens by asserting it is not, while `folds_into_the_repos_own_changelog` asserted a `### ` heading count that only holds when `### Changed` already exists.
  Both now read through a helper that reconstitutes the pre-release shape by hoisting the newest dated section back into `[Unreleased]`, so the real-file coverage survives on release branches rather than being skipped there, and the subsection assertion is a delta that permits exactly the one heading the fold may legitimately create.
  Doing that surfaced a second, older defect in the same two tests: they checked headings with `str::contains` / `str::matches`, which count substrings anywhere on a line, while the `awk` extractor they mirror anchors at column 0.
  A curated bullet that quotes a heading in its prose — the #6628 entry says "appended its bullet to the single `## [Unreleased]` section" on an indented continuation line — therefore read as a boundary overrun and as 19 `[Unreleased]` headings.
  Both checks are now line-anchored, matching the extractor.
  This had never fired because the assertions had only ever run against an empty `[Unreleased]`.
  (#6690) (@houko)
- Fix `Sign Release Artifacts` failing every release since #6677, which shipped debug symbols as their own assets without updating the sibling-count guard that assumed one `.sha256` per platform target.
  The guard exists to catch matrix drift in either direction and is deliberately an equality, so the four new `librefang-<target>-debug-symbols.tar.gz.sha256` files read as four extra platforms: v2026.7.31, the first release cut after #6677 landed, failed with `expected exactly 12 .sha256 siblings, got 16` after every artifact had already been built and published.
  The two kinds of sibling are now counted separately, because only one of them is one-per-matrix-target.
  Platform binaries keep the strict `= 12` equality that makes a dropped or added target stop the release loudly.
  Debug symbols get a `2..4` range instead, matching how they are produced: `cli_mac` fails outright when its `.dSYM` is missing, so both macOS targets are guaranteed, while the cross-compiled `cli_linux` targets only warn when the `.dwp` is absent — a count below 2 means the macOS hard-failure path did not hold and is worth stopping for, and a count of 2 or 3 emits a warning rather than taking a release down over a diagnostic aid.
  The manifest itself is unchanged: the download loop and `ls *.sha256` still read the full asset list, so every hash including the debug-symbols ones stays in `SHA256SUMS` and under the cosign signature.
  (#6691) (@houko)
- Always offer the API-key field for a provider that declares `key_required = false`, instead of hiding it.
  The flag says whether a key is *mandatory*, not whether one is accepted: every built-in local provider (`ollama`, `vllm`, `lmstudio`, `lemonade`) declares it false, yet a self-hosted vLLM or an Ollama behind a reverse proxy answers 401 without one — and the runtime has always forwarded whatever key is stored as `Authorization: Bearer`.
  Hiding the input made those servers impossible to configure from the dashboard even though the daemon would have used the key, and the onboarding wizard dropped a typed key on the same reasoning.
  The provider list now also reports `key_present`, so a keyless provider that does have a key stored offers "replace" and "remove" rather than pretending none exists — `auth_status` cannot carry that, since it collapses to `not_required` either way.
  The registry conflict error stops pointing at `?allow_overwrite=true`, a query parameter no UI surface sends, and names the endpoints that actually edit a provider (#6703, #6714) (@houko)
- Open external links from the desktop app in the user's real browser instead of silently discarding them.
  The Tauri window registered no new-window handler, and wry connects WebKitGTK's `create` signal — plus the WKWebView and WebView2 equivalents — only when one exists, so every `target="_blank"` anchor and `window.open()` call in the dashboard died on arrival inside the desktop app: the EveryAPI partner panel, marketplace and plugin links, the skill-workshop PR link, and the command palette's registry entries all did nothing on click, and right-clicking a link and choosing "open link" did nothing either.
  New-window requests are now handed to the OS default handler and the in-app window is denied, with the scheme restricted to `http` / `https` / `mailto` so that a `file:` or `javascript:` target coming from agent output or a server-controlled catalogue is never forwarded to the shell (#6706, #6711) (@houko)
- Manual schedule runs now deliver successful agent and workflow output through configured primary and fan-out targets, matching timed cron fires instead of returning output only to the API caller (#6708) (@Kvitral)
- A task assigned to an agent now wakes that agent, instead of reaching it only when an operator had separately registered a matching `task_posted` trigger.
  Nothing in the kernel ever created such a trigger, so delivery was entirely operator-supplied: with none declared — or with one deleted, lost, or never added for a newly onboarded worker — an addressed task sat `pending` indefinitely and no log line said so.
  The kernel now synthesizes the wake itself as one more entry in the dispatch list, so it inherits the existing trigger lane, per-agent semaphore, per-fire timeout, ordering and cycle guard, and persists nothing: no record appears in `trigger_jobs.json` or `trigger list`, and the new `[task_board] assignee_wake` knob (default `true`, per-agent override on the manifest) fully reverts it.
  A stored trigger that can currently fire for the assignee still owns delivery, so an operator's prompt, cooldown, session mode and workflow routing are untouched and no agent is woken twice; a trigger that is disabled or has exhausted `max_fires` is treated as a gap to fill rather than as a decision to stay silent, since a dead record is indistinguishable from a lost one and that ambiguity is what the outage was made of.
  Only agents that `task_claim` actually reaches are woken, which leaves installations whose board is drained by an external claimer on the agent's behalf exactly as they were: withholding the tool through any of the three mechanisms the runtime honours — a `capabilities.tools` list without it, a narrowing `tool_allowlist`, or a `tool_blocklist` entry — also withholds the wake, while an agent that declares nothing at all is unrestricted and is woken.
  Four diagnostics now cover the ways delivery can still break — an assignee that resolves to no registered agent, a wake switched off with no trigger to take over, an assignee that cannot claim, and a trigger that exists but can no longer fire.
  `[task_board]` is also reclassified from restart-required to no-op in the config-reload plan and its documentation table: the sweeper has always re-read its three knobs on every tick, so the promised restart was never required.
  Delivery no longer depends on the event surviving at all: the task-board sweeper — already a reconciler, since `task_reset_stuck` reacts to task state rather than to any event — gains a second rule of the same shape, waking the assignee of anything still `pending` past `[task_board] pending_grace_secs`.
  That makes a dropped event a latency question instead of a lost task, which matters because a trigger cooldown discards events for distinct subjects rather than deferring them (#6756), and an event-driven wake can only ever be as reliable as the event.
  The reconcile deliberately does not consult the trigger-coverage check the event path uses: a task still pending past the grace window is evidence that whatever was configured did not deliver it, whatever the configuration says.
  It rate-limits per assignee rather than per task, since the wake prompt is drain-style, and backs off exponentially to `wake_backoff_max_secs` whenever a wake leaves the pending set unchanged, so an agent that cannot make progress is not woken on every tick. (#6744) (@nevgenov)
- Restore the Windows test lane, which had aborted on every push to `main` since #6711 and taken `CI Gate` down with it, so for three days no PR merged with a Windows signal behind it.
  `librefang-desktop`'s test binary links on Windows but cannot be loaded — the process dies at start with `0xc0000139` (`STATUS_ENTRYPOINT_NOT_FOUND`) — and nextest executes every test binary with `--list` to enumerate its tests, so the lane died before running a single one.
  The crate is now excluded from that lane's nextest invocation and built there link-only instead, which keeps the Windows compile+link coverage that nothing else in CI provides while its 11 platform-independent URL/scheme tests continue to run on Ubuntu, macOS and the unit-fast lane.
  The missing DLL export behind the abort is still unidentified, so this is a workaround carrying a note to remove it once the real cause is found.
  A green `main` run also no longer closes an open `main-red` issue unless a Rust test lane actually ran on that commit.
  The `changes` gate skips every Rust lane on a docs- or dependency-only push while the run still concludes `success`, which is how this one breakage came to be filed and auto-closed three times (#6716, #6721, #6729) before anyone noticed `main` had been red for days — nine of the last sixteen green `main` runs would have closed it.
  The failure notice now names the head commit and the run URL without asserting that the commit caused the failure, because three consecutive filings told readers to revert an unrelated openrouter model-snapshot PR for a breakage introduced days earlier.
  (#6735) (@houko)
- Close the three provider routes that managed configuration mode left able to rewrite `config.toml`, so `LIBREFANG_CONFIG_MODE=managed` now enforces what #6717 documented rather than most of it.
  Setting a provider key, pointing a provider at a different base URL, or switching the default provider each persisted into the deployment-owned file — `[default_model]`, `[provider_urls]`, `[provider_proxy_urls]` — and answered `200`, so an operator whose configuration comes from a ConfigMap could silently drift the running daemon away from the manifest and lose the change on the next rollout with nothing in the response to suggest anything had gone wrong.
  Setting a provider key is refused in full rather than only at its config write, because that write is conditional on live daemon state the caller cannot see: guarding it alone would accept or refuse the same request depending on timing, having already rewritten `secrets.env` in the refusing case.
  The operator-facing known-gaps list is corrected in the same pass — it named three files without their write sites and missed the sidecar-channel and init routes entirely, which meant anyone reading it to decide whether managed mode was a complete seal was reading a list assembled from the wrong evidence (#6737) (@houko)
- Stop two Slack formatting defects that made long replies unreadable or invisible.
  Runs of blank lines now collapse to the single blank line Slack uses as a paragraph separator; the Markdown converter mapped lines one-for-one, so a model that padded its answer turned a short reply into a wall of whitespace.
  The collapse runs while fenced code is masked as a single token, so code interiors keep their own blank lines.
  An interactive Block Kit reply longer than Slack's 3000-character per-section limit is now split across as many sections as it needs instead of being rejected wholesale and dropped with nothing but a log line, and the section count is budgeted against the 50-blocks-per-message cap so the buttons — the functional payload — are never the thing that gets dropped.
  The plain-text path was never affected: its chunker already emitted pieces under the limit (#6741) (@houko)
- Stop the Slack adapter leaving a permanent 👀 on messages the daemon never answers.
  The reaction was added the moment the adapter received a message, but `dispatch_message` has roughly two dozen `return` paths above the first adapter-visible lifecycle signal — mention-only group gating, per-user and per-channel rate limits, RBAC, command policy, slash commands the bridge handles itself — and none of them was visible to the adapter, so the mark stayed on a message no agent ever read.
  Both halves of the receipt now ride the turn lifecycle instead: 👀 on the `queued` phase, ✅ on `done`, ❌ on `error`.
  That closes every one of those paths structurally rather than one at a time, gives a failed turn a terminal state it previously never got, and honours the daemon's `clear_done_reaction` knob on Slack for the first time.
  Keying strictly on the triggering message's own id also deletes a "pick the first pending message in this channel" fallback that fired on every in-thread reply — the send hook looked up the thread root while the 👀 was tracked under the message's own timestamp, so the ✅ landed on an unrelated sibling, sometimes one of the leaked marks (#6741) (@houko)
- Report a mis-declared `channel_overrides.group_trigger_patterns` entry when an agent's manifest is accepted, because the usual mistake is invisible by construction and cost a reporter hours.
  Writing the natural `group_trigger_patterns = ["(?i)\bvivi\b"]` in a TOML basic string does not produce a word-boundary regex: `\b` is TOML's *backspace* escape, so the kernel receives `(?i)<U+0008>vivi<U+0008>`, and the regex crate accepts a bare control character as a verbatim literal.
  The pattern therefore compiles — the bridge's existing "invalid regex" error never fires — and then matches nothing, so the agent simply never answers to its own alias in group chats and every message is dropped as `mention_only_no_mention`.
  The new check names the offending codepoint and prescribes the fix (a TOML literal single-quoted string, or doubled backslashes), and runs at spawn, hand-role activation, on-disk hot-reload, `update_manifest` and boot-time restore from persistent storage, so an operator iterating on a broken alias sees it on the next reload rather than never.
  The restore path is the load-bearing one: it registers a persisted agent without going through the spawn path, so on any daemon past its first run it is the only route the diagnostic could fire on at boot.
  It warns and never rejects: an unreachable alias is a typo, and failing the spawn would turn a cosmetic mistake into a missing agent.
  The bridge's own compile path could not serve this purpose — it is lazy, memoised per distinct pattern set, and does not consider the control-character case an error at all.
  The channels and agent-overrides docs gain the escaping rule and the previously undocumented `group_trigger_patterns` field, and no longer claim `MentionOnly` is what an unset `group_policy` does, which #6445 made false.
  (#6742) (@houko)
- Make a superseded agent turn visible in the log instead of discarding it at `debug!` level, below the default filter.
  When a newer message arrives for the same `(agent, session)` the in-flight turn is aborted and produces no reply at all, and in a group channel the error text is suppressed too, so the only symptom an operator ever saw was a bot that silently ignored them.
  The abort key matters more than it looks: for a group the session spans the whole **channel**, not the thread, so a message from any user in any thread preempts whatever turn is running for that agent there.
  The warning now names that mechanism and reports how long the discarded turn had been running, and the channel bridge separately records that the aborted turn emits nothing.
  Logging only — the supersede policy itself is unchanged.
  (#6742) (@houko)
- Bound nested workflow runs by the inter-agent call-depth quota, which the `workflow_run` tool had been bypassing entirely.
  Running a workflow executed it inline on the calling agent's task and each step nested a complete agent turn, but nothing counted that nesting — so an agent whose workflow step targets an agent that runs a workflow again recursed with no bound other than the wall-clock `triggers.max_workflow_secs`, long after the tokio worker's stack had run out.
  Workflow nesting is now charged to the same `max_agent_call_depth` budget that `agent_send` hops already use, because `A --agent_send--> B --workflow--> C` stacks real agent turns exactly the way `A -> B -> C` does and one operator knob should cap both.
  A run entered too deep is refused as a policy error before the run record is created, so a capped chain reports the quota instead of leaving an orphan `Pending` run behind.
  Two hops between the kernel and the agent were flattening that refusal into a generic server error, so the agent was told its workflow had crashed rather than that it had hit a limit.
  It now arrives as a permission denial, which also stops a capped agent from losing its whole turn to a repeated-failure abort.
  The daemon and the desktop app's embedded server also raise their tokio worker stack from tokio's 2 MiB default to 8 MiB: an agent turn is a chain of very large futures and a nested turn restacks it, so overflowing it aborted the whole process — with only two workers that took the HTTP API and every cron job down together.
  The CLI TUI's in-process kernel mode runs the same turn chain on its own long-lived runtime and on the dedicated thread that streams a turn's events, so both get the same 8 MiB stack.
  The stack change is headroom rather than a bound, and neither change is proven to be the cause of the crash reported in #6659, whose original report is unrecoverable; the next occurrence on the larger stack is what will tell unbounded recursion apart from bounded depth with fat frames.
  (#6743) (@houko)
- A sub-list nested inside a list item keeps its own bullets, indented, rather than being folded into its parent's line.
  Folding a list item's children onto one line is what lets a bullet still read as a bullet, but a nested `li` has already emitted its own `- ` by the time the outer one folds, so the whole sub-list collapsed into the parent as `- Fruits - Apple - Banana` — markers mid-sentence that read as list items, or as a numeric range on text like `- Price - 5 - 10`.
  An `article` also now earns a blank line the way `section` and `div` already do, so entries on a feed page that carry no heading of their own stay distinct instead of running together into one block (#6624, #6745) (@nevgenov)
- Image generation works again against OpenAI's `gpt-image-*` models.
  The request always carried `response_format`, which that family rejects with `400 Unknown parameter`, so every generation against those models failed while DALL-E was unaffected.
  The parameter is now sent only to models that accept it, and unrecognised model names keep the previous behaviour so third-party OpenAI-compatible endpoints are unchanged.
  (#6750) (@houko)
- The media integration tests no longer depend on the developer's shell lacking provider credentials.
  With an API key exported, tests asserting the missing-key path stopped exercising it and instead made real, billable calls — one generated an mp3 through the live OpenAI TTS endpoint while asserting that no provider was configured.
  The harness now clears every credential variable the media drivers read, and a dedicated test fails if that ever stops happening.
  A unit test that resolved a provider before reading its input file had the same dependency and could have reached a live transcription request; its input path is now guaranteed absent.
  (#6750) (@houko)
- Transcription of `.mp4` and `.mov` recordings works again instead of silently returning nothing.
  The audio track was extracted by piping the container to ffmpeg, but these formats keep their index at the end of the file and demuxing it requires seeking backwards, which a pipe cannot do — so the extraction produced a valid container carrying no audio at all, and whatever the transcription provider made of a soundless file was what the operator saw.
  Because ffmpeg reports success in this case, neither existing check noticed.
  The input is now staged to a scratch file so it can be seeked, and a stream that arrives without audio is rejected outright rather than uploaded.
  `.mkv` and `.avi` were never affected, being streamable formats.
  (#6751) (@houko)
- A trigger's cooldown no longer discards an event because a *different* event happened a moment earlier.
  The window was keyed on the trigger alone, so it could not tell "the same thing fired twice" from "two things happened a second apart", and the second was dropped rather than delayed — with nothing to re-announce it, since `evaluate_with_resolver` produced no match at all and `fire_count` never moved.
  On a task board that meant a completed task's notification vanished, or worse, a posted task's wake did, which is how work went missing while the log said nothing above `debug`.
  The window is now scoped to what the event is about — the task id for the three task-board patterns, the memory key for `MemoryUpdate` and `MemoryKeyPattern`, the agent for `AgentSpawned` and `AgentTerminated` — so two distinct subjects arriving inside one window are two windows, while a repeat of the same subject is suppressed exactly as before.
  Patterns that name a category rather than a transition (`All`, `System`, `SystemKeyword`, `Lifecycle`, `ContentMatch`) keep the trigger-wide window they have today: they match streams whose subjects are nearly always distinct, so keying on the subject would turn "at most once per window" into "once per event" for a trigger whose bounded firehose is the point.
  Per-subject windows live in memory and are pruned once they can no longer suppress anything, so `last_fired_at` on disk keeps meaning "when this trigger last fired"; a subject-scoped trigger therefore starts a restart with no window at all, including for a subject it fired on moments earlier, which trades an extra delivery against growing `trigger_jobs.json` by one entry per subject ever seen.
  Worth knowing if you pair `cooldown_secs` with `max_fires` on a scoped pattern: the trigger now fires once per subject, so a burst of distinct subjects consumes the fire budget as fast as they arrive rather than at one per window, and a trigger that exhausts `max_fires` disables itself.
  The CLI help and the trigger/config documentation described the window as per-trigger and have been corrected alongside. (#6918) (@nevgenov)
- The public Rust channel message splitter now always consumes at least one complete character, including when an incomplete HTML entity begins a chunk or the byte limit falls inside a multi-byte UTF-8 character.
  Custom Rust channel consumers can no longer enter a non-progressing split loop on those inputs (#6777) (@houko)
- Proactive-memory extraction now moves its prompt-size cutoff to a valid UTF-8
  boundary before searching for a newline or truncating. Long conversations that
  place a CJK character, emoji, or other multi-byte character across the 8,000-byte
  boundary no longer abort automatic memory extraction with a slicing panic
  (#6778) (@houko)
- Session stream garbage collection now retains broadcast channels while a turn forwarder is active, so reconnecting and late-attaching clients continue receiving in-flight events instead of joining an orphan replacement channel (#6785) (@houko)
- Agent identity persistence now remains blocked after an existing registry file fails to load, preserving recoverable on-disk mappings instead of replacing them with an empty fallback snapshot on the next mutation (#6787) (@houko)
- Sticky assistant routing now reads the same sender- and thread-scoped cache keys it writes, while `explicit_only` channels remain on their configured agent when no route has been explicitly cached instead of invoking classification (#6788) (@houko)
- Generate release articles with working CHANGELOG anchors, safe validated tag URLs, fence-aware section extraction, and opt-in replacement of existing hand-edited output. (@houko)
- Make the live channel-progress smoke fail when the kernel emits no tool event, supervise and clean up its foreground daemon reliably, and safely construct its dedicated test-agent manifest. (@houko)
- Repair the Go SDK streaming example so it compiles, validates dynamic agent IDs instead of panicking, and reports stream error events instead of silently succeeding. (@houko)
- Package the real `librefang` Python module tree in legacy setuptools builds, with distribution name and version metadata kept in sync with `pyproject.toml`. (@houko)
- Keep the website GitHub statistics section stable when optional translations change, and cancel its in-flight statistics requests when the section unmounts. (@houko)
- Restore registry responses after an empty or expired worker cache by routing inline refreshes through the repository synchronization path instead of calling a removed function. (@houko)
- Restore `xtask` builds on the workspace Rust 1.94.1 MSRV by keeping its `sysinfo` dependency on the compatible 0.38 release line. (@houko)
- Make the `xtask` license fallback inspect the full Rust dependency graph and evaluate SPDX AND/OR expressions correctly instead of silently skipping third-party crates or matching license-name substrings. (@houko)
- Keep Dashboard string-map edits from replaying parent change callbacks under React Strict Mode. (#6799) (@houko)
- Preserve Dashboard struct-list expansion and focus while editing valid JSON values. (#6800) (@houko)
- Keep Dashboard confirmation dialogs open until asynchronous actions succeed, with retry support after failures. (#6801) (@houko)
- Preserve large resource quotas when agent manifests pass through the Dashboard visual editor. (#6802) (@houko)
- Prevent an unmounting Dashboard drawer from closing a newer drawer that has taken over the shared slot. (#6803) (@houko)
- Keep empty and incomplete number-map edits as local drafts until they become valid numbers, and restore the last committed value when an invalid draft loses focus. (@TechWizard9999)
- Keep an empty structured-list textarea as an uncommitted JSON draft and restore its last valid item on blur instead of silently replacing that item with an empty object. (@TechWizard9999)
- Enforce `xtask license-check --deny` against Rust dependency metadata even when cargo-deny is installed, while retaining the repository's cargo-deny policy as the first gate (#6807) (@houko)
- Make `xtask license-check --deny` match denied SPDX license ids case-insensitively.
  The denied-list comparison used exact string equality against the canonical SPDX id, so a custom `--deny` entry with different casing than the canonical form (e.g. `gpl-3.0-only` vs `GPL-3.0-only`) silently failed to match and let the license through (#6807) (@houko)
- Enforce denied SPDX licenses for web dependencies from pnpm's JSON report, including Commons Clause rejection and fail-closed command or report errors, instead of printing a truncated report and always succeeding (#6808) (@houko)
- Stop RL exporter tests from mutating the process-wide environment while exercising secret indirection and the public SSRF dispatch. (@houko)
  The tests previously called `set_var` and `remove_var` while Rust's test harness was free to run other cases on parallel threads, making outcomes dependent on shared process state and creating a future Rust 2024 safety blocker.
  Production still resolves configured secret names through `std::env::var`; the crate-private dispatcher now accepts the lookup function so tests can supply deterministic values and missing-variable errors without touching the real environment.
- Reject non-object `params` on known Python sidecar commands as a recoverable protocol error. (@houko)
  Truthy arrays, strings, booleans, or numbers previously escaped `parse_command` as `AttributeError`, killed the reader task, and left the sidecar waiting forever instead of reporting the malformed frame and processing the next command.
  Unknown future command methods retain their raw parameter shape for forward compatibility.
- Stop and restart Python sidecars when their command reader encounters an unexpected fatal error. (@houko)
  An exception from the stdin source, parser, or protocol-error emitter previously killed only the reader task while the main runtime waited forever, leaving a live process that could no longer receive commands or shutdown.
  The runtime now logs the traceback, signals cleanup, raises a cause-preserving `ReaderCrashed`, and maps it to a nonzero stdio-process exit so the daemon supervisor can recover the adapter.
- Tie Fly deployment progress to the real request lifecycle instead of a cosmetic timer. (@houko)
  The deploy page previously marked one setup step complete every 1.5 seconds even while `/api/deploy` was still pending, and left that interval running when the form unmounted.
  Pending deployments now show only the request as active, mark completion only after a successful response, and abort plus ignore late results when the form unmounts.
- Always close generated Python SDK streaming responses when iteration ends. (@houko)
  The SSE generator previously leaked its HTTP response when it returned on `[DONE]`, the caller stopped iteration early, or a read or decode operation raised before the loop reached the trailing `close()` call.
  Response cleanup now lives in `finally`, covering normal EOF, protocol completion, generator close, and exceptional exits while preserving the original event and error semantics.
- Apply a bounded timeout to every generated Python SDK HTTP request.
  Both ordinary API calls and SSE stream setup previously called `urlopen` without a timeout, leaving connection establishment and stalled socket reads without an inactivity bound.
  Clients now use a 30-second default for both paths and accept a constructor-level timeout override for deployments that need a different network budget.
  A slow-to-respond server now raises the SDK's own `LibreFangError` instead of a bare `TimeoutError`, keeping the new failure mode inside the same error contract callers already rely on for connection and HTTP errors (#6823) (@houko)
- Wrap generated Python SDK connection failures in `LibreFangError`. (@houko)
  Both ordinary and streaming requests previously leaked `urllib.error.URLError` for failures such as DNS resolution errors, refused connections, and connection timeouts.
  Callers can now handle HTTP and connection-level API failures through the SDK's documented error type, with connection failures represented by status `0` and an empty response body.
- Preserve split UTF-8 characters in generated Python SDK streams. (@houko)
  The SSE reader previously decoded each 4096-byte network chunk independently, so a multibyte character split across reads raised `UnicodeDecodeError` and aborted the stream.
  Streaming now buffers raw bytes and decodes only complete SSE lines, making text decoding independent of transport chunk boundaries.
- Report generated Go SDK stream body encoding failures. (@houko)
  The streaming helper previously discarded `json.Marshal` errors and continued with an empty request body, hiding unsupported values from callers.
  It now emits a status-`0` error event and closes the stream before constructing or sending an HTTP request.
- Handle generated Go SDK stream request-construction failures. (@houko)
  The streaming helper previously ignored `http.NewRequest` errors and dereferenced a nil request, allowing malformed methods or URLs to panic its goroutine and terminate the process.
  It now emits a status-`0` error event and closes the stream before accessing the invalid request.
- Decode email bodies with their declared MIME charset. (@houko)
  The IMAP email helper previously forced UTF-8 for multipart plain text, HTML fallback, and non-multipart bodies, silently dropping bytes from common encodings such as ISO-8859-1 and GB2312.
  Each body part now uses its `charset` parameter while retaining UTF-8 as the fallback for missing or unknown charset labels.
- Decode complete RFC 2047 email subjects. (@houko)
  The IMAP email helper previously decoded only the first subject segment, truncating mixed plain/encoded subjects and subjects composed of multiple encoded words.
  It now joins every decoded segment in order and retains a UTF-8 fallback for unknown charset labels.
- Guarantee IMAP session cleanup in the email reader. (@houko)
  The helper previously logged out only on selected success and handled-error paths, leaking the connection when an unexpected exception occurred after login.
  It now closes every constructed IMAP session through a non-masking cleanup path, including login failures and all post-login exits.
- Validate IMAP FETCH responses before parsing email bytes. (@houko)
  The email helper previously indexed the server response without checking its shape, producing opaque index/type errors or passing flag-only data into the MIME parser.
  Empty, truncated, non-tuple, and non-byte responses now fail with a clear malformed-response diagnostic.
- Escape sender values in IMAP email searches. (@houko)
  The helper previously interpolated the sender directly into a quoted SEARCH criterion, allowing quotes, backslashes, or line controls to alter or break the command structure.
  Quotes and backslashes are now escaped as IMAP quoted-string data, while CR, LF, and NUL are rejected before opening a connection.
- Surface generated Rust SDK stream transport failures. (@houko)
  The SSE reader previously stopped silently when a response body chunk returned an error, making truncated connections indistinguishable from clean stream completion.
  It now emits a status-`0` `stream error` event before closing the channel, while preserving any valid events received before the failure.
- Encode generated Rust SDK path parameters as URL segments. (@houko)
  Generated endpoints previously interpolated path values directly, so slashes, query/fragment delimiters, whitespace, and Unicode could change the request target or address a different resource.
  URLs are now assembled with `Url::path_segments_mut`, preserving base-path prefixes and percent-encoding each parameter as one segment; literal `.` and `..` segments fail closed instead of being normalized away.
- Bound generated Rust SDK stream buffering to 256 events. (@houko)
  Streaming previously used Tokio's unbounded channel, allowing a fast server and stalled consumer to grow memory without limit.
  The producer now awaits a bounded channel, applying transport backpressure and stopping promptly when the receiver is dropped. Stream methods consequently return `tokio::sync::mpsc::Receiver<Value>`; callers with explicit `UnboundedReceiver` annotations must update the annotation, while normal inferred `.recv()` usage is unchanged.
- Add default network timeouts to the generated Rust SDK.
  The default reqwest client previously had no connect timeout, and ordinary API requests could wait indefinitely for a server that accepted a connection but never responded.
  All requests now use a 10-second connect timeout, while non-streaming calls additionally use a 60-second total timeout; SSE bodies remain exempt from the total deadline so long-lived streams continue normally (#6836) (@houko)
- Fixed the generated JavaScript SDK dropping a final server-sent event when a stream ended without a trailing newline, the same defect class fixed for the Rust and Python SDKs in this release.
  `_stream` split incoming bytes on `\n` and only processed complete lines, so a clean EOF right after the last `data: ` line left it sitting unprocessed in the leftover buffer (#6837) (@houko)
- Fixed the generated Python SDK dropping a final server-sent event when a stream ended without a trailing newline, the same defect class fixed for the Rust SDK in this release.
  `_stream` split incoming bytes on `\n` and only processed complete lines, so a clean EOF right after the last `data: ` line left it sitting unprocessed in the leftover buffer.
  The trailing-buffer flush now decodes as strictly as the per-line decode in the main loop above it, instead of silently replacing truncated multi-byte UTF-8 with a `�` placeholder and yielding a `{"raw": ...}` event that hid the corruption (#6837) (@houko)
- Fixed the Rust SDK dropping a final server-sent event when a stream ended without a trailing newline. (#6837) (@houko)
- Added a Rust SDK constructor that accepts a configured `reqwest::Client`, enabling authenticated requests and other custom HTTP settings across all generated resources. (@houko)
- Updated the Rust SDK basic example to report unexpected API response shapes instead of silently displaying zero items. (@houko)
- Made the Rust SDK basic example honor `LIBREFANG_URL`, while retaining the local daemon URL as its default. (@houko)
- Reduced the Rust SDK's Tokio feature set to the runtime, synchronization, and macro capabilities it actually uses, avoiding unnecessary downstream feature unification. (@houko)
- Made the Rust SDK's reqwest TLS backend explicit and selectable: existing users retain default TLS, while downstream crates can choose rustls or disable TLS features. (@houko)
- Aligned the Rust SDK with the workspace's thiserror 2 dependency, avoiding duplicate major versions in monorepo builds. (@houko)
- Removed Tokio's multi-thread scheduler from the Rust sidecar SDK's published dependency features and moved its echo example to the current-thread runtime. (@houko)
- Added an explicit crates.io package allowlist for the Rust sidecar SDK so unrelated local files cannot enter published archives. (@houko)
- Declared the Rust sidecar SDK's tested serde, serde_json, and Tokio version floors instead of accepting untested early 1.x releases. (@houko)
- Simplified the Rust sidecar SDK quick-start imports so the minimal adapter example lists only the APIs it uses. (@houko)
- **Breaking:** Aligned Rust sidecar poll builder option IDs with the kernel's `u8` wire contract, preventing adapters from constructing out-of-range poll payloads. The Telegram sidecar now rejects out-of-range upstream option IDs at its translation boundary. (@houko)
- Documented the Rust sidecar SDK's deliberate fail-closed handling of missing required command fields and its compatibility difference from the legacy Python parser. (@houko)
- Avoided cloning the full JSON parameter tree while parsing known Rust sidecar commands. (@houko)
- Bounded Telegram streaming state with stale-entry eviction, concurrent-stream and per-stream buffer caps, and graceful-shutdown cleanup. (@houko)
- Rejected malformed Telegram update payloads that omit required response, update, or message identity fields instead of silently defaulting their IDs. (@houko)
- Warned in the Telegram dashboard schema that leaving `ALLOWED_USERS` empty permits all users. (@houko)
- Prevented Telegram's degenerate HTML chunking path from emitting chunks above the configured UTF-16 limit. (@houko)
- Escaped raw HTML metacharacters in Telegram sanitizer text nodes while preserving already-valid HTML entities. (@houko)
- Fixed the Python sidecar's Telegram HTML sanitizer emitting invalid crossed tags (e.g. `<b><i>x</b>` → `<b><i>x</b></i>`) when a closing tag matched an entry below the top of the open-tag stack.
  The sanitizer now closes every tag above (and including) the match, innermost first, matching the Rust sanitizer's stack-drain behavior. (#6856) (@houko)
- Prevented self-closing Telegram HTML tags from being emitted as literal `<tag/>` markup in the Python sidecar's sanitizer, matching the Rust sanitizer's fix.
  Telegram's HTML subset has no self-closing-tag syntax, so a literal `<tag/>` risked either an "Unclosed start tag" error from the Bot API or the tag staying open for the rest of the message; self-closing input is now rebuilt as a balanced `<tag></tag>` pair instead. (#6856) (@houko)
- Prevented self-closing Telegram HTML tags from wrapping all following text during sanitization. (#6856) (@houko)
- Rejected Telegram location payloads with missing or non-numeric coordinates instead of silently sending `(0, 0)`. (@houko)
- Prevented self-closing and void HTML tags from leaking into Telegram chunk carry state. (@houko)
- Enforced Telegram's UTF-16 chunk limit against the actual generated HTML close-tag suffix instead of relying only on a fixed reserve. (#6859) (@houko)
- Rendered every adjacent single-star italic run in Telegram messages instead of leaving alternate runs as literal Markdown. (@houko)
- Preserved Telegram HTML tags containing `>` inside quoted attribute values instead of truncating and corrupting them. (@houko)
- Added a complete Markdown-to-sanitized-and-chunked Telegram formatting helper and routed text sends through it. (@houko)
- Kept rendering content after an unclosed Telegram Markdown code fence instead of swallowing the remainder into one code block. (@houko)
- Restored Telegram inline-code placeholders in one linear pass instead of repeatedly rescanning and reallocating the whole message. (@houko)
- Honored delta-seconds from Telegram HTTP `Retry-After` headers in the Python sidecar's `sendMessage` / multipart-upload retry paths before falling back to the JSON body or default backoff, matching the Rust adapter's fix.
  `_extract_retry_after` previously only read `parameters.retry_after` from the JSON body, so a server that only set the HTTP header (and omitted the JSON field) fell straight through to the 2s default instead of honoring the server's requested delay.
  Also capped the retry sleep at `MAX_RETRY_AFTER_SECS` (300s): the Python retry paths had no cap at all, so a flood-wait response with an extreme `retry_after` would have slept for that entire duration instead of skipping the retry, unlike the Rust adapter. (#6866) (@houko)
- Honored delta-seconds from Telegram HTTP `Retry-After` headers before falling back to the JSON body or default backoff. (#6866) (@houko)
- Returned recoverable Telegram API errors if a retry loop ever exhausts instead of panicking and killing the sidecar. (#6867) (@houko)
- Scaled Telegram multipart upload timeouts with payload size so valid large media can complete on slower links. (#6868) (@houko)
- Accepted Telegram message IDs encoded as JSON integers as well as decimal strings for edit and delete commands. (#6869) (@houko)
- Rejected malformed non-object Telegram media-group entries instead of silently dropping them from the outgoing group.
  Also rejected a media-group item missing its required `url` field instead of sending Telegram an empty `media` value. (#6870) (@houko)
- Dropped Telegram callback events without chat context instead of routing them into an empty synthetic channel. (#6871) (@houko)
- Detected Telegram Ogg/Opus voice uploads from the Ogg page's actual first-packet offset instead of assuming a fixed header layout. (#6872) (@houko)
- Logged Telegram `getFile` failures before falling back to media placeholders, making persistent media degradation visible to operators (#6873) (@houko)
- Reported invalid Telegram channel and reaction message IDs consistently across typing, reaction, interactive, streaming, and ordinary send commands. (#6874) (@houko)
- Normalized emoji variation selectors consistently before mapping Telegram progress reactions. (#6875) (@houko)
- Preserved typed JSON decoding errors in the Telegram sidecar error source chain for diagnostics and downcasting. (#6876) (@houko)
- Removed the unnecessary `T: Default` bound from Telegram API response envelopes while preserving their default field values. (#6877) (@houko)
- Added the registry version required for publishing the Telegram sidecar's local SDK dependency once that SDK is available on crates.io. (#6878) (@houko)
- Expanded Telegram schema regressions to cover the type and visibility of every dashboard configuration field. (#6879) (@houko)
- Removed a per-link allocation from Telegram href scheme validation while preserving case-insensitive and UTF-8-safe checks. (#6881) (@houko)
- Grouped consecutive Telegram Markdown quote lines into one multi-line blockquote, matching the Python adapter. (#6882) (@houko)
- Logged Telegram typing-action and reaction-update API failures while preserving their best-effort command semantics. (#6883) (@houko)
- Made Telegram chunk progress derive solely from the newly selected input, preventing formatting carry from skewing boundary consumption. (#6885) (@houko)
- Logged dropped Telegram stream deltas and stream-end events whose stream ID has no active state, while preserving best-effort handling. (#6886) (@houko)
- Rejected malformed Telegram poll options and missing or out-of-range quiz answers before issuing a Bot API request.
  Also enforced the Bot API's question, option, and explanation length bounds locally so an oversize poll fails fast instead of a 400 from Telegram. (#6887) (@houko)
- Logged best-effort Telegram callback acknowledgment failures with control-safe callback and error details (#6891) (@houko)
- Fixed the `librefang-rl-export` test call sites that still passed `&Value` to `redact_metadata` after it became by-value, which broke `cargo check --all-targets` on the aarch64 lane and blocked every open PR behind a red CI Gate. (#6896) (@houko)
- Harden the discussion-to-issue and weekly-report workflows against partial failures and unsafe assumptions.
  The discussion backfill now serializes through a concurrency group, bounds every job with a timeout, and records per-discussion failures instead of continuing past them silently.
  The manual `/to-issue` command now requires an exact token match instead of a substring match, so a comment that merely contains that text can no longer trigger a promotion.
  The weekly report now fails closed on any command error, resolves the repository from `github.repository` instead of a hardcoded name, and surfaces Discord delivery failures instead of swallowing them (#6904) (@houko)
- Warn about duplicate Dashboard map keys and preserve compact struct-list JSON drafts until blur. (#6906) (@houko)
- Require approval before unrecognized channel senders can use network or tool-discovery capabilities. (#6908) (@houko)
- Simplify canonical agent identity registration and remove silent mutex-poison recovery. (#6910) (@houko)
- Proactive-memory extraction now keeps its kernel-handle slot usable when a thread panics while holding the lock, instead of silently ignoring all later handle reads and updates.
  Conversation prompt assembly also uses a preallocated buffer and writes each message directly, avoiding a temporary allocation per turn.
  (#6911) (@houko)
- Link understanding now compiles its URL extraction pattern once and shares it across messages, avoiding repeated regex parsing and allocation on the message processing path. (#6912) (@houko)
- Canvas sanitization now enforces its configured byte limit before appending each output fragment, preventing entity escaping from temporarily growing a rejected document several times beyond the limit. (#6913) (@houko)
- The dashboard agent editor now blocks periodic schedules without a cron expression and JSON-schema response formats whose schemas are empty, malformed, or cannot be represented faithfully in TOML.
  Validation errors automatically open their sections and are exposed to assistive technology.
  It also removes a redundant schedule parsing branch, clears duplicate tag submissions, and gives the stream-thinking toggle an accessible name.
  (#6914) (@houko)
- Release changelog generation now fails closed when git or GitHub metadata is incomplete, bounds external commands, rejects model-generated section headings, preserves the Unreleased section on a first release, and reuses compiled title patterns. (#6915) (@houko)
- Repository automation now preserves devcontainer build failures, cancels only superseded ignored-test PR runs, and pins first-party actions in the supply-chain audit. (#6916) (@houko)
- Session repair now removes prompt-injection markers after international text without corrupting Unicode byte boundaries. (#6917) (@houko)
- Harden trajectory export by using the existing audited SHA-256 dependency, preserving hexadecimal identifiers during blob redaction, respecting workspace path-component boundaries, and surfacing JSON serialization failures (#6920) (@houko)
- Reuse stable session-scoped files when loading inline history images, move their filesystem work off Tokio worker threads, keep empty-session response fields consistent, localize malformed session IDs, and enforce the documented 100 KiB tool-result cap in UTF-8 bytes (#6921) (@houko)
- Closed two holes in the AI-attribution guards that between them let the harness footer reach 285 PRs and issues unchallenged.
  Both layers matched only the "with" spelling of the generated-by line while the footer uses "by", so every check in front of it reported clean; both now match either verb plus the footer's own link-and-host shape, which leaves a genuine claude.ai artifact link alone.
  Nothing inspected a PR body at all — the existing rule reads only `git commit -m`, and the git-side hook cannot see a body that never enters a commit — so `gh pr`, `gh issue` and `gh release` bodies are now checked too, reading the file behind `--body-file` rather than only inline flags, since the convention mandates the file form.
  A third defect surfaced while pinning the corpus: the Python predicate required a space inside the product name where the shell hook allowed none, so two variants the shell hook's own corpus lists as must-block were waved through one layer up.
  `check-bash-rules.py` decides every PreToolUse verdict and had no test of any kind, which is how a one-word gap survived; it now has a mutation-checked corpus wired into the `githook-tests` CI job (#6936) (@houko)
- Bound the rendered MCP summary cache, which grew one entry per distinct allowlist combination for the lifetime of the daemon.
  Agent manifests control the allowlist, so a caller cycling through one-off combinations (or stale generations left behind by config reloads) could grow the cache without limit.
  The cache now caps at 256 distinct entries and clears wholesale before admitting a new key past that cap, while preserving current-generation cache hits and rendered summary content (#6939) (@houko)
- `atomic_write` fsynced the staged temp file before the rename but never synced the containing directory afterward, so the rename itself was not guaranteed durable.
  A crash between the rename syscall and the next unrelated fsync of that directory could still lose the update on some filesystems, even though the write looked atomic from the caller's side.
  On Unix, the parent directory is now fsynced after the rename so the new directory entry survives a crash (#6942) (@houko)
- Secret writes to `secrets.env` could report success while a staging-file `fsync` failure went unnoticed, or leave a 0600 secret-bearing staging file behind after a failed write or rename.
  The staging file now propagates `fsync` errors instead of discarding them, gets removed on any write or rename failure, and the parent directory is fsynced after the atomic rename on Unix so a completed write survives a crash immediately afterward (#6944) (@houko)
- Sidecar config writes used `fs::write` for the staging file, which never fsyncs, so a crash between the write and the rename could leave the renamed file pointing at stale or truncated data, and a rename failure left the staging file behind instead of being cleaned up.
  The staging file is now opened with `create_new`, fsynced before the rename, removed on any write or rename failure, and the parent directory is fsynced after a successful rename on Unix so a completed write survives a crash immediately afterward (#6945) (@houko)
- Skill secret writes staged to a fixed `.tmp` sibling name, so concurrent writers to the same `secrets.env` could clobber each other's staging file, and a write or sync failure left that 0600 secret-bearing staging file behind on disk.
  The staging file now gets a name unique per process and call, is removed on any write or sync failure, and the parent directory is fsynced after the atomic rename on Unix so a completed write survives a crash immediately afterward (#6947) (@houko)
- Cron script TOML writes opened their staging file with `File::create`, which truncates and silently reuses an existing file of the same name instead of failing loudly on a staging-name collision.
  The staging file is now opened with `create_new` so a collision surfaces as an error rather than being silently overwritten, and the parent directory is fsynced after the atomic rename on Unix so a completed write survives a crash immediately afterward (#6948) (@houko)
- Memory consolidation and the per-user spend ranking used `.filter_map(|r| r.ok()).collect()` over their SQLite row iterators, which silently dropped any row that failed to decode instead of surfacing the failure.
  A corrupted `agent_id` could make consolidation skip a tenant's memories with no error, and a corrupted usage row could make a user vanish from the spend ranking rather than showing up as a failed query.
  Both call sites now collect into `rusqlite::Result<Vec<_>>` and propagate the decode error (#6951) (@houko)
- Paginate the GitHub API queries in the issue-inactive and issue-pr-link workflows, which previously only read the first page of results.
  A repository with more than 100 open assigned issues could skip inactive-issue reminders for issues past the first page, and a repository with more than 100 open pull requests could have `has-pr` incorrectly stripped from an issue that a later-page PR still linked (#6959) (@houko)
- Restore Vite's default dev-server proxy error logging, which a custom logger and a set of no-op `error` handlers on the `/api` proxy, its outgoing request, and its incoming response were silently swallowing.
  A backend that was down or unreachable during `npm run dev` produced no diagnostic output at all, making the failure look like a hang instead of a connection error.
  The WebSocket (`ws: true`) and five-minute proxy timeout behavior are unchanged (#6965) (@houko)
- The audit trail's boot-time integrity check verified as intact even when a row failed to decode from SQLite, because the loader silently skipped the malformed row instead of treating the load as incomplete.
  `AuditLog::with_db` now records the first load error it hits — a bad connection, a failed query, or a row that fails to decode — and `verify_integrity` fails closed whenever one is present, so a partially loaded chain never reports as verified (#6968) (@houko)
- Fixed a race in `CronScheduler::add_job` where concurrent creators could each pass the global and per-agent job-limit checks before any of them inserted, letting the total job count exceed the configured cap.
  Capacity checks, validation, and insertion are now serialized on a dedicated lock so the whole add sequence is atomic (#6970) (@houko)
- `max_content_chars` now bounds the link table's opening line along with its entries, so the extraction stays inside the ceiling an operator set rather than overshooting it by that line's length.
  The budget summed the entries and stopped there, but the rendered block also opens with a line naming the marker form and the base origin, and that line reaches the model with the entries — 50,093 characters against a 50,000 cap on the Rust Wikipedia article, the 93 being that line for a 24-character origin.
  The test could not have caught it: it re-derived the table's cost in its own port and asserted against that same derivation, so the budget and the assertion agreed by construction whatever the renderer did.
  Every ported test in this module now asserts that the template still contains the rule it models, since a port is only evidence about the script while the two agree — and nothing else would have noticed the script and its port drifting apart.
  It now asserts against what `render_page_body` actually produces, which is the string that reaches the model (#6624, #6973) (@nevgenov)
- CLI commands that rewrite `config.toml`, channel configs, MCP server entries, and ChatGPT OAuth secrets used a plain truncating `fs::write`, so a crash or kill mid-write could leave a corrupt or empty file behind.
  These call sites now go through a shared `durable_atomic_write` helper that stages content in a unique sibling file, fsyncs it, and atomically replaces the target via `rename` on Unix or `MoveFileExW` on Windows, fsyncing the parent directory afterward on Unix so the replacement survives a crash.
  New secret files are created at 0600 and an existing file's permissions are now preserved exactly, including bits a restrictive process umask would otherwise silently strip from the creation mode (#6974) (@houko)
- The MCP migrator wrote synthesized `[[mcp_servers]]` configuration with a truncating write, so a crash or kill could leave `config.toml` empty or partial.
  The config is now staged, fsynced, and atomically published; existing Unix permissions are preserved, newly created config files use mode 0600, parent-directory sync failures after a successful publish are logged without misreporting the migration as skipped, and Windows publishes with write-through semantics. (#6975) (@houko)
- Serialized local skill installs (`POST /api/skills/install`) behind the same per-skill file lock already used by evolve and uninstall.
  Previously the handler checked destination existence, then copied the skill directory outside any lock, so two concurrent installs of the same skill could both pass the existence check and race to write into the same directory, and a failed loser's cleanup (`remove_dir_all`) could delete a winner's just-installed files.
  The existence check now happens after the lock is acquired, and cleanup on a failed copy only ever removes the failed copier's own attempt (#6977) (@houko)
- Add a global React Query `MutationCache` error fallback so a rejected mutation without its own `onError` handler now surfaces a localized toast instead of failing silently.
  Mutations that already register a mutation-specific `onError` are left untouched to avoid duplicate feedback (#6978) (@houko)
- The cron scheduler's final persistence attempt during kernel shutdown discarded its result with `let _ = …`, so a failed flush of execution state (a full disk, an unwritable data dir) left no trace anywhere.
  `run_cron_scheduler_loop` now logs a structured `warn!` with the underlying I/O error when the shutdown-time persist fails, while still letting shutdown proceed (#6979) (@houko)
- `PATCH /api/memory/config` read and wrote `config.toml` with untorn but non-atomic `std::fs::write`, so a crash mid-write could leave the file truncated, and two concurrent dashboard saves could interleave a read and a write and silently revert each other's change.
  The managed-mode guard now runs before the file is touched, the full read-modify-write-reload transaction is serialized under the shared config write lock, the read moved off the blocking thread, and the write goes through the durable atomic writer (temp file, fsync, rename, directory fsync) on the blocking pool (#6982) (@houko)
- Registry content creation raced on the no-overwrite check: two concurrent `POST /api/registry/content/{type}` calls for the same identifier could both observe an absent file and each write, silently discarding whichever write lost.
  The existence check and the write are now serialized under the same `config_write_lock` used by the other config-mutating endpoints, and the write itself goes through the fsync-based atomic writer instead of a plain `fs::write`.
  A rejected provider definition is now rolled back to its prior contents (rather than merely deleted), so a failed overwrite of an existing provider no longer leaves it missing (#6984) (@houko)
- `GET /api/sessions` ran its `count_sessions` and `list_sessions_paginated` SQLite calls directly on the async handler, so a large or contended sessions table could stall the Tokio worker thread and delay every other request being served by it.
  Both calls now execute together on `tokio::task::spawn_blocking`, and a query or blocking-task failure is now logged server-side instead of being silently discarded (#6986) (@houko)
- `POST /api/init` checked and wrote `config.toml` with unsynchronized blocking `std::fs` calls directly on the async handler, so two concurrent requests could race past the existence check and one write could clobber the other, and every call blocked an async worker thread on disk I/O.
  The existence check now uses async metadata, the write path serializes on the same `config_write_lock` used by the other config-mutating routes and rechecks existence after acquiring it, and directory creation plus the atomic config write both run on Tokio's blocking pool (#6988) (@houko)
- Added the five error-message translations missing from the Japanese Fluent locale (an agent invalid-sort key and four webhook error keys), preserving every Fluent interpolation variable used by the English source.
  Added a regression test asserting the Japanese locale covers every English error key so a newly introduced key can no longer ship without a translation (#6998) (@houko)
- Restored missing diacritics and inverted punctuation across the Spanish error-message locale (`válido`, `sesión`, `configuración`, `¿agente no encontrado?`, and similar), and corrected a few literal, unnatural phrasings alongside unit formatting for size limits.
  Added regression assertions for representative accented translations so a future edit cannot silently strip them again (#6999) (@houko)
- Restored missing diacritics across the French error-message locale (`déjà`, `échec`, `création`, `déclencheur`, and similar), and corrected unit-abbreviation typography for size limits.
  Added regression assertions for representative accented translations so a future edit cannot silently strip them again (#7000) (@houko)
- Proactive-memory lock recovery from a poisoned state is now logged for the runtime config lock and the decay/cleanup/counter-prune maintenance locks, instead of recovering silently.
  Config reads and writes, and background maintenance scheduling, remain usable after recovery (#7003) (@houko)
- Channel agent-router lock recovery from a poisoned state is now logged for both the binding list and the broadcast configuration, instead of recovering silently.
  Routing and broadcast resolution both remain usable after recovery (#7004) (@houko)
- A2A task-store lock recovery from a poisoned state is now logged for both the in-memory task map and the backing SQLite connection, instead of recovering silently.
  Task loading, persistence, lookup, and mutation all remain usable after recovery (#7006) (@houko)
- Audit-log lock recovery from a poisoned state is now logged with the specific state involved — entries, tip, chain anchor, or load-error — instead of recovering silently across every accessor.
  Recording, verification, and retention all continue to operate correctly after recovery, with the hash chain's integrity preserved (#7007) (@houko)
- Command lane read/write lock recovery from a poisoned state is now logged with the affected lane, instead of recovering silently.
  The lock's poison flag is cleared once the recovered state has been read out, so a single panic produces one diagnostic log line rather than a permanent per-access warning for the rest of the process (#7013) (@houko)
- Hand activation now logs when the activation mutex recovers from a poisoned lock, instead of recovering silently.
  The mutex only serializes the check-and-insert critical section and guards no data of its own, so recovering via `into_inner()` was already safe — the gap was visibility into a prior panic, not correctness.
  The recovery path also clears the mutex's poison flag once the inner state has been read out, so a single panic produces one diagnostic log line rather than a permanent per-call warning for the rest of the process, matching the fix already applied to `CommandQueue`'s locks (#7013).
  This brings `activate_with_id` in line with the existing `persist_lock` poison-recovery logging (#7028) (@houko)
- Recover the agent context cache after a mutex poisoning event instead of permanently disabling it.
  `get_cached` and `store_cached` used to give up silently once the lock was ever poisoned, which meant every future turn served no cached `context.md` and every write became a no-op for the remaining life of the process.
  The cache now recovers the poisoned guard and logs a warning so the corrupted synchronization state stays observable (#7029) (@houko)
- Recovered the checkpoint snapshot concurrency counter after mutex poisoning instead of panicking at snapshot entry or silently skipping the decrement in the cleanup guard.
  A panic while the counter lock was held used to either abort the current snapshot attempt outright or leave the permit accounting off by one forever, since the old cleanup path only decremented on `Ok`.
  The lock is now recovered via `into_inner()` and the poison flag cleared so the mutex stops re-poisoning every later lock attempt, and a regression test exercises the poison-then-recover-then-release sequence (#7030) (@houko)
- The stuck-task reset sweep silently dropped any `task_queue` row it could not decode from SQLite, so a single corrupt row (e.g. a non-numeric `retry_count`) caused the rest of that sweep's stuck tasks to be skipped with no error surfaced to the caller.
  `task_reset_stuck` now decodes the full candidate set before applying any reset update, so a row decode failure fails the sweep closed instead of silently reducing its coverage (#7031) (@houko)
- Session search (`SessionStore::search_sessions` / `search_sessions_paginated`) now propagates a row-decode failure from `sessions_fts` as an error instead of silently dropping the corrupt row and returning a partial result set.
  A single malformed row previously vanished from search results without a trace; the same failure is now surfaced to the caller so the underlying corruption gets noticed and investigated (#7032) (@houko)
- Group roster storage (`RosterStore::upsert`, `members`, `remove_member`, `member_count`) swallowed every SQLite pool-exhaustion and row-decode error, returning empty results or fixed defaults instead of failing.
  A corrupted `group_roster` row was silently dropped from `members()` rather than surfacing as a query failure, and a pool outage during `upsert` or `remove_member` looked identical to success to every caller.
  All four methods now return `LibreFangResult`, and the channel bridge and kernel handle boundaries propagate the error instead of discarding it (#7033) (@houko)
- `TraceStore::query`, `query_by_trace_id`, and `count` swallowed SQLite failures and poisoned-mutex errors, returning an empty list, `None`, or `0` indistinguishably from a genuine empty result.
  A corrupt row or a failing query on the hook-trace store was therefore reported to callers as "no traces found" rather than as a failure.
  These methods now return `rusqlite::Result`, and `GET /api/context-engine/traces/:trace_id` surfaces a scrubbed HTTP 500 instead of a false 404 when the store itself fails (#7034) (@houko)
- Approval audit queries used to swallow SQLite failures and return an empty list or a zero count, which looked identical to "no audit history exists" on the dashboard and in the duplicate-resolution helper used by channel bridges.
  `query_audit` and `audit_count` on `ApprovalManager` now return a typed result, the `/api/approvals/audit` route surfaces a scrubbed HTTP 500 on failure instead of a fabricated empty page, and the channel-bridge duplicate check logs a warning and falls back to its prior no-match behaviour rather than pretending the query succeeded (#7035) (@houko)
- `Path::parent()` yields `Some("")` rather than `None` for a bare relative filename, and three recently added atomic writers treated that empty-but-present case as an error.
  In the cron script writer the parent is opened for the post-rename directory fsync, so an empty parent would fail with ENOENT after the rename had already succeeded — reporting failure for a write that landed on disk.
  In the Skillhub and skill-evolution writers it only anchors the staging file beside the target, where an empty parent happened to work because the join and the rename both resolved against the process directory, making the same-directory invariant that keeps the rename atomic hold by accident.
  All three now resolve an empty parent to `.`, matching the API crate's atomic writer, which already handled it (#7036) (@houko)
- The shared metering budget snapshot (`MeteringEngine::budget_status`) silently converted a usage-store query failure into zero spend via `.unwrap_or(0.0)`, so a broken SQLite read looked identical to "no spend yet" everywhere the snapshot was consulted.
  `budget_status` now returns a `LibreFangResult<BudgetStatus>`; the `/api/budget`, `/api/budget` update, and `/api/system/health/detail` routes return a scrubbed HTTP 500 on failure instead of a fabricated zero-spend response, and the WebSocket `budget` command and channel-bridge budget reply now report an explicit "temporarily unavailable" message instead of misleading zero values (#7037) (@houko)
- Stop channel message dispatch when the recovery journal fails to persist an entry, instead of logging the failure and continuing as if the write-ahead record existed.
  `MessageJournal::record` now returns `true` only once the entry is durable and indexed, and both `dispatch_message` and `dispatch_with_blocks` abort with a user-facing retry notice on `false` rather than proceeding without crash-recovery coverage (#7040) (@houko)
- `GET /api/sessions` swallowed a failed `count_sessions` call with `.unwrap_or(0)` and both it and `GET /api/sessions/search` fell back to an empty `200 OK` page on a database error, so a broken sessions table looked identical to "no sessions yet" from the client's side.
  `search_sessions` also leaked the raw SQLite error string (e.g. table names) straight into the response body via `ApiErrorResponse::internal(error.to_string())`.
  Both handlers now propagate the failure as a scrubbed `500` with a generic message, logging the real error server-side with `tracing::error!`, consistent with the rest of the file (#7041) (@houko)
- Move dashboard archive extraction and installation off Tokio's async worker threads. (@xiaomo)
- Load WASM agent modules asynchronously so filesystem latency cannot block Tokio worker threads. (@xiaomo)
- Return an internal error when backup directory entries or metadata cannot be read instead of reporting a misleading empty or zero-sized backup list. (@xiaomo)
- Propagate malformed prompt-version, experiment, variant, and metrics rows instead of replacing invalid UUID, JSON, or timestamp fields with default data. (@xiaomo)
- Abort sidecar config and secrets.env read-modify-write operations when an existing file cannot be read instead of treating the failure as an empty file. (@xiaomo)
- Abort auto-dream lock acquisition when an existing lock file cannot be read instead of treating it as an unowned stale lock. (@xiaomo)
- Return a scrubbed server error when an extension install or uninstall cannot apply its on-disk MCP configuration, instead of reporting success against stale runtime state. (@xiaomo)
- Move agent-template directory and manifest reads off synchronous filesystem APIs, and surface corrupt or unreadable listings instead of returning an empty or incomplete template list. (@xiaomo)
- Make the sidecar configuration include check asynchronous and fail closed when the root or included configuration cannot be read or parsed, instead of continuing with a potentially shadowing write. (@xiaomo)
- Move agent identity-file writes, renames, canonicalization, and deletes off Tokio worker threads while preserving containment checks and atomic replacement. (@xiaomo)
- Read skill supporting files asynchronously with a real 256 KiB buffer limit, and surface canonicalization errors instead of disguising every filesystem failure as a missing file. (@xiaomo)
- Fail closed when reading a hand manifest fails instead of silently returning a lower-priority or synthesized manifest, and move the file read off the async request worker. (@xiaomo)
- Read exported configuration asynchronously so downloading `config.toml` cannot block API request workers. (@xiaomo)
- Serialize session compaction with concurrent message writers so an LLM compaction cannot overwrite messages saved after its initial snapshot (#7070) (@houko)
- Use effective dashboard i18n defaults, make channel save warnings coherent, stop unavailable QR polling, and keep channel selections synchronized. (#7072) (@houko)
- Clear kernel router cache lock poison after recovering routing state (#7126) (@houko)
- Recover poisoned background watcher state and close the stop-versus-registration race so stopping an agent aborts its in-flight tick and promptly releases the shared LLM concurrency permit. (#7129) (@houko)
- Recover poisoned channel-bridge abort-handle state so tracked tasks still stop during shutdown and hot reload. (#7139) (@houko)
- Recover poisoned terminal activity tracking so live PTY sessions retain accurate idle-timeout behavior. (#7141) (@houko)
- Recover poisoned command-catalog skill registry reads so installed slash commands remain visible. (#7142) (@houko)
- Recover poisoned agent skill-assignment registry reads so available skills remain visible without repeated recovery. (#7143) (@houko)
- Recover poisoned agent-message default-model reads so provider preflight keeps the active override without repeated recovery. (#7144) (@houko)
- Recover poisoned system-status model overrides and return provider/model from one consistent snapshot. (#7145) (@houko)
- Recover poisoned per-agent watcher slots and close registration races so background tasks are aborted when agents stop. (#7146) (@houko)
- Recover poisoned skill-catalog registry reads so installed skill lists and details remain available. (#7147) (@houko)
- Reject malformed and non-base64 image data URIs in OpenAI-compatible chat requests instead of forwarding corrupt vision blocks. (#7148) (@houko)
- Treat invalid session creation dates as undated so malformed timestamps cannot hide newer sessions in the dashboard. (#7149) (@houko)
- Validate dashboard date and uptime inputs so epoch timestamps render correctly and malformed values use a stable placeholder. (#7150) (@houko)
- Report malformed quoting, duplicate headers, and accurate source row numbers during dashboard user CSV imports. (#7151) (@houko)
- Make memory decay sweeps atomic, surface malformed access timestamps, and document zero-TTL behavior. (#7152) (@houko)
- Preserve goal run start times and validate deterministic persistence metadata. (#7153) (@houko)
- Reject malformed rate-limit counts and timestamps without panicking on provider headers. (#7154) (@houko)
- Preserve usage accounting errors and allow records that exactly reach configured quotas. (#7155) (@houko)
- Disable durable audit appends after an incomplete database reload and reject unknown persisted actions without coercion (#7179) (@houko)
- Restore the dashboard Hooks correctness gate and align lint overrides with test, config, and clipboard helper boundaries. (#7321) (@houko)
- Let dashboard section-label callers reliably override layout classes, centralize compact-label typography, and harden Overview range, memoization, timestamp, and typed-navigation contracts. (#7322) (@houko)
- Keep the dashboard Comms page resilient to partial snapshots and query failures, and align polling, refreshes, and counts with the active tab. (#7323) (@houko)
- Reject malformed Hand metadata in the dashboard chat picker and preserve agents that hold multiple Hand roles, instances, or memberships. (#7332) (@houko)
- Make generated agent-manifest Markdown resilient to table delimiters, embedded code fences, repeated blank lines, large backtick inputs, non-decimal costs, unsupported extras, and unknown schedule modes. (#7333) (@houko)
- Avoid recording canvas undo history or reallocating graph state when a stale context-menu node or connection target is deleted. (#7334) (@houko)
- Validate continuous agent schedule intervals, surface invalid values in the visual editor, and keep parsed manifest list identities stable across reloads. (#7335) (@houko)
- Preserve existing chat metering and memory metadata when delayed terminal frames omit optional fields. (#7336) (@houko)
- Wait for dashboard translation initialization before mounting and normalize detected regional locales to supported language codes. (#7337) (@houko)
- Pin the dashboard Lucide version used by curated deep imports and enforce every icon mapping and the exact-version contract with smoke tests. (#7338) (@houko)
- Honor the user's reduced-motion preference across the dashboard, reuse filter-free shared dialog variants, and remove paint-heavy blur keyframes. (#7339) (@houko)
- Align the dashboard session-selector documentation and short-ID fallback with their actual guarded contracts. (#7340) (@houko)
- Make dashboard skill-hub lookup null-safe, configure self-hosted registry URLs per deployment, and shell-quote copied install commands. (#7341) (@houko)
- Normalize video task statuses and stop dashboard polling after terminal states. (#7394) (@houko)
- Paginate dashboard memory records, apply agent and level filters before pagination and search caps, and return grouped per-agent counts without N+1 polling. (#7395) (@houko)
- Refresh stale dashboard version data when a long-lived window regains focus. (#7397) (@houko)
- Allow unfiltered dashboard cron queries while preserving explicit caller opt-outs. (#7400) (@houko)
- Update dashboard session truncation reactively and stop reconnecting completed streams. (#7401) (@houko)
- ClawHub CN skill details now share the same one-minute freshness window as the other dashboard marketplace detail views. (#7402) (@houko)
- Dashboard user filters now share one cached full-list request, match roles case-insensitively, and tolerate malformed channel-binding values without breaking search. (#7404) (@houko)
- Dashboard workflow detail, run, and operator-pause queries now preserve required-ID guards even when callers provide query enablement overrides. (#7405) (@houko)
- Session stream attachments now support authenticated WebSockets and release connection slots immediately when clients disconnect. (#7406) (@houko)
- Disclose when audit queries and exports can only inspect a truncated in-memory history window. (#7408) (@houko)
- Label authorization denial audit records with the endpoint that rejected the request. (#7409) (@houko)
- Restore both `config.toml` and `secrets.env` when a sidecar configuration write fails, reuse the runtime's canonical dotenv parser for shadow detection, and serialize registry metadata directly from its typed response. (#7412) (@houko)
- Give builtin slash commands precedence over colliding skills and release the skill registry lock before formatting command responses. (#7413) (@houko)
- Bound manual provider-test and pending A2A discovery caches with named, expiring entries, and remove stale route dead-code suppressions. (#7415) (@houko)
- Recover poisoned OFP peer rate-limiter locks without discarding active message or token counters. (#7417) (@houko)
- Recover a poisoned supervised-subprocess cooldown lock while preserving its respawn-storm guard. (#7418) (@houko)
- Recover a poisoned MCP OAuth refresh-lock registry without losing active single-flight entries. (#7419) (@houko)
- Recover poisoned plugin state-file and persistent-process registries without discarding active lock or process slots. (#7420) (@houko)
- Recover the external memory-provider slot after a provider panic poisons its lock, preserving the registered provider and allowing later hot swaps. (#7421) (@houko)
- Return an ACP internal error when an agent prompt stream closes before reporting its completion reason instead of presenting the aborted turn as successful. (#7422) (@houko)
- Let editor-backed filesystem calls fall back to local files when the optional ACP reverse-RPC times out or loses its response channel. (#7424) (@houko)
- Fixed ACP `session/resume` replaying persisted history to clients that already have the conversation. (#7425) (@houko)
- Fixed dashboard sparklines failing to render large data sets that exceed the JavaScript function argument limit. (#7426) (@houko)
- Fixed unknown workflow operator actions crashing the dashboard action bar. (#7427) (@houko)
- Fixed the dashboard schedule editor accepting out-of-range cron field values. (#7428) (@houko)
- Fixed shared dashboard buttons submitting surrounding forms unless explicitly configured as submit controls. (#7429) (@houko)
- Fixed dashboard input error styling disappearing while the field is focused or hovered. (#7430) (@houko)
- Fixed clickable dashboard cards and KPIs being inaccessible from the keyboard. (#7431) (@houko)
- Fixed multi-select free-text duplicates and active option announcements. (#7432) (@houko)
- Fixed status pills defaulting unknown states to running and labeling denied states as rejected. (#7433) (@houko)
- Fixed unnamed shared select controls for assistive technology. (#7434) (@houko)
- Fixed missing accessibility state and names in the skill output panel. (#7435) (@houko)
- Harden dashboard route parsing and stale-asset recovery without risking unbounded reloads when browser storage is unavailable. (#7465) (@houko)
- Respect reduced-motion preferences across CSS animations, transitions, scrolling, and their delays (#7467) (@houko)
- Validate canvas imports before replacing React Flow state and detach imported canvases from previously selected workflows.
  Dependency selections and imported legacy labels use stable step-node IDs; invalid restored references and stale runtime state are rejected or cleared. (#7468) (@houko)
- Preserve unsaved user-policy edits across background refreshes without overwriting unrelated concurrent server changes.
  Channel rule keys are normalized before duplicate checks, and successful saves immediately become the clean form baseline. (#7469) (@houko)
- Reject invalid analytics budget values before submitting a partial update.
  CSV exports now neutralize spreadsheet formulas and control-character prefixes in agent and model identifiers. (#7470) (@houko)
- Keep mobile pairing countdowns, QR rendering, and concurrent device removals synchronized with their actual request state.
  Invalid expiry timestamps now fail closed instead of displaying `NaN` (#7471) (@houko)
- Keep the memory embedding provider and model controls synchronized when switching catalogs.
  The custom-model input remains available while a new value is entered, and provider changes reset stale model and key settings. (#7472) (@houko)
- Prevent an explicit terminal disconnect from suppressing reconnects on a replacement WebSocket. (#7473) (@houko)
- Validate dashboard locale files and preserve array structure when checking translation-key parity. (#7497) (@houko)
- Restore strict dashboard dependency build enforcement so installs fail when scripts are not explicitly approved. (#7498) (@houko)
- Restore setup instructions in the pinned MCP registry fixtures by keeping them at the catalog root. (#7499) (@houko)
- Align the pinned Bedrock provider fixture with the bearer-token credential used by the runtime driver. (#7500) (@houko)
- Finish every failed MCP reconnect transition so health status no longer remains stuck in an in-progress state after connection or configuration errors. (#7514) (@houko)
- Honor HandsHub retry guidance as the complete wait before the next request so rate-limit responses do not stack server-directed delays with client backoff. (#7515) (@houko)
- Keep missing extension resources as distinct typed errors so API clients receive accurate 404 responses without losing the original failure text. (#7516) (@houko)
- Bound cron token-cap trimming with a binary search and compare estimates in u64 space so large limits remain correct on 32-bit targets. (#7523) (@houko)
- Keep deferred approvals pending when the kernel self-handle needed to resume them is unavailable so the decision remains retryable. (#7524) (@houko)
- Neutralize every triple-backtick sequence in untrusted reviewer context, including longer backtick runs that previously rebuilt a valid code fence. (#7525) (@houko)
- Preserve not-found and external-edit conflict status codes when wiki vault errors cross the kernel handle boundary. (#7526) (@houko)
- Return a typed not-found error when goal updates target an absent store or missing goal while keeping malformed goal storage as an internal error. (#7527) (@houko)
- Clamp persisted goal progress to the percentage range before rendering it in agent prompts so oversized values cannot wrap during integer narrowing. (#7528) (@houko)
- Report provider catalog scan, read, and parse failures while counting only successfully parsed catalog files in sync results. (#7529) (@houko)
- Reject rate-limit reset durations that overflow either `Duration` or the system clock so malformed headers fall through to a usable cooldown instead of panicking. (#7530) (@houko)
- Invalidate cached Vertex AI access tokens after authentication failures while preserving newer tokens installed by concurrent refreshes. (#7531) (@houko)
- Release the process registry guard before awaiting persistent-process stdin writes while serializing writes on a per-process pipe lock. (#7532) (@houko)
- Resolve conversation overrides and channel instance defaults in one SQLite snapshot so concurrent resets or rebinds cannot produce a mixed dispatch decision. (#7533) (@houko)
- Persist config migrations through durable atomic replacement while preserving symlinks and permissions. (#7535) (@houko)
- Serialize review-label mutations on the exact PR number, reconcile queued jobs against the latest actionable review, preserve actionable collectors, and propagate unexpected label failures. (#7546) (@houko)
- Serialize MCP server entries directly as TOML so absent options are omitted without deleting operator-authored JSON fields. (#7555) (@houko)
- Report config reload outcomes truthfully in audit records and surface channel adapter restart failures to API and Dashboard users. (#7556) (@houko)
- Fail workflow template list and detail requests explicitly when serialization fails instead of silently dropping templates or returning a null success body. (#7557) (@houko)
- Classify trigger registration failures by cause so missing agents return 404, invalid input returns 400, backpressure returns 503, and unexpected kernel failures return a scrubbed 500. (#7558) (@houko)
- Keep channel rate-limiter bucket admission within its hard cap under concurrency and avoid evicting buckets touched after an overflow snapshot. (#7559) (@houko)
- Sync channel journal appends before dispatch, use unique compaction staging files, and abort compaction when any snapshotted entry changes before atomic replacement. (#7560) (@houko)
- Join the sidecar supervisor during shutdown so an in-flight restart cannot leave a subprocess running after the adapter stops. (#7561) (@houko)
- Bound in-memory group rosters by chat and member count, evict least-recently-seen chats, and add explicit member and chat removal APIs. (#7562) (@houko)
- Require thread-ownership keys to pass through the validating constructor and normalizing scope builders while retaining read-only component accessors. (#7563) (@houko)
- Keep operator PYTHONPATH entries ahead of the embedded sidecar SDK fallback and serialize torn-tree recovery across concurrent extractors. (#7564) (@houko)
- Reject prompt-bearing skill files above 10 MiB and cap the actual supply-chain scan read so concurrent file growth cannot cause an unbounded allocation. (#7565) (@houko)
- Reject PDF payloads above 20 MiB and stream extracted characters into a 200K-character sink so output truncation no longer requires first allocating the complete text. (#7566) (@houko)
- Enforce per-result and total context character budgets even for tiny windows and retention floors. (#7596) (@houko)
- Return a guest-visible error instead of panicking when the WASM HTTP client cannot be built. (#7597) (@houko)
- Bound generated web-search queries and injected results so automatic augmentation cannot consume unbounded context (#7598) (@houko)
- Return OAuth callback errors immediately and release the loopback listener after every terminal result (#7599) (@houko)
- Escaped untrusted agent, provider, and model labels in Prometheus metrics to prevent malformed exposition and metric injection. (#7601) (@houko)
- Scrubbed internal agent-injection and proactive channel-delivery failures from API responses while retaining actionable server-side diagnostics. (#7603) (@houko)
- Rolled back webhook mutations that fail before durable replacement while retaining committed in-memory state when only the post-replacement directory sync fails. (#7607) (@houko)
- Timed out stalled inbound and outbound pre-authentication handshake reads under a shared deadline so unauthenticated frame buffers are released promptly. (#7608) (@houko)
- Retained last-known-good workspace context through transient path, metadata, and read failures while evicting confirmed missing or oversized files. (#7609) (@houko)
- Preserved existing ClawHub skill installs through staged promotion failures, cleaned failed staging trees, and stopped checksum discovery errors from downgrading installs to unverified downloads. (#7610) (@houko)
- Offloaded migration filesystem work, relocated only agents imported by the request, and kept relocation and response paths consistent on failures. (#7611) (@houko)
- Serialized quick-init configuration fields safely so provider catalog values cannot inject TOML keys or tables. (#7612) (@houko)
- Reported applied HTTP and WebSocket limits and only marked manifest signing available when every configured trust anchor is usable. (#7613) (@houko)
- Bounded terminal child exit polling so a stuck process cannot retain its WebSocket task indefinitely. (#7614) (@houko)
- Return a pollable workflow run ID when a synchronous API wait times out while execution continues in the background. (#7615) (@houko)
- Fail loudly when a built-in channel sanitizer regex is invalid instead of silently disabling the security rule. (#7617) (@houko)
- Age deferred channel journal entries from their retry deadline so stale recovery preserves the intended retry window. (#7618) (@houko)
- Launch the published SQLite MCP server with its configured database path instead of the unavailable npm package. (#7620) (@houko)
- Fail closed when OpenTelemetry tracing starts without its registered reload slot, and reject duplicate reload-layer installation. (#7621) (@houko)
- Authenticate notification broadcasts with recipient-bound peer handshakes while preserving existing connection state during short-lived deliveries. (#7622) (@houko)
- Let manual dream completion record time without retaining the current PID as a live lock holder.
  This prevents the next manual dream from being suppressed for up to the one-hour stale window (#7623) (@houko)
- Refresh the website's bounded offline HTML shell after successful navigation.
  Deep links now receive the latest cached application instead of an install-time-only root page or no fallback at all (#7624) (@houko)
- Fail startup when migration audit-row healing cannot complete instead of reporting a successful upgrade with an inconsistent audit trail.
  The healing pass is now atomic and can be retried safely after the underlying SQLite failure is resolved (#7625) (@houko)
- Allow external hooks to declare an exact executable path and lossless argument vector, preserving paths and individual arguments that contain whitespace (#7631) (@houko)
- Finalize unprocessable and delivered inbox files without repeated scans or duplicate delivery, while preserving processed files across timestamp collisions and retrying transient archival failures (#7632) (@houko)
- Bound EveryAPI credential-process pipe reads by the command deadline, report oversized output explicitly, and require safe HTTPS legacy custom endpoints (#7633) (@houko)
- Release process-local auto-dream lock claims when acquisition is cancelled during asynchronous file I/O, so later consolidation attempts are not permanently blocked. (@houko)
- Keep the xtask real-changelog regression green immediately after a release when the new Unreleased section contains only single-line prose. (@houko)
- Recover poisoned channel sidecar schema and schema-error caches so discovery and configuration remain available with preserved adapter metadata. (@xiaomo)
- Allow checkpoint restore to use valid abbreviated Git commit hashes shorter than eight characters without panicking while recording the pre-rollback snapshot. (@houko)
- Parse Codex CLI JSONL completion events so responses report the CLI's actual input, cached-input, and output token usage to metering instead of recording every call as zero tokens. (@xiaomo)
- Credential pools now warn when recovering from a poisoned state lock instead of silently continuing after a panic. (@xiaomo)
- Serialize cron-session pruning with persistent agent message writes so a blind prune save cannot overwrite a concurrently appended cron turn. (@houko)
- Drain the desktop dashboard sync task during embedded-server shutdown instead of dropping its JoinHandle and cancelling runtime work implicitly. (@xiaomo)
- Recover poisoned kernel and plugin event-bus drop-warning locks so overload and consumer-lag diagnostics remain visible. (@xiaomo)
- Parse Gemini CLI JSON output so responses report aggregated prompt, cached-prompt, candidate, and thinking token usage to metering instead of recording every call as zero tokens. (@xiaomo)
- Identity-file writes staged through a fixed `.{filename}.tmp` path, so two concurrent `PUT /api/agents/{id}/files/{filename}` requests for the same file shared one staging path and each `fs::write` truncated whatever the other had staged, leaving the renamed file holding interleaved bytes rather than either payload intact.
  The same write also never fsynced the staged file before the rename, nor the parent directory after it, so a crash could publish a directory entry pointing at unflushed content.
  Routing this through the crate's existing `atomic_write` helper fixes all three: the staging name is derived from the process ID and a per-process counter, the staged file is `sync_all`-ed before the rename, and the parent directory is synced afterwards on Unix (#7084) (@houko)
- Fail startup migrations when the migration audit or table schemas cannot be inspected instead of treating SQLite query and row-decoding failures as missing history or columns and continuing from an unverified schema. (@xiaomo)
- Recover poisoned passkey registration and authentication ceremony locks so later login flows continue with preserved short-lived challenge state. (@xiaomo)
- Recover poisoned background process registry state so one panicking holder cannot permanently disable output tracking, lifecycle updates, queries, or cleanup. (@xiaomo)
- Provider URL updates now serialize config writes, persist URL and proxy changes in one atomic replacement, and keep blocking filesystem work off async request workers. (@xiaomo)
- Serialize initial proxy publication and recover poisoned proxy state so concurrent initialization cannot repeat environment export and hot reloads are not silently discarded. (@xiaomo)
- Pipe Qwen Code prompts through subprocess stdin so conversation content is no longer exposed in process argument listings or constrained by platform argv limits. (@xiaomo)
- Recover and serialize the embedded SDK probe cache so concurrent sidecar starts issue at most one interpreter probe per command, including after a prior lock panic. (@xiaomo)
- Sidecar state and capability locks now warn once and clear poisoned state when recovering after a panic, preventing repeated recovery warnings on every later access. (@xiaomo)
- Recover poisoned trace-store SQLite connection state so telemetry insertion, trace queries, and circuit-breaker persistence continue after a panicking lock holder. (@xiaomo)
- Recover poisoned memory-wiki write serialization so page, compile-state, index, and backlink updates continue without disabling hand-edit conflict protection. (@xiaomo)

### Changed

- `browser_read_page` and `browser_navigate` now emit each link as a `⟨n⟩` marker in the prose plus a deduplicated marker-to-URL table, instead of inlining `[text](url)` at every occurrence, and `browser_click` accepts a marker.
  Measured across an aggregator, an article, a results page, and a docs site, separating the URLs from the prose is what pays: the link payload drops 70–84% on link-dense pages (13,739 → 4,028 on Hacker News, 50,285 → 8,274 on a Wikipedia article), where deduplication alone buys 1.5–7.6% and *costs* 4–5% on a page with no repeats.
  Same-origin entries are stored as a path against the page URL, since nearly every link on a page points back into it.
  The table lists only the links the surviving prose still refers to, so a marker cut off by the cap does not spend context on a URL the model cannot see.
  The table is a separate field on the extraction result rather than a section appended to `content`, so a caller reading `content` today is unaffected and the marker-to-URL map reaches `browser_click` as data rather than as prose to parse back out.
  `max_content_chars` now bounds prose *and* table together rather than prose alone: on a link-dense article the table by itself is larger than the default cap, so budgeting only the prose would have handed an operator who sized the cap to a context window a payload well past it.
  Trimming prose is what shrinks the table, since the table lists only surviving markers, so the two are solved together by searching for the largest prose cut whose combined total still fits — rather than by dropping entries and leaving markers in the prose that resolve to nothing.
  `browser_click` resolves a marker by re-running the extraction script rather than through a second copy of the traversal, so the number the model saw and the number that is resolved cannot drift apart.
  An anchor used as a click hook rather than as navigation — a bare `#` href, or a `javascript:` one — is left as plain text instead of being marked, since every one of them on a page resolves to the same string and deduplicating on it would give two unrelated controls the same marker.
  The dashboard's page preview (`GET /api/hands/instances/{id}/browser`) renders the link table beside the prose through the same shared renderer the tool result uses, rather than reading `content` alone, which would have shown an operator markers with nothing to resolve them against.
  Its 2,000-character budget cuts the prose before the table is joined, since the table alone runs to thousands of characters on a link-dense page and a budget applied afterwards would have spent all of itself on URLs.
  A bare number is treated as a marker only after the CSS and text paths have found nothing, so a page's own numeric link text — a pagination `5`, a numbered tab — stays clickable the way it always was rather than being claimed as a marker id.
  Text taken off the page has anything shaped like a marker defused before it reaches the output, since a marker is actionable and a page printing a literal `⟨2⟩` in its own text would otherwise attach a link it never wrote to whatever words it liked — and pull that link back into the table even where the real marker had been cut, the table being built by scanning the surviving prose.
  An anchor's scheme is compared with ASCII whitespace removed and case folded, so `JavaScript:`, a leading space and a tab inside the scheme are all recognised as the click hook they are rather than being marked as links.
  A list item whose sub-list sits between two runs of its own text keeps that text where the page put it, rather than joining everything before the sub-list to everything after it.
  That matters because the existing text fallback picks the first element whose `textContent` merely *contains* the selector, which resolves to the wrong element for 28% of the links on Hacker News and 16% on a Wikipedia article — on Hacker News the link text `new` resolves to `/news` where the intended link is `/newest` (#6624, #6746) (@nevgenov)
- Refresh the checked-in OpenRouter model snapshot used as the offline fallback catalog.
  The runtime's live catalog remains authoritative whenever OpenRouter is configured, so this update only affects lookups made before the first live fetch completes (#6701) (@houko)
- Refresh the checked-in OpenRouter model snapshot used as the offline fallback catalog.
  The runtime's live catalog remains authoritative whenever OpenRouter is configured, so this update only affects lookups made before the first live fetch completes (#6715) (@houko)
- Refresh the checked-in OpenRouter model snapshot used as the offline fallback catalog.
  The runtime's live catalog remains authoritative whenever OpenRouter is configured, so this update only affects lookups made before the first live fetch completes (#6720) (@houko)
- `agent_send` now delegates non-blockingly by default when the calling agent is known, returning a `task_id` whose reply is delivered to the caller's session on completion.
  The previous blocking default required the model to predict in advance that a delegation would be slow and opt in to `"async": true`; a wrong guess spent the entire turn waiting for `tool_timeout_secs`.
  An unnecessary `task_id` costs one extra turn to collect, whereas an unnecessary block can lose the turn outright.
  Pass `"async": false` for a quick sub-question whose answer is needed within the same turn.
  Callerless system-initiated sends keep dispatching synchronously, because the async tracker requires a known caller agent to route a completion back to. (#6740) (@houko)
- Split the Slack multi-step task-progress card from the processing-state reactions with a new `SLACK_PROGRESS_CARD` switch.
  The card was gated on `SLACK_REACTIONS`, so the only way to stop the emoji noise was to also lose the step list — the more useful of the two indicators on a long tool-using turn.
  The new switch defaults to whatever `SLACK_REACTIONS` resolves to, so an operator who set `SLACK_REACTIONS=false` for silence keeps exactly that, while `SLACK_REACTIONS=false` with `SLACK_PROGRESS_CARD=true` now gives the card without the reactions and the reverse gives the reactions without the card (#6741) (@houko)
- Refresh the checked-in OpenRouter model snapshot used as the offline fallback catalog.
  The runtime's live catalog remains authoritative whenever OpenRouter is configured, so this update only affects lookups made before the first live fetch completes (#6757) (@houko)
- Shared Telegram multipart upload storage across retry attempts instead of copying the full attachment on the happy path. (#6868) (@houko)
- Unified Telegram `getUpdates` and command responses behind the same generic API envelope while retaining strict required-status parsing. (#6884) (@houko)
- Replaced the Telegram done-reaction boolean argument with an explicit emit-or-suppress policy type. (#6888) (@houko)
- Made Telegram inline keyboard URL and callback actions mutually exclusive in the internal outbound type. (#6889) (@houko)
- Made Telegram photo-reply upgrading explicitly text-only without an unreachable panic branch in the inbound path (#6890) (@houko)
- Classified the publishable Telegram sidecar binary as a command-line utility instead of an API bindings library. (#6892) (@houko)
- Removed the unused reqwest streaming feature from the Telegram sidecar dependency graph. (#6893) (@houko)
- PDF text extraction for chat attachments now runs on Tokio's blocking pool instead of an async request worker, so a large or malformed PDF no longer stalls other in-flight requests on the same worker thread.
  Concurrent extractions are capped at two, with the semaphore permit held inside the blocking closure so a cancelled request cannot free capacity while its parser keeps running (#6961) (@houko)
- `GET /api/agents/{id}/files` now probes workspace identity-file existence and size on a `spawn_blocking` task instead of calling `std::fs::metadata` inline on the async handler.
  The per-file `.identity/` vs workspace-root fallback check previously ran as two separate `exists()` stats followed by a third `metadata()` call directly on a Tokio worker thread, parking it on disk I/O for every probed file on every request.
  The listing now runs as a single batched blocking task, and each probe collapses to one `metadata()` call instead of a redundant `exists()` + `metadata()` pair.
  A failed blocking task returns a scrubbed 500 rather than propagating the raw `JoinError` (#6980) (@houko)
- CLI passthrough model-config detection (Codex, Claude Code, Gemini CLI, Qwen Code) now runs on the blocking thread pool instead of the async request handler, since it reads files and environment variables from disk synchronously.
  All four probes are grouped into a single blocking task per request, and the reads are skipped entirely when an explicit `?tier=` filter excludes the synthesized `custom` rows they would produce (#6983) (@houko)
- User-management writes to `config.toml` (create/update/delete user, key rotation, provider-key changes) previously read, backed up, and wrote the file with blocking `std::fs` calls directly on the async request-handling task.
  Under load this could stall the Tokio worker thread the request landed on for the duration of the disk I/O, delaying unrelated requests scheduled on the same worker.
  The read and backup steps now go through `tokio::fs`, and the durable atomic write moves onto the blocking thread pool via `spawn_blocking`, with the existing config/API-key lock ordering and corrupt-config protection unchanged (#6985) (@houko)
- Move the identity-file path resolution, directory creation, and copy work performed by `POST /api/agents/{id}/clone` onto Tokio's blocking pool instead of running it inline on the async worker thread handling the request.
  The request's `ErrorTranslator` is dropped before awaiting the copy task, since it is `!Send` and would otherwise trip axum's `Handler` bound across the `spawn_blocking` await point; `agent_registry().get()` already hands back an owned clone rather than a lock guard, so no registry lock was ever held across the blocking work.
  Migrated `.identity/` files are still preferred over legacy workspace-root files, with the same fallback behaviour as before (#6987) (@houko)
- `/api/dashboard/snapshot` now runs its database health probe and session-count query together on Tokio's blocking pool instead of inline on the async worker.
  Both calls go through the synchronous SQLite substrate, so a slow disk could previously stall the worker thread handling the dashboard's 5 s poll.
  A blocking-task failure now logs at `error` level and falls back to the existing degraded-health / zero-count semantics instead of silently collapsing. (#6989) (@houko)
- `POST /api/config/set` read the existing `config.toml`, created the backup directory, copied the backup, and wrote the new file synchronously on the async worker thread handling the request.
  The existing-config read and the backup copy now go through Tokio's async filesystem APIs, and the durable atomic write of the new config runs on Tokio's blocking pool instead of inline.
  The missing-file case (no config to read or back up yet) is now handled via `ErrorKind::NotFound` instead of a synchronous `exists()` pre-check, preserving the same allowlist, TOML round-trip, validation, reload, and scrubbed-error behavior (#6990) (@houko)
- `DELETE /api/channels/sidecar/{name}` rewrote `config.toml` synchronously on the async worker thread handling the request, still holding the existing `config_write_lock` across the call.
  The sidecar-block removal and durable atomic rewrite now run on Tokio's blocking pool instead, with the config write lock held across the whole operation exactly as before.
  A join failure on that blocking task (for example a panic inside the removal closure) is now caught and returned as a scrubbed internal error instead of propagating as an unhandled panic in the request future (#6991) (@houko)
- `POST /api/channels/sidecar/{name}/configure` ran the `include`-shadow check, the `secrets.env` membership read, secret/config writes, and the `config.toml` upsert synchronously on the async worker thread handling the request.
  All of that now runs as a single `spawn_blocking` task on Tokio's blocking pool, still serialized under the same `config_write_lock` that gates `POST /api/config/set` and the legacy `configure_channel` handler.
  Moving the `include`-shadow check inside that lock (previously it ran before the lock was taken) closes a check/write race where a concurrent writer could add a conflicting `include` between the check and the write.
  Conflict and internal-error responses are unchanged; a join failure on the blocking task is now caught and returned as a scrubbed internal error instead of propagating as an unhandled panic (#6992) (@houko)
- Session-summary persistence (the SQLite `kv_store` write and the workspace `memory/session-*.md` mirror written when a session resets) ran synchronously inside the fire-and-forget background task that generates the summary, still occupying a Tokio worker thread for the duration of the disk I/O.
  That write now runs on Tokio's blocking pool via `tokio::task::spawn_blocking`, keeping the existing generate-then-persist ordering and the no-runtime synchronous fallback unchanged.
  A join failure on the blocking task is now logged as a WARN instead of propagating as an unhandled panic (#7038) (@houko)
- `POST /api/hands/{id}/pause|resume|deactivate` and `POST /api/hands/reload` ran hand-registry persistence, and for activation/deactivation the workspace and SQLite I/O behind it, synchronously on the async worker thread handling the request.
  All five lifecycle operations now run their kernel call through `tokio::task::spawn_blocking` so the request handler never parks on disk I/O.
  Successful responses and existing business-error status codes are unchanged; a join failure on the blocking task now returns a scrubbed 500 instead of propagating as an unhandled panic (#7039) (@houko)
- Chromium binary discovery for browser sessions previously ran synchronously on the async task launching the session, probing configured/candidate paths with blocking `std::fs` checks and shelling out to `which` / `where.exe` for a PATH lookup.
  A slow disk or a hung `which` invocation could stall the Tokio worker thread handling that request for the duration of the search.
  Discovery now runs on Tokio's blocking thread pool via `spawn_blocking`, preserving the existing configured-path, environment, platform-candidate, and PATH lookup order, with a blocking-task failure surfaced explicitly instead of silently propagating a `JoinError` (#7042) (@houko)
- Share one foreground polling policy across dashboard network reads. (#7396) (@houko)
- Name shared dashboard plugin and registry foreground refresh cadences. (#7398) (@houko)
- Correct dashboard credential-pool freshness and foreground refresh documentation. (#7399) (@houko)
- Dashboard terminal health freshness and live-window polling now use separate named cadences, making their intentionally different cache policies explicit. (#7403) (@houko)
- Return the already-serializable inbox status directly instead of allocating an intermediate JSON value. (#7414) (@houko)
- Replace a vacuous staged-turn drop test with an explicit ownership-invariant comment while retaining behavioral coverage for staged padding and commits. (#7534) (@houko)
- Released the shared router regex-cache lock before evaluating message matches, avoiding unnecessary serialization across routing requests. (#7604) (@houko)
- Workspace metadata cache misses in async non-streaming message paths now scan project and identity files on a blocking worker instead of stalling the runtime worker. (@xiaomo)

### Security

- Stop `GET /api/hands` from returning plaintext values for satisfied environment-variable requirements, which exposed host credentials and other sensitive process configuration to any caller allowed to list Hands.
  Requirement status now reports only whether each variable is present while preserving the existing Dashboard save contract (#6752) (@houko)
- Enforce ownership checks across agent-scoped reads and require an authenticated Admin or Owner credential for audit-ledger access, preventing cross-owner disclosure of prompts, configuration, files, sessions, traces, logs, delivery history, cron jobs, and schedules.
  Trusted credential-free deployments retain their existing compatibility for other routes but can no longer read the audit ledger without an explicit administrator credential (#6753) (@houko)
- Close a gap in the agent-ownership scoping this release also adds: `GET /api/agents` only injected `?owner=<caller>` when the query parameter was absent, so a non-admin caller could still list another user's agents by supplying `?owner=<other-user>` explicitly.
  Non-admin callers now always have `owner` pinned to their own username, regardless of any value supplied in the query string; Admin/Owner callers and the trusted no-auth compatibility mode are unaffected (#6753) (@houko)
- Close the last cross-owner gap in this release's agent-ownership scoping: `POST /api/agents/{id}/message` and `/message/stream` checked only that the target agent existed, not that the caller owned it.
  `agent_message` is one of the few RBAC carve-outs that let a plain `User`-role caller reach an arbitrary agent id, so without this check a non-owner could drive a full LLM turn — tool execution and budget spend included — on another user's agent by guessing or enumerating its UUID.
  Both handlers now apply the same `can_access_agent` ownership check already used for the read-only routes and for `/clone` (#6753) (@houko)
- Publish hot-reloaded users, channel bindings, tool groups, and role caches as one atomic authorization snapshot, preventing concurrent requests from briefly entering guest mode and bypassing RBAC while configuration is reloaded (#6754) (@houko)
- Require context-free blocking Hand tool requests to enter the human approval queue, preventing curated Hand auto-approval from bypassing per-user RBAC when sender and `force_human` context are unavailable (#6758) (@houko)
- Reject path-traversal values in Skillhub hand-scoped install requests before accessing the filesystem (#6759) (@houko)
- Reject current- and parent-directory segments in scoped capability path globs, including recursive `**` grants (#6760) (@houko)
- Pin legacy web-fetch connections to SSRF-validated DNS results and reject automatic redirects (#6761) (@houko)
- Harden link-context URL filtering against userinfo confusion, private IP ranges, and alternate IP encodings (#6763) (@houko)
- Require direct transport for DNS-pinned webhook test deliveries and URL attachment downloads (#6764) (@houko)
- Block entity-encoded and control-character-smuggled script URLs plus active SVG data URLs in Canvas HTML (#6765) (@houko)
- Escape untrusted ChatGPT OAuth callback errors before rendering the browser response. (#6766) (@houko)
- Escape untrusted provider OAuth callback errors before rendering the browser response. (#6767) (@houko)
- Apply the cron pre-processing script allowlist consistently to job updates. (#6768) (@houko)
- Match pooled Docker containers on the full sandbox isolation configuration. (#6769) (@houko)
- Keep credentials disabled after permanent authentication failures until pool reload. (#6770) (@houko)
- Require authentication by default in the AUR Docker package (#6771) (@houko)
- Agent context reads now bind path validation and file access to the same opened handles, preventing a workspace path swap from redirecting `context.md` outside the workspace.
  Symlinked identity entries no longer shadow a regular legacy context, and replacing a previously trusted context with a symlink falls back to its cached good content (#6772) (@houko)
- The hosted Fly deploy flow no longer copies a shared OpenRouter credential into user-owned machines, where every deployer could inspect and reuse it.
  Deployers now provide their own key, and the Worker forwards only that caller-owned credential into the caller's Fly machine configuration (#6774) (@houko)
- The Windows desktop uninstaller now parses the registered NSIS command line with native Windows argument semantics and launches the executable directly.
  A tampered per-user `UninstallString` can no longer append commands through shell metacharacters because the desktop app no longer passes it to `cmd /C` (#6775) (@houko)
- The Rust WASM skill SDK now rejects negative or otherwise invalid guest-memory ranges before constructing slices, returns a null sentinel for non-positive allocations, and validates host-call response ranges against current linear memory.
  Malformed ABI values can no longer create oversized or out-of-bounds Rust slices inside a skill guest (#6776) (@houko)
- Schema migration now fails closed when SQLite cannot read `PRAGMA user_version`, preventing a live database from being mistaken for a fresh version-zero schema and routed through destructive historical migrations (#6783) (@houko)
- Make the API-to-kernel import CI guard succeed when its scan reaches zero matches, use private per-run temporary files instead of shared `/tmp` paths, and remove the unaudited `boot_with_config` filtering escape hatch. (@houko)
- Restore registry signature verification after the Cloudflare account migration by synchronizing the daemon and Pages endpoint with the active signing-worker public key, and repair the CI lockstep guard so future key drift fails visibly. (@houko)
- Keep the manual release-tag version input out of generated shell source by passing it through step environment variables, with a CI regression check covering every release-tag `run:` block. (@TechWizard9999)
- Validate that the manual release-cli input names a canonical existing release before any build starts, pass it through workflow environment data for every upload, download, and signing shell step, and extend the release workflow CI scanner to cover both manual release workflows. (@TechWizard9999)
- Fail closed when the RL trajectory exporters cannot construct their redirect-disabled HTTP client. (@houko)
  W&B, Tinker, and Atropos previously fell back to the shared default client after a builder error; because that fallback follows redirects, a rare local client-configuration failure silently removed the SSRF guard and could replay export credentials to a redirected destination.
  Client construction is now shared by all three exporters, preserves the configured proxy and TLS settings, disables redirects, and returns the construction error instead of weakening the transport policy.
- Resolve and validate every RL exporter destination address immediately before upload, then pin the direct HTTP client to that complete validated set. (@houko)
  Tinker and the fixed W&B endpoint previously checked only the URL text, so a public-looking hostname could rebind to loopback, RFC-1918, link-local, cloud metadata, unspecified IPv6, or an IPv6 form embedding a forbidden IPv4 address between validation and connection; local-only Atropos aliases likewise lacked a connection-bound address check.
  Exporter traffic now bypasses explicit and environment proxies because ordinary HTTP, CONNECT, and socks5h proxies resolve the target outside LibreFang's validated resolver path. Redirects remain disabled, and DNS or secure-client construction failures stop the export without sending credentials or trajectory bytes.
- Stop RL exporters from buffering an upstream's complete error response before truncating the diagnostic to 4 KiB. (@houko)
  A malicious or broken W&B, Tinker, or Atropos endpoint could previously declare and stream an arbitrarily large 4xx/5xx body, forcing reqwest to accumulate it all in memory and potentially terminate the process before LibreFang applied its display cap.
  Error bodies are now consumed incrementally into a buffer capped at 4096 bytes, and the reader returns as soon as that cap is reached instead of waiting for the remaining response.
- Redact common AWS, GitHub, Slack, and Stripe credential formats from RL trajectory metadata before it leaves the process. (@houko)
  These tokens carry distinctive prefixes but can be shorter than the existing 40-character opaque-blob threshold, so values such as `AKIA…`, `ghp_…`, `xoxb-…`, and `rk_live_…` previously passed through to W&B or Tinker unchanged unless surrounding text happened to match the generic key/value rule.
  The exporter now applies a dedicated, prefix-constrained credential pattern before its existing generic API-key and blob rules, while retaining the kernel baseline parity check unchanged.
- Bound RL trajectory metadata redaction to 128 nested JSON containers and replace any deeper branch with `<REDACTED:TOO_DEEP>`. (@houko)
  `toolset_metadata` can contain values assembled directly by tools rather than parsed with serde_json's default recursion limit, and the previous recursive walker had no independent depth budget; a sufficiently nested value could overflow the exporter thread's stack before upload.
  Values through the documented budget retain the existing recursive credential scrubbing behavior, while the first over-budget container is replaced wholesale so neither its contents nor further recursion reach W&B or Tinker.
- Keep RL exporter retry logs free of upstream response bodies, transport messages, and credential-bearing URLs. (@houko)
  Both the warning emitted before a retry and the debug event emitted when giving up previously formatted the complete `ExportError`; transient 429/5xx errors include up to 4 KiB of upstream-controlled body text, while network errors can include sensitive URL components, sending those values into centralized operational logs.
  Retry events now record only a fixed error category and the HTTP status code when one exists. The original error is still returned unchanged to the caller, but is never passed to the tracing macros.
- Resolve and validate every DNS address for Python webhook callback URLs, then connect directly to that validated address set while preserving HTTPS SNI. (@houko)
  Callback delivery previously checked only IP literals and reserved hostname strings, so a public-looking hostname could resolve or rebind to loopback, RFC-1918, link-local, cloud metadata, or a private IPv4 endpoint embedded in IPv6.
  The callback transport now bypasses environment proxies and never re-resolves the hostname after validation; DNS failure or any unsafe answer fails closed before the signed request is sent.
- Default-denied Telegram updates without an identifiable sender whenever `ALLOWED_USERS` restricts access. (#6861) (@houko)
- Reserve each `Idempotency-Key` atomically before its handler starts, so concurrent retries can no longer execute the same state-creating side effect twice.
  An in-flight duplicate now receives `409 idempotency_key_in_use`; owner tokens prevent stale requests from modifying replacement reservations, cancelled and non-successful attempts release their reservation, and storage, clock, or corrupt-status failures fail closed instead of bypassing deduplication.
  Expired-row pruning is limited to once per minute (#6919) (@houko)
- Persist upload ownership metadata across restarts, enforce the same owner checks when attachments enter agent messages, explicitly mark daemon-generated images as shared, move upload serving off Tokio workers, and report the configured upload limit accurately (#6922) (@houko)
- Make verified TOTP codes single-use through an atomic SQLite claim shared by dashboard login, HTTP approval, enrollment reset, confirmation, revocation, and channel approval paths, and fail closed before sensitive state changes when replay persistence is unavailable.
  Move the claim onto Tokio's blocking pool instead of holding a process-wide mutex and synchronous SQLite work on an async worker.
  Register manual approval requests before returning `201 Created`, return recent resolved approvals from the per-id endpoint, report mixed batch outcomes with HTTP 207, and describe session-wide resolution accurately as best-effort rather than transactional. (#6923) (@houko)
- Replace the hand-rolled SHA-256 implementation in the plugin integrity path with the workspace's existing vetted `sha2` crate.
  The hand-rolled version was never audited and carried a stale comment suggesting a future swap to `sha2` that never happened, leaving plugin checksum verification resting on unreviewed cryptographic code.
  The public `sha256_hex` API and its lowercase 64-character hex digest format are unchanged, so no caller or stored checksum is affected (#6940) (@houko)
- Replace the hand-rolled RSA-SHA256 signer used to sign Vertex AI service-account JWTs with the workspace-vetted `jsonwebtoken` RS256 implementation.
  The removed code carried its own PEM/ASN.1 parser, PKCS#1 v1.5 padding, and a from-scratch big-integer modular-exponentiation routine — none of which had received the scrutiny a cryptographic primitive needs, and any subtle bug there (padding, timing, or big-integer arithmetic) could have corrupted or leaked the OAuth2 assertion used to authenticate to Google Cloud.
  The service-account claim set and OAuth assertion exchange are unchanged; new coverage signs with a generated PKCS#8 RSA key and verifies with the corresponding public key, and separately asserts that an invalid private key is rejected (#6941) (@houko)
- `spawn_agent_by_name` built the agent manifest path directly from the channel-supplied manifest name, so a name containing `..`, a nested path, or an absolute path could resolve outside `~/.librefang/workspaces/agents/` and load an arbitrary `agent.toml` from elsewhere on disk.
  The manifest name is now validated to be exactly one normal path component before the lookup, rejecting empty names, `.`, `..`, embedded separators, and absolute paths on both Unix and Windows (#6950) (@houko)
- The `build-timings` workflow still referenced `actions/upload-artifact` by the mutable `v4` tag, which is exactly the supply-chain gap this PR's sibling change to `cargo-deny.yml` was closing.
  It now pins to the same immutable commit already used for `v4` elsewhere in the workflow set (`coverage.yml`), keeping the tag in a trailing comment for readability (#6958) (@houko)
- The `cargo-deny` CI job pinned `EmbarkStudios/cargo-deny-action` to the mutable `v2` tag, so a compromised or repointed tag on that action would run inside CI with no additional review.
  The workflow now pins to an immutable commit SHA, keeping the `v2` release tag in a trailing comment for readability (#6958) (@houko)
- The Codex, Gemini, Qwen Code, and CodeWhale CLI drivers spawned their subprocess with an unbounded `.output()`/stdout-drain call, so a hung or malicious CLI process could block a request — and its stdout/stderr reader tasks — indefinitely.
  Subprocess execution is now bounded by a configurable per-driver timeout (`with_message_timeout`, defaulting to 300s, overridable per request), enforced via a shared `output_with_timeout` helper that kills the child and aborts its pipe-reader tasks on deadline, and the qwen-code streaming path now applies the same deadline to its line-by-line reads and final wait/drain, and now also surfaces accumulated partial text on a mid-stream timeout the same way the other streaming drivers already do (#6960) (@houko)
- `PeerRateLimiter`'s message and token counters keyed on the peer-supplied `peer_id`, and a peer authenticated with a shared secret can pick any node ID it likes, so a malicious or misbehaving peer could grow both `DashMap`s without bound simply by rotating identities.
  Both counters now cap at 10,000 distinct identities per window, sweeping expired entries before admitting a new one and rejecting the new identity outright once the cap is still hit.
  A count-only cap still let an attacker inflate memory through key size rather than entry count, since `peer_id` is attacker-controlled and was otherwise bounded only by the 16 MiB wire message limit, so oversized peer IDs are now rejected before either map is touched at all (#6962) (@houko)
- Require authentication by default in the reusable Fly.io deploy template.
  `deploy/fly/fly.toml` previously shipped `LIBREFANG_ALLOW_NO_AUTH=1` unconditionally, so every deployment derived from the template inherited the official demo's intentionally open auth posture, not just the demo itself.
  The one-command deploy script now generates a 256-bit `LIBREFANG_API_KEY` and imports it as a Fly secret before the app's first boot, and the official public demo's unauthenticated exception moved into its own release CI job rather than the shared template (#6963) (@houko)
- The GCP Terraform deploy opened SSH and the LibreFang dashboard/API firewall rules to `0.0.0.0/0`, and cloud-init still set the stale `LIBREFANG_BIND` variable instead of the supported `LIBREFANG_LISTEN`, leaving the public listener with no bearer key configured.
  Both firewall rules now require an operator-supplied `allowed_source_cidr`, with `0.0.0.0/0` and `::/0` rejected at plan time, and a required 32-character-minimum `LIBREFANG_API_KEY` is generated and wired through cloud-init so the API enforces bearer authentication (#6964) (@houko)
- `From<KernelOpError> for ApiErrorResponse` echoed the kernel's `Display` string straight into the HTTP body for every 500 and 503 response, so an `Internal` or `Unavailable` variant could surface database paths, file paths, or other internal state to the client.
  Server-error responses now return a fixed generic message (`Internal server error` / `Service unavailable`) while the full error is still logged server-side via `tracing::error!`; 4xx responses keep their actionable, client-caused message unchanged (#6967) (@houko)
- `PairingManager::complete_pairing` read the pending token, checked the device cap, and inserted the paired device as three separate `DashMap` operations, so concurrent redemptions of the same single-use token could each pass the checks before any of them removed the token — letting more devices redeem one pairing token than the configured `max_devices` cap allowed.
  The token-consume, cap-check, and device-insert sequence is now serialized under a dedicated lock, held only across the security-sensitive state transition and released before any blocking persistence callback runs (#6969) (@houko)
- The build-timings workflow's `upload-artifact` step and every Cloudflare Wrangler deployment invocation still referenced a mutable major-version tag (`actions/upload-artifact@v4`, `wrangler@4`), so a new release published under that same tag would run in CI without any additional review.
  `upload-artifact` now pins to the same audited v4 commit already used by `coverage.yml`, and each `wrangler` invocation is pinned to the exact `4.121.0` release (#6972) (@houko)
- Detect dangerous shell commands hidden behind `$IFS` whitespace expansion or base64 decode-to-shell pipelines, including when the agent uses Full exec policy. (#7068) (@houko)
- Preserve migration validation status while scrubbing internal path failures. (#7128) (@houko)
- Bind A2A and MCP caller context to authenticated principals, bound communication event streams, and make external-agent identity and host matching unambiguous. (#7416) (@houko)
- Suppress owner-private notices on ACP sessions that do not provide an explicitly owner-authenticated update channel. (#7423) (@houko)
- Bumped the transitive `h2` dependency from 0.4.13 to 0.4.16, closing RUSTSEC-2026-0258 ("h2 unbounded empty DATA frames").
  The advisory was published 2026-08-17 and immediately turned the Security lane red on every PR whose CI ran after it, since `cargo audit` counts it as a vulnerability rather than a warning.
  `h2` is purely transitive here — nothing in the workspace declares it — so the fix is a lockfile bump with no manifest change (#7708) (@houko)
- Keep the current-turn message boundary valid when heartbeat history pruning removes older silent responses, preventing stale-index skips and panics during post-turn memory processing. (@houko)
- Reject IPv4-compatible IPv6 literals such as `::127.0.0.1` across outbound URL guards, closing a private-network SSRF bypass. (@xiaomo)
- Scrub TOTP setup, confirmation, approval, and revocation 500 responses so vault, replay-store, and QR-generation details remain server-side. (@xiaomo)

### Documentation

- Correct the two task-board trigger snippets in the trigger-dispatch-concurrency guide, which documented a field that does not exist.
  Both wrote `event = "task_posted"`, but `ManifestTrigger` has no `event` field — the key is `pattern` and the value is the externally-tagged enum form `pattern = { task_posted = {} }`.
  Because `ManifestTrigger` derives `#[serde(default)]` the unknown key was dropped in silence, `pattern` fell back to JSON `Null`, and reconcile skipped the entry with a warning, so an operator copying either snippet got a manifest that parsed cleanly and registered no trigger whatsoever.
  The guide now also states that the key is `pattern`, explains why a typo there fails quietly, and shows the filtered `assignee_match` form so the narrower shape is discoverable.
  (#6742) (@houko)
- Documented that Telegram multi-chunk text sends can return an error after earlier chunks have already been delivered. (#6880) (@houko)
- Cut `CLAUDE.md` from 45k to 23k characters so it fits under Claude Code's 40k context budget again, moving the long-form detail into three new pages under `docs/development/` (`ai-safety-hooks.md`, `build-and-verify.md`, `github-collaboration.md`) plus `docs/architecture/session-mode-resolution.md` rather than deleting it.
  Every rule an agent has to obey stays inline; only the rationale and the incident write-ups behind each rule moved.
  Fixed three stale claims found while auditing: `CLAUDE.md` pointed `core.hooksPath` at a `.githooks/` directory that does not exist (it is `scripts/hooks/`), located session resolution in `kernel/mod.rs` instead of `kernel/agent_execution.rs`, and both agent files undercounted the workspace (24 and 15 crates against an actual 29).
  `docs/architecture/skill-workshop.md` documented `enabled` as defaulting to `true` in four places while `SkillWorkshopConfig::default()` has shipped `enabled: false` since #3328, which would have told an operator the workshop was already running for every agent.
  (#7709) (@houko)

### Added

- Accept configured HTTP clients (#6838) (@houko)
- Expose selectable TLS backends (#6842) (@houko)

### Fixed

- Adjust message boundary after heartbeat pruning (#6779) (@houko)
- Accept short hashes in checkpoint restore (#6780) (@houko)
- Release cancelled auto-dream claims (#6781) (@houko)
- Serialize cron prune with message writes (#6782) (@houko)
- Make API kernel import check zero-safe (#6789) (@houko)
- Restore registry pubkey lockstep (#6790) (@houko)
- Harden changelog article generation (#6791) (@houko)
- Enforce channel progress smoke contract (#6792) (@houko)
- Repair streaming example (#6793) (@houko)
- Package modules in legacy builds (#6794) (@houko)
- Stabilize GitHub stats hook lifecycle (#6795) (@houko)
- Restore stale cache refresh path (#6796) (@houko)
- Restore sysinfo MSRV compatibility (#6797) (@houko)
- Scan transitive dependency licenses (#6798) (@houko)
- Escape TOML control characters (#6804) (@houko)
- Preserve numeric map edit drafts (#6805) (@houko)
- Preserve empty struct list drafts (#6806) (@houko)
- Prevent release tag input injection (#6809) (@houko)
- Prevent release CLI input injection (#6810) (@houko)
- Fail closed on HTTP client errors (#6811) (@houko)
- Pin validated DNS addresses (#6812) (@houko)
- Cap error body reads (#6813) (@houko)
- Redact common credential formats (#6814) (@houko)
- Bound metadata redaction depth (#6815) (@houko)
- Keep retry logs payload-free (#6816) (@houko)
- Pin validated callback DNS addresses (#6818) (@houko)
- Reject malformed command params (#6819) (@houko)
- Surface reader task crashes (#6820) (@houko)
- Tie deploy progress to request lifecycle (#6821) (@houko)
- Close streaming responses (#6822) (@houko)
- Wrap connection errors (#6824) (@houko)
- Preserve split stream UTF-8 (#6825) (@houko)
- Report stream marshal errors (#6826) (@houko)
- Handle stream request errors (#6827) (@houko)
- Honor MIME body charsets (#6828) (@houko)
- Decode complete subjects (#6829) (@houko)
- Always close IMAP sessions (#6830) (@houko)
- Validate IMAP fetch responses (#6831) (@houko)
- Escape IMAP search senders (#6832) (@houko)
- Surface stream transport errors (#6833) (@houko)
- Encode URL path segments (#6834) (@houko)
- Bound stream event buffering (#6835) (@houko)
- Validate basic example responses (#6839) (@houko)
- Align poll option ID types (#6848) (@houko)
- Bound streaming state (#6851) (@houko)
- Require update identity fields (#6852) (@houko)
- Bound degenerate chunks (#6854) (@houko)
- Escape sanitizer text nodes (#6855) (@houko)
- Require location coordinates (#6857) (@houko)
- Ignore self-closing carry tags (#6858) (@houko)
- Render adjacent italic runs (#6860) (@houko)
- Parse quoted tag attributes (#6862) (@houko)
- Expose complete format pipeline (#6863) (@houko)
- Preserve unclosed fence content (#6864) (@houko)
- Pass owned values to redact_metadata in tests (#6898) (@houko)
- Preserve real multiline changelog test (#6900) (@houko)
- Unbreak main — dropped-translator session guard, the test premise it hid, and the clippy debt behind it (#6938) (@houko)
- Preserve budget serialization errors (#6952) (@houko)
- Prune empty tag index buckets (#6956) (@houko)
- Report hand rollback persistence failures (#6993) (@houko)
- Durably patch ClawHub provenance (#6994) (@houko)
- Durably patch Skillhub provenance (#6995) (@houko)
- Validate commands behind env and nohup wrappers (#6997) (@houko)
- Align the generic error locale contract (#7001) (@houko)
- Log peer registry poison recovery (#7002) (@houko)
- Log sidecar state lock poison recovery (#7005) (@houko)
- Log approval lock poison recovery (#7008) (@houko)
- Recover poisoned cache locks observably (#7009) (@houko)
- Log skills state lock recovery (#7010) (@houko)
- Log accessor lock poison recovery (#7011) (@houko)
- Recover poisoned shutdown locks (#7012) (@houko)
- Log registry sync lock recovery (#7014) (@houko)
- Log ChatGPT token cache recovery (#7015) (@houko)
- Log reservation ledger recovery (#7016) (@houko)
- Log bindings lock recovery (#7017) (@houko)
- Log user credential vault recovery (#7018) (@houko)
- Log shared vault recovery (#7019) (@houko)
- Recover poisoned provider state (#7020) (@houko)
- Log Copilot token cache recovery (#7021) (@houko)
- Log A2A registry lock recovery (#7022) (@houko)
- Log taint warning cache recovery (#7023) (@houko)
- Log quality regex cache recovery (#7024) (@houko)
- Log trigger persistence recovery (#7025) (@houko)
- Log workflow persistence recovery (#7027) (@houko)
- Serialize provider URL config writes (#7043) (@houko)
- Log credential pool poison recovery (#7045) (@houko)
- Clear recovered sidecar lock poison (#7046) (@houko)
- Meter Codex CLI token usage (#7047) (@houko)
- Meter Gemini CLI token usage (#7048) (@houko)
- Pipe Qwen prompts over stdin (#7049) (@houko)
- Fail closed on migration audit errors (#7050) (@houko)
- Move dashboard install off async workers (#7051) (@houko)
- Load WASM modules asynchronously (#7052) (@houko)
- Surface backup listing errors (#7053) (@houko)
- Propagate prompt store read errors (#7054) (@houko)
- Fail closed on config read errors (#7055) (@houko)
- Fail closed on dream lock read errors (#7056) (@houko)
- Surface extension reload failures (#7057) (@houko)
- Load agent templates asynchronously (#7058) (@houko)
- Fail closed on sidecar include errors (#7059) (@houko)
- Block IPv4-compatible IPv6 SSRF (#7073) (@houko)
- Drain dashboard sync on shutdown (#7075) (@houko)
- Serialize proxy initialization (#7130) (@houko)
- Recover process registry lock poison (#7131) (@houko)
- Serialize SDK probe cache (#7132) (@houko)
- Recover trace store lock poison (#7133) (@houko)
- Recover wiki vault write lock (#7134) (@houko)
- Recover passkey ceremony locks (#7135) (@houko)
- Recover channel schema caches (#7136) (@houko)
- Scrub TOTP internal errors (#7137) (@houko)
- Recover event bus warning locks (#7138) (@houko)
- Close shell expansion command bypasses (#7166) (@houko)
- Serialize session compaction writes (#7167) (@houko)
- Atomically persist MCP migration (#7168) (@houko)
- Restore Windows warning-free build (#7171) (@houko)

### Performance

- Trim Tokio features (#6841) (@houko)
- Drop multithread Tokio runtime (#6844) (@houko)
- Avoid cloning command payloads (#6850) (@houko)
- Restore code placeholders once (#6865) (@houko)
- Offload backup listing and deletion (#6954) (@houko)
- Make status probes asynchronous (#6955) (@houko)
- Offload ClawHub install finalization (#6996) (@houko)
- Offload workspace metadata scans (#7044) (@houko)
- Offload agent file mutations (#7060) (@houko)
- Bound skill supporting file reads (#7061) (@houko)
- Read hand manifests asynchronously (#7062) (@houko)
- Read config exports asynchronously (#7063) (@houko)
- Offload sidecar configuration writes (#7170) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Documentation

- Make basic endpoint configurable (#6840) (@houko)
- Trim quick-start imports (#6847) (@houko)
- Clarify required field policy (#6849) (@houko)
- Warn about open allowlist (#6853) (@houko)
- Record two batch-merge failure modes learned from a 120-PR backlog (#6895) (@houko)
- Warn that cancelling a live run leaves CI Gate permanently red (#6901) (@houko)
- Clarify Bedrock context fallback (#7616) (@houko)

### Maintenance

- Update model snapshot (#6693) (@houko)
- Bump the cargo-minor-patch group with 4 updates (#6709) (@app/dependabot)
- Bump the actions-minor-patch group with 3 updates (#6718) (@app/dependabot)
- Bump the web-minor-patch group in /web with 7 updates (#6724) (@app/dependabot)
- Bump the dashboard-minor-patch group in /crates/librefang-api/dashboard with 7 updates (#6725) (@app/dependabot)
- Update model snapshot (#6727) (@houko)
- Bump the docs-minor-patch group in /docs with 9 updates (#6733) (@app/dependabot)
- Update model snapshot (#6738) (@houko)
- Update model snapshot (#6786) (@houko)
- Isolate secret lookup (#6817) (@houko)
- Align thiserror major version (#6843) (@houko)
- Allowlist package contents (#6845) (@houko)
- Declare dependency floors (#6846) (@houko)
- Update model snapshot (#6907) (@houko)
- Bump the cargo-minor-patch group with 5 updates (#6924) (@app/dependabot)
- Bump totp-rs from 5.7.2 to 6.0.0 (#6927) (@app/dependabot)
- Update model snapshot (#6929) (@houko)
- Bump the actions-minor-patch group with 2 updates (#6932) (@app/dependabot)
- Bump Swatinem/rust-cache from e18b497796c12c097a38f9edb9d0641fb99eee32 to a45951ff880207c249adf57334cf2e9bd81d6e1e (#6933) (@app/dependabot)
- Bump the cargo-minor-patch group across 1 directory with 3 updates (#6937) (@app/dependabot)
- Update model snapshot (#6953) (@houko)
- Inherit workspace package metadata (#6957) (@houko)
- Bump the web-minor-patch group across 1 directory with 4 updates (#7064) (@app/dependabot)
- Bump framer-motion from 12.43.0 to 13.0.0 in /web (#7065) (@app/dependabot)
- Bump the dashboard-minor-patch group in /crates/librefang-api/dashboard with 8 updates (#7066) (@app/dependabot)
- Bump motion from 12.43.0 to 13.1.0 in /crates/librefang-api/dashboard (#7067) (@app/dependabot)
- Update model snapshot (#7101) (@houko)
- Bump the docs-minor-patch group in /docs with 5 updates (#7161) (@app/dependabot)
- Bump motion from 12.43.0 to 13.1.0 in /docs (#7162) (@app/dependabot)
- Update model snapshot (#7292) (@houko)
- Update model snapshot (#7466) (@houko)
- Update model snapshot (#7602) (@houko)
- Update model snapshot (#7698) (@houko)
- Update model snapshot (#7706) (@houko)

</details>


## [2026.7.31] - 2026-07-31

_58 PRs from 4 contributors since v2026.7.27._

### Highlights

- **API key security** — Keys now support env/vault indirection and a hashed form, closing a hash-only WebSocket/terminal auth bypass; three additional authorization boundaries around plugin execution, MCP env values, and cross-user token refresh were also closed.
- **Speech-to-text improvements** — Language and prompt parameters now thread through STT, and video containers are accepted as input for transcription.
- **Browser CDP attachment** — Agents can now attach to browser-level Chrome DevTools Protocol endpoints via `Target.createTarget`, enabling richer browser automation.
- **EveryAPI integration** — Auto-detection of EveryAPI CLI credentials and new partner surfaces make connecting to EveryAPI faster and require no manual setup.
- **Kubernetes deployment** — A single-replica baseline with readiness contract and rootless restricted-PSS container support makes LibreFang deployable on Kubernetes out of the box.

### Added

- Add `exec_policy.full_mode_skips_approval`, which decouples the two properties `mode = "full"` has always fused, so an operator can run unrestricted shell commands that still prompt for approval.
  `Full` waived the global `approval.require_approval` list for `shell_exec` as well as skipping allowlist validation, so an operator who deliberately set `Full` for one agent silently lost their `require_approval = ["shell_exec"]` for it, with nothing on any surface saying so.
  That coupling was deliberate rather than accidental, so the flag makes a documented decision overridable instead of fixing a bug: with the flag off, `Full` waives only command validation and `[approval]` decides who must confirm, exactly as under `allowlist`.
  The default is `true` and preserves today's behaviour on every existing install, and it is deliberately not flipped for two reasons that are load-bearing together: `ApprovalPolicy::default()` ships with `shell_exec` in `require_approval`, and `Kernel::spawn` promotes any standalone agent whose `capabilities.tools` contains `shell_exec` or `*` and which declares no `exec_policy` to `mode = "full"`.
  On a stock install this waiver is therefore the only reason ordinary agents run shell commands unattended, and a flipped default would prompt on every command rather than expressing a new operator intent.
  The `safe_bins_skip_approval` waiver from #6000 is intentionally left unconditional: it is by its own name an explicit approval opt-out, whereas `Full` is a command-validation mode that never claimed to speak for `[approval]`.
  A per-user RBAC `NeedsApproval` still forces the approval queue in either position of the flag, and the field is readable on `GET /api/config` alongside its neighbour.
  Like the rest of `exec_policy` it is baked into each agent's manifest at spawn / restore time, so `POST /api/config/reload` does not retrofit it onto already-running agents — kill the agent and let it respawn, or restart the daemon (#6594) (@houko)

- Add an EveryAPI connect action to the dashboard's Providers page, so registering the gateway no longer requires dropping to `librefang models connect everyapi` in a terminal.
  EveryAPI is not a built-in provider: until a registry entry exists it is absent from `GET /api/providers` altogether rather than merely unconfigured, so the Add picker — which lists what that endpoint already returns — could never surface it, and the dashboard had no path to it at all.
  The picker footer now offers a connect action while no `everyapi` entry exists, opening a two-field drawer for the relay key and an optional gateway root.
  It keys off the entry *existing* rather than being usable, because an entry with a missing key is already reachable through the normal configure flow and re-offering connect there would overwrite it.
  The entry is registered with an empty `models` array on purpose: `catalog_needs_initial_refresh` in `crates/librefang-api/src/everyapi_catalog.rs` is true exactly when the provider is configured but has no live models, so the daemon's `refresh_if_missing_in_background` fetches `/v1/models` and `/api/pricing` and synthesises the catalog itself.
  Duplicating that synthesis in the browser would mean a second copy of the pricing rules to keep in sync, and the gateway is not CORS-open to the dashboard in any case.
  The provider id, display name, key env var and default gateway root are exported as `EVERYAPI_PROVIDER` alongside the mutation and documented as a cross-language contract with the CLI's constants, since a drift there would register a provider the daemon never refreshes (#6586) (@houko)
- Add `librefang models connect everyapi [--set-default]`, which registers an [EveryAPI](https://github.com/everyapi-ai/everyapi) aggregating gateway as a custom LLM provider in one command, plus an `EveryApiWiringCheck` in `librefang doctor` that reports whether such a gateway is present and which route is actually in effect.
  EveryAPI's own `everyapi use <tool>` injects environment variables and execs a child process, which does nothing for a long-lived daemon whose HTTP drivers read `base_url` from config rather than the environment — so the wiring belongs on the LibreFang side.
  The command reads `api_base` and `relay_key` from `~/.config/everyapi/credentials.json` (honouring `$XDG_CONFIG_HOME`), fetches the gateway's live `/v1/models` listing, resolves model metadata from the builtin catalog (the gateway publishes ids but no pricing and mostly no context window), stores the key in `~/.librefang/.env`, and persists the provider through the running daemon when one is up or by writing `~/.librefang/providers/everyapi.toml` when it is not; the key never enters the provider TOML, the terminal, or the logs.
  A text model whose context window and output limit cannot both be resolved is skipped and named in the output rather than registered, because such an entry is discarded when the catalog loads and would otherwise vanish silently; entries with unresolved pricing are marked `pricing_known = false` so budget math does not treat them as free.
  Synthesised entries deliberately avoid `ModelTier::Custom`: `find_model` returns the first `Custom` match immediately (#983) and `merge_catalog_file` dedupes on `(id, provider)`, so a `Custom` gateway copy of a colliding id (`claude-sonnet-5`, `claude-opus-5`, `gemini-3.5-flash`) would have hijacked every provider-blind lookup and silently re-priced agents that never opted into the gateway.
  `--set-default` will not pick an `openai-response`-only model, since those reject the non-streaming calls that compaction, proactive memory, the skill workshop, and web augmentation all issue.
  The doctor check warns when an env route and a provider entry are live at once, naming both, because the effective gateway then differs per driver and neither surface mentions the other (#6583) (@houko)
- Source EveryAPI model metadata from the gateway's own public `/api/pricing` feed rather than reverse-looking-up ids in the compiled-in OpenRouter snapshot, and refresh the catalog on a TTL so the model list does not go stale.
  The snapshot lookup guessed a vendor prefix from `owned_by`, resolved only 7 of 18 models, and copied the upstream vendor's list price — which is not what the gateway charges.
  `/api/pricing` is served with optional auth, publishes `context_window` plus the gateway's real per-token ratios, and converts as `model_ratio * 2.0` for input and `model_ratio * completion_ratio * 2.0` for output; that conversion is corroborated by both the gateway's own docs and its `billing/quota.go` settlement path against `QuotaPerUnit = 500000`.
  Models billed per call carry no per-token price and are emitted with `pricing_known = false` instead of a bare `0.0`, which would have asserted they are free.
  Output-token limits are the one figure neither gateway endpoint publishes, so the snapshot is kept solely as their fallback and the command reports which models borrowed one.
  A new `everyapi_catalog` module in `librefang-api` mirrors the existing `openrouter_catalog` shape — TTL staleness check, background backfill, and a shared retry window — so a gateway that adds or removes models is picked up without re-running the connect command (#6583) (@houko)
- Add `librefang service install --system` on macOS, which registers a boot-time LaunchDaemon instead of the login-time LaunchAgent the command has always written.
  The existing behaviour was that every platform got a *per-user* service, so a Mac that rebooted and stopped at the login window — or at the FileVault unlock screen — never started the daemon at all, which makes an always-on install impossible without hand-writing a plist.
  `--system` writes `/Library/LaunchDaemons/ai.librefang.daemon.plist`, the directory launchd loads at boot before any login.
  It requires root and specifically requires `sudo` rather than a root login, because the job has to run as a real account and `SUDO_USER` is the only signal identifying which one; a missing `SUDO_USER` is an error rather than a silent default to root.
  The generated job sets `UserName` to that account, `HOME` to its home directory and `LIBREFANG_HOME` to `~/.librefang`, resolved from the passwd database rather than through `dirs::home_dir()` — under `sudo` the latter returns root's home and the daemon would serve a state directory the invoking user cannot read.
  The install also creates the state directory and `daemon.log` and hands the whole tree to the target account, because launchd opens `StandardOutPath` before dropping privileges and the daemon would otherwise be unable to write its own log.
  The handover is recursive on purpose: `~/.librefang` almost always exists already by the time anyone reaches for `--system`, so chowning only the directory node would leave its contents on whatever uid created them, and a state directory previously written by a `sudo librefang start` would leave the LaunchDaemon hitting EACCES at an arbitrary depth long after `service status` reported everything registered.
  The walk records symlinks but never descends them (`DirEntry::file_type` describes the entry, not its target, and the chown uses `lchown`), so a link pointing out of the state directory cannot redirect the handover onto an unrelated tree.
  The plist is chmodded to 0644, since launchd refuses to load a LaunchDaemon writable by anyone but its owner.
  The pre-existing root guard on the per-user path is unchanged: plain `service install` still refuses to run under `sudo`, and now points at `--system` when it does.
  `service uninstall --system` removes it, `service status` reports both the LaunchAgent and the LaunchDaemon with no flag needed, and `--system` on Linux or Windows is a clear error pointing at the `services.librefang` NixOS module or a hand-written system unit rather than a silent no-op (@houko)
- Promote NixOS from a package-only target to a first-class deployment surface, and give deepin / Debian-family hosts the distro awareness they previously lacked entirely.
  The flake gains system-agnostic `nixosModules.default` / `nixosModules.librefang` (backed by the new `nix/nixos-module.nix`) exposing `services.librefang` with `port`, `openFirewall`, `user`, `group`, `stateDir`, `environmentFile`, and `extraEnvironment`, plus `overlays.default` so nixpkgs consumers can pull `librefang-cli` / `librefang-desktop` into their own package set.
  `ExecStart` uses `start --foreground` deliberately: the default `librefang start` takes the `!spawned && !foreground` branch into `spawn_detached_daemon`, which `setsid`s a child and returns from the parent, so a unit without the flag would have its service torn down moments after `nixos-rebuild` reported success.
  Evaluation-time assertions reject an `environmentFile` inside the Nix store (which would make provider keys world-readable), a `stateDir` whose final component is empty or whose parent is a shared FHS directory, an `extraEnvironment.LIBREFANG_HOME` that would desynchronise from `StateDirectory`, and an `openFirewall` paired with a non-loopback bind that has no auth source.
  `checks` now covers `librefang-desktop` on Linux (previously reachable only through `packages`, so a `wrapGAppsHook3` or desktop-item regression passed `nix flake check`) and a `nixos-module-eval` check that instantiates a throwaway `nixosSystem` and asserts thirteen properties of the generated unit, which is how CI validates the module without a NixOS host.
  A `nixos-vm-test` check boots a real NixOS guest and asserts the unit reaches active, port 4545 opens and `/api/health` answers, covering the one thing evaluation cannot prove; it is opt-in via `nix build .#checks.x86_64-linux.nixos-vm-test` because building it compiles the CLI and boots a VM, while `nix flake check --no-build` still instantiates the expression so a module rename that broke the test is caught (measured: the full `--no-build --all-systems` pass instantiates both new checks and performs zero builds).
  `nix-build.yml` gains a `pull_request` job that runs evaluation only, so the Nix path is gated before merge without paying the 80-95 minute cold compile the push-to-main matrix still runs.
  The installer learns `detect_distro` from `/etc/os-release` with `/etc/NIXOS` as the authoritative NixOS marker, degrading silently to unknown where neither exists, and on NixOS it now suppresses the glibc fallback at both decision sites — that binary can never execute there for want of `/lib64/ld-linux-x86-64.so.2`, so the previous behaviour downloaded it, failed the post-install run check, rolled back, and printed a generic failure that told a NixOS user nothing.
  `librefang doctor` gains a Linux-only soft check that probes pkg-config for the desktop webview stack and maps `/etc/os-release` to a per-family remediation hint, deliberately suggesting a package *search* rather than concrete package names because the package shipping a given module differs between distributions and between releases of one distribution.
  Documented in `docs/operations/nixos.md`, plus new README and docs-site sections for NixOS and Debian / Ubuntu / deepin in both English and Chinese (@houko)
- Implement the MCP `resources` primitive in the `librefang-runtime-mcp` client, which was previously tools-only, so an agent can now consume MCP servers that expose their data as resources rather than tools.
  `McpConnection` gains `list_resources` (`resources/list`), `read_resource` (`resources/read`), and `list_resource_templates` (`resources/templates/list`) on both the rmcp (stdio + streamable-HTTP) and hand-rolled SSE transports; HttpCompat has no resources concept and returns a clear error.
  When a server advertises the `resources` capability (read live from the rmcp handshake `peer_info`, or captured from the SSE `initialize` response) the client registers two synthetic tools — `list_resources` and `read_resource` — that flow through the normal tool-call loop and are intercepted before the transport `tools/call`, so a real server tool literally named `read_resource` is unaffected.
  A `resource_link` in a tool result is now surfaced as a first-class `[resource_link] name — uri (mime)` line instead of being flattened into an opaque JSON string, and an embedded resource contributes its text (binary blobs are elided, never inlined into the prompt); resource lists are sorted by URI for prompt-cache stability.
  No `resources` client capability is declared because the MCP `resources` capability is server-side and rmcp's `ClientCapabilities` has no such field (#6501) (@houko)
- Emit a `librefang_media_understanding_failures_total{kind,provider,model}` counter (with a matching structured `warn!`) whenever vision description or audio transcription fails, so a hosted model that a provider silently retires — e.g. Groq removing the hardcoded default `meta-llama/llama-4-scout-17b-16e-instruct` — surfaces as an actionable metric instead of a days-later user report.
  The counter is incremented at the point of failure inside `librefang-runtime-media` (both the image and audio provider-dispatch paths, plus the empty-result case), so it is captured regardless of caller, and its description is registered in `librefang-telemetry` for the Prometheus exporter.
  The hardcoded default models are intentionally left unchanged (choosing replacements is an operator call); a comment on `default_vision_model` now documents that these ids rot and points at the new metric (#6538) (@houko)
- Expand the scheduler delivery-target channel-type dropdown from 4 presets to all 25 first-party sidecar channel adapters, so operators picking a `channel` fan-out target can select WeChat, WeCom, Feishu / Lark, DingTalk, Microsoft Teams, Google Chat, Rocket.Chat, Matrix, Mattermost, WhatsApp, QQ, LINE, and the rest by name instead of typing the raw `channel_type` string. The two adapters that map to their own delivery-target tabs (`email`, `webhook`) are intentionally excluded to avoid two UI paths to the same target, and the existing "Custom…" escape hatch still passes through any channel_type the transport accepts (#6476) (@houko)
- Edit `HAND.toml` online from the Hands panel: the read-only manifest viewer now has an Edit / Save / Cancel affordance backed by a new authenticated `PUT /api/hands/{id}/manifest` that validates the submitted TOML by parsing it into a `HandDefinition` (rejecting invalid TOML or a changed `id` with a 400 and leaving the on-disk file untouched), runs the same supply-chain audit as the install path, persists the file to whichever on-disk copy the hand loads from, and hot-reloads the in-memory definitions — an already-active hand instance keeps its old manifest until it is deactivated and reactivated, and edits to a built-in (registry) hand are overwritten on the next registry sync (#6478) (@houko)
- Slack sidecar multi-step task-progress display: the generic AgentPhase lifecycle (Thinking → ToolUse{name} → Done/Error) now reaches the Slack adapter as `reaction` commands — Slack declares the existing `reaction` capability rather than a new one, since that is the capability `ChannelAdapter::send_reaction` already dispatches the phase lifecycle through — and is rendered as an updated-in-place Block Kit step list via `chat.update` for multi-step turns, while single-step turns keep the prior eyes → white_check_mark receipt reactions (both honour `SLACK_REACTIONS`); the `reaction` command gained optional `phase` / `tool_name` fields (backward-compatible, omitted when empty) carrying the lifecycle detail the emoji alone drops, and the bridge now also emits `ToolUse` phases on the non-streaming dispatch path so a non-streaming adapter that declares `reaction` sees the full `Thinking → ToolUse… → Done/Error` sequence (#6451) (@houko)
- Add opt-in completion notification for `process_start` background processes via the #4983 async-task tracker: a call with `notify_on_completion: true` registers a new `TaskKind::Process { pid }`, and when the process exits on its own or is killed the kernel injects a `TaskCompletionEvent` (exit code + tail of the captured output) back into the originating session, so a long-running process no longer has to be polled to learn it finished — the same delivery path (mid-turn injection / wake-idle) and `[async_tasks]` config that workflow / delegation tracking already use, with no new config surface (#6471) (@houko)
- Add opt-in completion notification for `process_start` background processes via the #4983 async-task tracker: a call with `notify_on_completion: true` registers a new `TaskKind::Process { pid }`, and when the process exits on its own or is killed the kernel injects a `TaskCompletionEvent` (exit code + tail of the captured output) back into the originating session, so a long-running process no longer has to be polled to learn it finished — reusing the same delivery path (mid-turn injection / wake-idle) as workflow / delegation tracking with no new config surface; note the `[async_tasks]` `default_timeout_secs` auto-kill is deliberately NOT applied to processes (a background server is meant to run indefinitely), only the completion-delivery machinery is shared (#6471) (@houko)
- Expose per-channel `dm_policy`, `group_policy`, `threading`, and `output_format` on `[[sidecar_channels]]`, restoring the channel-level override slot that was lost in the sidecar migration (#6445): each is `Option<_>`, so `overrides_from_sidecar_config` projects only the fields an operator explicitly set and an unset knob never materializes a policy they did not write. Precedence is unchanged — agent-level `[channel_overrides]` still wins over the per-channel value (#6468) (@houko)
- Per-user LLM provider credentials, end-to-end (#6460): a human user can store their own upstream provider API key encrypted in the existing credential vault (`CredentialVault`, AES-256-GCM, keyed by `LIBREFANG_VAULT_KEY` / OS keyring) under a per-user, per-provider namespace via `LibreFangKernel::{set,get,remove,list}_user_provider_key`, and their agent turns now bill that key. The authenticated `AuthenticatedApiUser.user_id` is read at the `/api/agents/{id}/message` and `/message/stream` handlers and threaded as `owner: Option<UserId>` through the kernel send/streaming/ephemeral entry points into `resolve_driver_for_owner`, whose precedence is (highest first) org allowlist (#6459) > agent-pinned `api_key_env` > user-scoped key > credential pool > operator `auth_profiles`/`provider_api_keys` rotation > catalog/convention env. The same owner-key preference is applied to every fallback-chain slot, so a provider failover cannot silently bill the operator's credential (a chargeback leak). Paths without a single authenticated initiator — channel messages, cron fires, agent-to-agent sends, and forks/sub-agents — pass `owner = None` and fall back to the daemon-global credential; the fork path additionally clears any inherited owner defensively so a sub-agent's spend is never mis-attributed to the parent turn's user. Global-only behaviour is byte-identical when no user key exists, the plaintext value is never returned through the API (listing surfaces provider names only), and per-owner spend is queryable via the existing `/api/budget/users` rollup. The HTTP/dashboard management surface for provider keys remains a follow-up (#6460) (@houko)
- Owner-gated HTTP surface for the per-user provider credentials above (#6460 Follow-up B): `PUT /api/users/{name}/provider-keys/{provider}` stores a user's upstream key, `DELETE /api/users/{name}/provider-keys/{provider}` removes it, and `GET /api/users/{name}/provider-keys` lists the provider NAMES a user has configured — never any secret value, since the kernel's plaintext getter stays `pub(crate)` and is deliberately absent from the `KernelApi` trait the HTTP layer calls through; writes are Owner-only via the existing `is_owner_only_write` `/api/users/` prefix gate (the same gate that guards create / delete / rotate-key) and the list GET is Owner-gated in `min_role_for_privileged_get` (mirroring `/api/config/export`) so an Admin cannot enumerate another user's provider layout, the `provider` segment is validated against the canonical `known_providers()` registry (rejecting empty / `/`-containing / unknown names with 400), and an unknown user name yields 404 so a typo never orphans a vault entry (#6460) (@houko)
- Let the master API credential live somewhere other than cleartext in `config.toml`, via a new `api_key_hash` and by routing `api_key` through the same env / `vault:` resolution the dashboard credentials already used.
  `config.toml` sits inside the daemon's own writable data dir and gets rewritten by the daemon, so it can never be a read-only Kubernetes Secret mount — `LIBREFANG_API_KEY` and `api_key = "vault:name"` are the two ways to keep the working secret out of the file entirely, and both are now resolved per auth snapshot rather than once at boot, so `POST /api/config/reload` no longer clobbers the override by re-reading the file from disk.
  `api_key_hash` holds `$sha256$…`, produced by the new `librefang hash-api-key` command, and `$argon2id$…` stays accepted for a hand-written value or a deliberately short human-memorable key.
  SHA-256 is the recommended form here and Argon2id remains the one for `dashboard_pass_hash`, which reads like an inconsistency and is not: a dashboard password is human-chosen, so the memory-hard KDF is what makes an offline dictionary attack uneconomic, while a master API key is a machine-generated bearer where there is no dictionary to enumerate — the KDF buys nothing against an offline attacker and instead charges ~50–100 ms of CPU to every request, including every wrong token from an unauthenticated caller on paths that have no login-attempt limiter.
  An `$argon2id$` master hash is therefore verified on a blocking thread rather than inline, so the format an operator chooses can never stall the async runtime.
  Existing plaintext deployments keep working and get a `$sha256$` hash written to a 0600 `api-key-hash.upgrade-hint` file on first authentication, mirroring the `dashboard_pass` → `dashboard_pass_hash` path; clients keep sending the same key, only the daemon's stored copy changes.
  The hash is never logged, because it is the verifier — anyone who could read it out of the log stream could paste it into their own config and authenticate.
  Reloading `api_key` or `api_key_hash` now reaches the HTTP middleware without a daemon restart: `api_key_lock` was previously written only at boot and on a dashboard credential change, so an edited master key kept authenticating with the old value (#6613) (@houko)
- Attach to browser-level CDP endpoints by creating a target and attaching to it, so a `cdp_endpoint` pointing at a browser-level WebSocket works instead of dying on the first command.
  `attach()` sent `Page.enable` immediately, which holds only for a page-level endpoint; against a browser-level one no page exists yet, so the session died at startup — Lightpanda reports this as `BrowserContextNotLoaded`, Chrome as `'Runtime.enable' wasn't found`.
  On a `ws://` endpoint librefang now asks `Target.getTargetInfo` which shape the endpoint is and, only when it answers `type: "browser"`, issues `Target.createTarget`, follows with `Target.attachToTarget` using `flatten: true`, and stamps the returned `sessionId` onto every later command so the browser routes it to that target.
  Reading the kind off the protocol rather than inferring it from a failed command is deliberate: Chrome accepts `Target.createTarget` on a page-level connection too and opens a second tab, so anything short of a definite answer would risk moving a configuration that points at a specific page onto a blank one.
  Anything other than `browser` — including an endpoint that does not implement `Target.getTargetInfo`, and any failure of the query itself — stays on the page-level path, reconnecting first so a server that drops the socket on an unknown method cannot leave the caller worse off than before the query was sent.
  A dropped CDP socket now fails the commands still waiting on it instead of leaving them to time out: the reader loop answers them with `CDP connection closed` when it exits, so a dead connection is reported as itself rather than as a 30-second stall per in-flight command.
  Both cleanup branches are pinned by tests that fail when the close call is neutered, since a leaked tab is invisible to an assertion on the returned error alone.
  A tab is now closed on a failed attach whichever way it was created — `Target.closeTarget` for one created over CDP, `/json/close/{id}` for one discovered over HTTP — since both leave a tab that no session will ever track.
  A target created this way is closed with `Target.closeTarget` rather than the `/json/close/{id}` route used for HTTP discovery, which does not exist on a `ws://` endpoint, and a handshake that fails partway closes the target it already created rather than abandoning a blank tab that nothing would reap (#6617) (@nevgenov)
- Make the page-extraction cap operator-configurable as `[browser] max_content_chars`, defaulting to the 50,000 characters it was hard-coded at, and report the pre-truncation length in the marker so the model can tell how much it is missing rather than only that something was lost.
  A mainstream Wikipedia article overruns the old cap, and the compile-time constant was the only extraction-adjacent limit `BrowserConfig` did not expose while carrying `timeout_secs`, `idle_timeout_secs`, `max_sessions` and the viewport dimensions as knobs — despite being the one most coupled to a deployment-specific fact the project cannot know, the context window of the model behind the agent.
  The marker now counts against the cap: the script cuts far enough back that content plus `... (truncated, N chars total)` lands within the limit, so an operator sizing the value to a context window gets a real ceiling rather than one the marker silently overshoots.
  `EXTRACT_CONTENT_JS` is no longer a `LazyLock` — a process-wide singleton over a config-dependent value would serve whichever cap the first extraction observed to every later one, so the script is built per call from a `str::replace` on a ~2 KB template, on a path that then makes a CDP WebSocket round trip.
  Like every other `[browser]` field the value takes effect on daemon restart, because `BrowserManager` captures `BrowserConfig` by value at boot (#6687) (@houko)
- Add per-PR changelog fragments under `changelog.d/`, so writing a changelog entry no longer means editing the one file every other open PR is also editing.
  Every PR appended its bullet to the single `## [Unreleased]` section of `CHANGELOG.md`, which made a merge conflict certain between any two concurrent PRs and carried no information when it happened — both sides were correct and the resolution was always "keep both".
  It bit hardest on fork PRs, where the maintainer cannot rebase the contributor's branch at all.
  A fragment is one file holding one bullet body without the leading `- `, in the section directory matching its `### ` heading (`added/`, `fixed/`, `changed/`, `security/`, `documentation/`), so two PRs never touch the same file and the conflict is structurally impossible rather than merely rarer.
  `cargo xtask collect-fragments` folds fragments into `## [Unreleased]` and deletes the files it consumed; `cargo xtask release` runs that step before cutting the dated release section, which is what keeps the `awk` extractors in `release.yml` and `release-notify.yml` slicing an unchanged file shape.
  Assembly **appends** to a `### ` subsection that already exists rather than replacing it, and creates a missing one in the repo's existing order, so editing `## [Unreleased]` by hand keeps working and the PRs already doing that are unaffected — the existing 160 bullets are deliberately not migrated, since converting them would rewrite the exact lines those PRs are conflicting on.
  Fragments are sorted by file name within each section rather than read in filesystem order, because an unsorted directory read would make the assembled file depend on the order the fragments happened to be created in.
  A fragment that cannot be deleted after the fold fails the command by name rather than propagating a bare `Permission denied`, because `CHANGELOG.md` is already written by then and a silent partial delete would make the next run fold the same entry in twice.
  `scripts/check-changelog-attribution.py` holds a fragment to the same standard as an `[Unreleased]` bullet in all four of its modes, reusing the one `bullet_block_has_attribution` predicate so there is a single copy of the `(@user)` rule, and additionally rejects a fragment in an unrecognised section directory — assembly has no heading to render such a fragment under, so it would be dropped without a word and the entry would vanish from the release notes.
  The `pre-commit` hook's attribution check now also fires when a commit stages only a fragment, which is the normal case and would otherwise have gone entirely unchecked.
  The release commit stages `changelog.d` as a directory so the deletions the fold performs land in the commit; a per-file stage is a no-op for a path that no longer exists, and leaving them unstaged would keep the consumed fragments on `main` and fold the same bullets in again at the next release (#6628) (@houko)
- Add `GET /api/ready`, a public readiness probe that returns 503 when a dependency required to accept work is unavailable.
  `GET /api/health` could not serve this purpose: it returns 200 even while its body reports `status: degraded`, so a Kubernetes probe — which sees only the status code — could never remove a degraded pod from Service endpoints.
  Changing `/api/health` itself would have conflated liveness with readiness and restart-looped pods through recoverable storage incidents, so the two contracts are now separate endpoints (#6633) (#6638) (@houko)
- Add an officially supported single-replica Kubernetes deployment under `deploy/kubernetes/`, as Kustomize manifests plus operator documentation.
  The repository previously shipped Docker Compose only, leaving every operator to invent their own StatefulSet — and to rediscover on their own that SQLite WAL on shared storage and `replicas: 2` both corrupt state.
  `scripts/check-k8s-manifests.py` asserts the properties that fail silently when they regress (`replicas: 1`, the liveness/readiness split, `ReadWriteOnce`, credentials from Secrets rather than literals), and a new CI workflow boots the manifests in kind under enforced `restricted` Pod Security and proves `/data` survives pod replacement (#6635) (#6638) (@houko)
- Add `LIBREFANG_API_KEY` as an environment override for the API bearer token.
  `config.toml` lives inside the daemon's own writable data dir and is rewritten at boot, so it cannot be mounted from a Kubernetes Secret — leaving no way to supply `api_key` without baking the literal into an image.
  An empty value is ignored with a warning rather than treated as "clear the key", because a Secret key that exists but is unset would otherwise disarm bearer authentication on a non-loopback bind (#6635) (#6638) (@houko)
- Auto-detect a locally installed and logged-in EveryAPI CLI and expose it as an LLM provider without ever copying its relay key into a LibreFang-owned file.
  Credentials are resolved per request through EveryAPI's own credential-process command and refreshed once after an HTTP 401, so EveryAPI remains the authority for key selection, OAuth refresh, and region resolution.
  Explicit provider keys, URLs, and user suppression still take precedence over auto-detection, and `librefang doctor` reports the detected wiring and any conflicting configuration (#6641) (@houko)
- Add official EveryAPI partner links and documentation across the website footer/CTA, README, dashboard sidebar, and docs navigation, in English and Chinese.
  Correct the EveryAPI provider guide to use `everyapi auth login` and the current credential-process discovery flow, and point EveryAPI CLI references at the public `everyapi-ai/everyapi-ai` repository (#6646) (@houko)
- Thread the `language` and `prompt` parameters from `media_transcribe` / `speech_to_text` through to the multipart form Whisper-compatible providers actually receive, instead of reading and discarding them.
  `language` was advertised in both tools' schema and silently dropped, so the provider always fell back to auto-detection regardless of what the caller asked for — a misdetected language does not error, it returns fluent, plausible, wrong text.
  `prompt` is genuinely additive: it supplies domain vocabulary and proper nouns the model would otherwise transcribe as phonetic near-misses, and improves punctuation and casing on long recordings.
  Both follow the precedent `tool_text_to_speech` already used for `language`: the per-call value wins, and `[media] audio_language` / `audio_prompt` are the new operator-configured fallback for calls that omit either field.
  Only the whisper-protocol provider arms (Groq, OpenAI, MiniMax, Fireworks, Together, SiliconFlow, and any `[media.custom_stt]` self-hosted endpoint) receive these — Gemini and ElevenLabs are separate provider contracts with no equivalent parameter.
  An install that sets neither field sees a byte-identical request to before either parameter existed (#6678, #6683) (@houko)

### Fixed

- Drive the dashboard's per-channel status indicator from the sidecar supervisor's real liveness instead of from message traffic, and surface the fields it reads on `GET /api/channels`.
  The indicator was `msgs_24h > 0 ? "running" : "idle"`, so a healthy-but-quiet channel rendered grey and a channel that died after handling messages rendered green; `ChannelStatus.connected` and `last_error` were maintained by the supervisor all along but were not present on the payload at all.
  Configured rows now carry `connected`, `started_at`, `last_message_at`, `messages_received`, `messages_sent`, `last_error`, and a `supervised` flag that says whether an adapter is registered for that instance name at all, all read per sidecar instance.
  The card maps those onto seven states — not started, starting, connected, active, degraded, stopped, failed — with the state spelled out as visible text and folded into the card's `aria-label`, since the card's own label otherwise overrides its contents for assistive tech and the colour would be the only carrier of meaning.
  `connected` with a `last_error` reads as degraded rather than healthy or dead, because the supervisor sets `last_error` on failure and never clears it, not even on the successful respawn that follows.
  A configured channel with no registered adapter reads as amber rather than grey: grey is what made a dead bot look benign in the first place, and the API layer genuinely cannot distinguish "start failed and the registration was rolled back" from "never started".
  The mapping lives in a shared `src/lib/channelLiveness.ts` so the Comms page's channel cards render the same verdict from the same payload rather than repainting config presence as an online badge.
  The `librefang channel list` table gains CONNECTED and IN/OUT columns plus a per-channel error footnote, fed the raw supervisor fields rather than a second copy of the state mapping (#6606) (@houko)
- Stop presenting the channels page's 24h message count as per-bot traffic when it is a per-channel-type aggregate.
  `usage_events.channel` stores the channel *type*, so the handler's `msgs_24h.get(channel_type).or_else(|| msgs_24h.get(name))` always hit on the first lookup and the per-instance fallback was unreachable: on a host running six Telegram sidecars every card reported the same number, the total across all six, and because the status dot was derived from it the whole page turned green whenever any one bot saw traffic.
  Re-keying that column per instance is not available: it is written from `SenderContext.channel`, which the bridge derives from the `ChannelType` on the inbound message (the instance name never reaches it) and which also feeds `SessionId::for_channel(agent, channel)` and the auth `identify(&channel, …)` binding, so re-pointing it at the instance name would silently re-derive every existing channel session.
  So the figure is now published as `msgs_24h_channel_type` alongside the `channel_type` it covers, the unreachable fallback is gone, and the dashboard shows it in the details drawer captioned with its actual scope instead of on the card where it read as this bot's traffic.
  Per-instance traffic comes from the supervisor's own `messages_received` / `messages_sent` counters, labelled as since-adapter-start because they survive supervised restarts and are not a 24h figure.
  `UsageStore::channels_msgs_24h_bulk` is renamed to `channel_type_msgs_24h_bulk` so the grouping is not misread again at the call site (#6606) (@houko)
- Render a failed `GET /api/channels` as an error on the channels page instead of the "no channels configured" empty state, which made an unreachable daemon look like a clean install on the page whose whole purpose is now health signalling (#6606) (@houko)
- Merge instead of replace in `PATCH /api/agents/{id}/identity`, so a partial body no longer nulls the identity fields it omits.
  The handler built a fresh `AgentIdentity` from the request alone with no read of the stored one, so `PATCH {"emoji": "X"}` silently discarded `avatar_url`, `color`, `archetype`, `vibe` and `greeting_style` and returned `200` — while the sibling `PATCH /api/agents/{id}/config`, which writes the same six fields, merged them correctly.
  Two PATCH endpoints on one resource with opposite semantics is the actual defect, so the six-field merge is now a single shared `merge_agent_identity` helper that both handlers call rather than a copy in each; an integration test asserts the two routes produce the same stored identity for the same partial body.
  Neither route can set a field back to `null`, since `null` already means "not provided"; sending an empty string stores an empty string, which is the closest thing to clearing one, and that is now stated in the endpoint's OpenAPI description (#6608) (@houko)
- Name the `tool_allowlist` entries that cannot take effect in the response of `PUT /api/agents/{id}/tools`, instead of accepting them silently.
  The kernel applies `tool_allowlist` after `capabilities.tools` as a `retain`, so it can only narrow: an entry naming a builtin or skill tool the declared set excludes grants nothing, yet the write returned `200`, a subsequent `GET` round-tripped the value, and the only way to discover the tool was never granted was to inspect a session export.
  The resolution semantics are deliberate and unchanged — the response now carries an optional `warnings` array naming each inert entry, and the OpenAPI description for the endpoint and for the field says that `tool_allowlist` narrows rather than grants.
  A request that submits only `capabilities_tools` is checked too, because narrowing the grant surface is itself a way to silence a stored allowlist entry, and the operator who just issued that request is the one who needs to hear about it; a request that touches neither field (blocklist only) stays quiet about whatever was already stored.
  Only provably inert entries are reported, because a false warning on a working configuration would be worse than the silence: the check is skipped entirely when `capabilities.tools` is unbounded (empty or `*`), and it never flags a glob (a later skill install or MCP connect can make it match), an `mcp_`-namespaced entry (MCP tools bypass `capabilities.tools`), or a self-evolution tool (injected regardless of what the manifest declares).
  The declared side is glob-evaluated rather than string-compared, so `capabilities_tools = ["file_*"]` with `tool_allowlist = ["file_read"]` is correctly left alone (#6609) (@houko)
- Render every `approval_audit.decision` value distinctly in the dashboard's Approvals History table instead of labelling four of the six values it can hold "Edited".
  `ApprovalsPage.tsx` branched on `approved` / `approve` and `rejected` / `reject` and let everything else fall through to a yellow pencil "Edited" badge — the same rendering as a genuine modify-then-approve — so on the reporter's host 46 of 56 audit rows (28 `pending`, 18 `timed_out`) claimed an operator edit that never happened.
  `timed_out` means nobody answered before the timeout expired and `pending` is the submission marker written before any decision exists; on an approval audit trail those are the opposite of the operator involvement the badge asserted.
  A `pending` row is not evidence that the request is still open — resolution inserts a second row instead of updating the first one — so every one of the reporter's 28 `pending` rows belongs to a request that has since closed; that data-model quirk is untouched here and raised on the PR.
  `denied` was mislabelled by the same fall-through, which the issue does not mention: `ApprovalDecision::as_str` writes `denied`, but the branch only matched the `rejected` / `reject` spellings that `routes/approvals.rs` uses on sibling shapes, so every genuinely-denied request rendered as an operator edit too — of the six values the daemon can write, only `approved` and (coincidentally) `modify_and_retry` were labelled correctly before this change.
  Each of the six values the daemon writes now carries its own label, icon and theme colour — `timed_out` neutral with a clock, `pending` in-progress, `modify_and_retry` the actual "Edited" case, `skipped` distinct — and every state pairs that colour with label text and an `aria-label` naming the decision, so the trail is readable without colour perception.
  An unrecognised value renders the raw string (or an explicit "unknown status" when the field is empty) rather than borrowing another decision's label, so a variant a newer daemon adds degrades visibly instead of becoming a false record.
  The frontend type is narrowed from `string` to a `KnownApprovalDecision` union plus an explicit escape for unknown values, the presentation table is a total `Record` over that union so adding a member without giving it a label is a compile error, and the entry interface picks up the `second_factor_used` field the Rust struct already carries.
  `ApprovalAuditEntry::decision` stays a `String` on the Rust side deliberately: the column is read back for rows written by any past version and `query_audit` drops rows it fails to deserialize, so a strict enum would silently shorten a security audit trail on one legacy value — the value set is documented on the field and pinned by a test instead (#6607) (@houko)
- Expose `external_auth.require_email_verified` and the six missing `OidcProvider` fields (`auth_url`, `token_url`, `userinfo_url`, `jwks_uri`, `audience`, and the per-provider `require_email_verified` override) in `GET /api/config`, while keeping every one of them non-writable.
  `require_email_verified` is the #3703 mitigation — it rejects a login whose ID token does not carry `email_verified = true`, which is what stops an unverified address in an `allowed_domains` domain from inheriting that domain's authorization — and it is deliberately absent from the `POST /api/config/set` allowlist so an Owner-role caller with a leaked API key cannot switch it off.
  Omitting it from the read side too was the wrong asymmetry: an operator had no way to confirm the protection was active without shell access to read `config.toml`, and with the provider endpoint overrides hidden as well, a non-OIDC provider's explicit `auth_url` / `token_url` / `jwks_uri` were invisible from any surface.
  Nothing newly exposed is secret-bearing: `client_secret_env` names the environment variable the secret is read from and `client_id` is the public half of the client registration, both of which were already emitted, and the secret itself never lives in config.
  The read/write parity guard added in #6604 cannot catch this class, because it enforces `writable ⊆ readable` and a field that is intentionally non-writable sits outside that invariant by construction (#6605) (@houko)
- Remove `ui.theme`, `ui.locale`, `ui.timezone`, and `ui.language` from the `POST /api/config/set` allowlist, where they had accepted writes that were silently thrown away since #4113.
  `KernelConfig` has no `ui` field and never had one, so the write path validated the dotted path against the allowlist, edited `config.toml` through `toml_edit` keyed by it, and dropped a `[ui]` table on disk that the next load discarded — `KernelConfig` does not set `deny_unknown_fields`, so neither the post-edit parse nor the reload could reject it.
  The caller received a success status for a change that was never applied and never read back, and `GET /api/config` reported `ui` as null.
  Nothing ever posted them: the dashboard keeps theme, language, and sidebar state in browser `localStorage` through zustand's `persist` middleware (key `librefang-ui-storage`), which is why four dead paths went unnoticed for three months.
  The unit test that pinned them as writable now pins them closed (#6605) (@houko)
- Add `every_writable_allowlist_entry_has_a_backing_config_field`, the mirror of #6604's read/write parity guard: every entry in the `POST /api/config/set` allowlist must name a field that actually exists on `KernelConfig`.
  The parity guard derives its candidate paths *from* a serialized config, so a path naming a field that does not exist is structurally invisible to it — which is why the four `ui.*` entries survived it.
  The oracle is the schemars-derived JSON Schema rather than a serialized config value, because a value walk cannot see a field whose `#[serde(skip_serializing_if = …)]` predicate holds for its default and 63 of those attributes exist in `config/types.rs`, several on writable paths (`exec_policy.allowed_env_vars`, `budget.providers`, `tool_invoke.allowlist`), so a value-based oracle would report real fields as dangling.
  The guard needs no exclusions — with the four `ui.*` entries gone, all 29 remaining exact paths and all 54 section prefixes resolve — and it carries the same sanity floor as its sibling plus negative controls asserting the resolver still rejects `ui.theme` and an invented section, since the failure mode of a path resolver is over-permissiveness rather than emptiness.
  The two allowlists moved from `const` items inside `is_writable_config_path` to module scope so the guard reads the real lists instead of restating them (#6605) (@houko)
- Share one tempfile sequence between the two `config.toml` writers in `routes/sidecar_toml.rs`, so a concurrent channel configure and remove can no longer clobber each other's write.
  `upsert_sidecar_block` and `remove_sidecar_block` each declared a function-local `static SEQ: AtomicU64 = AtomicU64::new(0)` while both formatted into the same `.config.toml.tmp.{pid}.{seq}` namespace, so the first call to either in a process minted the identical path and the two could write and rename each other's tempfile — landing one request's document at the other's target or losing it outright.
  The comment above the first counter already claimed it "guards against concurrent threads within this process (e.g. parallel tests, or two HTTP handlers racing on the same config file)", which is the guarantee two independent counters cannot provide.
  Both writers now go through one `atomic_write` helper backed by a module-level counter, and a test draws names concurrently from eight threads and asserts they are all distinct (#6605) (@houko)
- Store the EveryAPI gateway base URL with its `/v1` segment when connecting from the dashboard, matching what the CLI writes.
  `EVERYAPI_PROVIDER.defaultBaseUrl` was `https://api.everyapi.ai` on a doc comment claiming the drivers append the path themselves, which no driver does: the OpenAI-compatible driver builds `{base_url}/chat/completions` and the daemon's catalog refresh builds `{base_url}/models`.
  A dashboard connect with the gateway field left blank therefore registered a provider whose model fetch 404s, and because that failure is only `warn!`-logged and then throttled per base URL, the entry sat configured-with-zero-models indefinitely while the identical `librefang models connect everyapi` flow worked.
  The connect action landed in #6586 (#6602) (@houko)
- Refresh the EveryAPI catalog on both channel read paths, which #6583 left unwired.
  `list_models_by_provider` and `list_models_text` in `channel_bridge.rs` each refreshed for `openrouter` and fell through for `everyapi`, so a Telegram or Slack user listing or picking a model saw whatever snapshot the catalog last happened to hold while every `providers.rs` read path refreshed correctly.
  `list_models_text` spans every provider and so refreshes both catalogs unconditionally; the per-provider picker only refreshes the one being asked about (#6602) (@houko)
- Apply `tool_allowlist` when rendering MCP tool grants in the dashboard's Tools tab.
  The tab mirrored `tool_blocklist` only, but the kernel's Step 4 filter runs after MCP tools have joined the candidate set, so a non-empty allowlist naming no `mcp__*` glob strips the entire server (#6495).
  An allowlist-filtered server rendered as fully granted — the display asserted an agent could call tools the kernel had already removed.
  The Tools tab MCP display landed in #6578 (#6602) (@houko)
- Stop following symlinks in the two skill installers #6581 left untouched.
  That PR hardened `librefang_skills::marketplace::copy_dir_recursive` but the API route's copy helper and the CLI's `copy_dir_recursive` read the same registry checkout, the former dereferencing links through `std::fs::copy` and the latter branching on `Path::is_dir()`, which follows them.
  A symlink planted in a registry skill therefore still copied the target's real contents — or an entire external tree — into the installed skill (#6602) (@houko)
- Refuse `service install --system` on macOS while a per-user LaunchAgent is still installed.
  Both carry the `ai.librefang.daemon` label and the same state directory, so installing the LaunchDaemon over the agent produced a login-time respawn loop: the agent's `start --foreground` finds the daemon already holding the port, exits non-zero, and `KeepAlive` relaunches it.
  The check runs before the ownership handover and the plist write, so a refused install leaves nothing behind.
  `--system` landed in #6584 (#6602) (@houko)
- XML-escape the paths interpolated into both macOS plists.
  APFS permits every byte but `/` and NUL, so a volume named `Backup & Media` or a `LIBREFANG_HOME` containing `<` produced an ill-formed plist that launchd rejects wholesale, while the install path still reported it as written.
  The LaunchDaemon renderer landed in #6584 and the LaunchAgent one predates it (#6602) (@houko)
- Correct the NixOS module's non-loopback bind assertion, which accepted `environmentFile != null` as proof of authentication.
  No environment variable feeds `api_key` — it is read from `<stateDir>/config.toml` only — so the documented off-host recipe, whose environment file holds provider keys, passed `nixos-rebuild` and then deployed a unit the daemon refuses to start.
  The message now names the only credentials the unit environment can actually supply (`LIBREFANG_DASHBOARD_USER` / `LIBREFANG_DASHBOARD_PASS`), a new `authConfiguredExternally` option covers a `config.toml` maintained out-of-band, and the `LIBREFANG_ALLOW_NO_AUTH` disjunct now checks the value against the same five spellings `allow_no_auth_env()` accepts instead of merely testing for the key's presence.
  The module landed in #6582 (#6602) (@houko)
- Stop hand activation from fabricating a `mode = "full"` shell exec policy for every hand whose tool list contains `shell_exec`.
  `activate_hand_with_id` hardcoded `ExecSecurityMode::Full` whenever the hand's agent section declared no `[exec_policy]` of its own, so activating a marketplace hand silently granted unrestricted shell execution and — because `Full` also short-circuits the approval queue in `tool_runner::dispatch` — skipped the operator's `require_approval` globs entirely.
  The materialized policy is now inherited from the live global `[exec_policy]`, mode included; a hand that genuinely needs elevated exec still opts in by declaring its own `[exec_policy]`, which activation continues to respect verbatim.
  **This changes behaviour on a stock install, not only on a hardened one.** `ExecPolicy::default()` is `mode = "allowlist"` with an empty `allowed_commands` and 18 read-only `safe_bins` (`sleep`, `cat`, `head`, `wc`, `date`, `echo`, …), so a daemon with no `[exec_policy]` section at all now gives its hands that allowlist rather than unrestricted shell.
  A shell-using hand such as `devops` will therefore find `docker`, `kubectl`, `git`, and `cargo` rejected until the operator either adds them to `[exec_policy] allowed_commands` (or widens `mode`) or the hand declares its own `[exec_policy]` — the trade-off is deliberate: an operator who never wrote a shell policy has not consented to unrestricted third-party shell execution.
  The historically generous 300s / 120s timeouts survive as a floor rather than being replaced by the 30s `ExecPolicy::default()` values, because a timeout is not a security property and inheriting them wholesale would cut long-running hand commands short for reasons unrelated to the fix.
  The guard reads `capabilities.tools` and matches the `*` wildcard as well as `shell_exec`, because `spawn_agent_inner`'s own fallback promotes any manifest that still carries no `exec_policy` — leaving one unset on a manifest that fallback would match relocates the escalation rather than removing it.
  Activation also logs the resolved mode and where it came from, so an escalation is visible at activation time instead of only through the per-call warning much later.
  Scope: this covers the hand-activation path only, which leaves a divergence worth knowing about.
  `spawn_agent_inner` and the boot-time restore loop still promote a *standalone* agent to `Full` on the identical trigger, so post-fix the same `shell_exec` tool is unrestricted on a standalone agent and allowlisted on a hand agent; that promotion is deliberate today (it has its own regression test) and changing it needs an `allowed_commands` migration story, so it is left for a separate decision (#6594) (@houko)
- Require an explicit declaration before a hand's agent gets a wake-up cycle, instead of inferring autonomous ticking from `max_iterations`.
  `max_iterations` is the agent-loop iteration cap — `librefang-runtime` resolves it from `manifest.autonomous.max_iterations`, and the flat HAND.toml field exists only to carry it — but activation turned `autonomous.is_some()` into `ScheduleMode::Continuous` with `heartbeat_interval_secs` as the interval, so a hand asking for a loop-depth cap of 80 got a permanent 30-second wake-up cycle it never declared and could not find in any file.
  A role now leaves `reactive` only if its own section says so: an explicit `schedule`, honoured verbatim and still the only route to the `periodic` cron and `proactive` variants, or an explicit `[autonomous]` block, which schedules the role continuously at that block's own `heartbeat_interval_secs`.
  A role declaring nothing but `max_iterations` — at any `[metadata] frequency` — stays reactive and keeps its cap, and `[metadata] frequency` goes on being catalog-display metadata that the kernel does not read.
  Distinguishing an author-written `[autonomous]` block from the one synthesized to carry `max_iterations` is impossible after deserialization, since both land in the same `Option<AutonomousConfig>`, so the decision moved into `librefang-hands` where the raw TOML table is still available and an `autonomous` key means somebody typed one; the schedule-rewriting block in `hands_lifecycle.rs` is deleted outright, leaving hand roles to honour `schedule` exactly the way `spawn_agent_inner` already honours it for a standalone agent.
  **Hands that were implicitly ticking will stop, and that is a behaviour change, not only a bug fix.**
  Every role that wants to keep its loop needs an explicit `[autonomous]` block (or an explicit `schedule`) added to its HAND.toml; across the bundled registry that is 20 of the 59 roles — every role that carries `max_iterations`, since not one declares an `[autonomous]` block or a `schedule` today — including all four `wiki` roles and both loop-running `devops` roles (`main` and `implementer`).
  `devteam`'s `pm` / `engineer` / `qa` are the only roles that inherit through `base =`, from the `planner` / `coder` / `code-reviewer` templates in the registry repository; none of those three declares `max_iterations`, `[autonomous]`, or `schedule`, so those roles were already reactive and are unaffected either way.
  The shipped registry lives in its own repository, so those manifests are a follow-up there rather than something this change can carry, and `devops`'s own system prompt asserts "The Hand is already `frequency = \"continuous\"`, so this Phase fires once per turn" — prose that needs revisiting alongside the manifest.
  No role in the registry starts ticking: not one of the 59 declares an `[autonomous]` block or a `schedule` today, so the new condition is unsatisfied everywhere and the set of roles with a background loop only shrinks.
  The rule is not a subset of the old one in the abstract — a flat-format role that did write an explicit `[autonomous]` block would newly tick, since that block used to be dropped by the flat-format parse entirely (the bullet below) — but no such role exists to be affected.
  `frequency = "reactive"` also stops being a TOML parse error — #6595 reports an operator reaching for that spelling while hunting the ticks — though as catalog metadata it has no effect on scheduling either way (#6595) (@houko)
- Key the heartbeat monitor off an agent's `schedule` rather than the presence of `[autonomous]`, so an agent that nothing wakes is no longer flagged `BecameUnresponsive` for sitting idle.
  The comment above the check already said "skip passive agents"; `autonomous.is_some()` was only ever a proxy for that, and it held while activation derived `Continuous` from the mere presence of the field.
  Once `autonomous` means "carries a loop cap, and possibly guardrails" — which says nothing about whether the agent wakes itself — the proxy inverts: a role declaring `max_iterations` with no `[autonomous]` block and no `schedule` becomes the common shape, 20 of the 59 roles in the registry, and every one of them would be reported unresponsive for behaving exactly as configured.
  Four pre-existing heartbeat tests built their agents with a default reactive schedule; the two that expected the agent to be checked would have failed, and the two that expected it skipped would have started passing for the wrong reason, so all four now declare a continuous schedule and still exercise what their names claim (#6595) (@houko)
- Carry `schedule`, `[autonomous]`, and `[exec_policy]` through the flat HAND.toml agent format instead of dropping them.
  `parse_single_agent_section` tries `LegacyHandAgentConfig` first for any agent section without a `[model]` sub-table — the shape every hand in the registry uses — and that struct has no `deny_unknown_fields`, so all three keys were deserialized into a struct that did not declare them and vanished with no diagnostic.
  Each is a documented per-hand opt-in, so the opt-in was unreachable for exactly the hands that need it: such a hand could not pin a schedule of its own, could not declare the autonomy it wanted, and could neither tighten nor loosen its exec policy.
  This is what makes the explicit declarations above reachable at all for a flat-format hand, and an explicitly declared `[autonomous]` block now wins over the one synthesized from `max_iterations`.
  One consequence worth knowing: a malformed value under any of the three keys now fails the parse and the hand is skipped by `reload_from_disk`, where it used to be ignored silently — no hand in the registry declares any of them today, so nothing regresses now (#6594, #6595) (@houko)
- Expose every config field on `GET /api/config` that `POST /api/config/set` accepts, so a setting the dashboard just saved stops reading back as "not configured".
  The response body is hand-enumerated section by section, and a long tail of writable fields had never been added to it — among them `browser.enabled` / `browser.cdp_endpoint`, `media.image_model` / `media.custom_stt`, `tts.custom`, `approval.totp_grace_period_secs`, `web.timeout_secs`, `exec_policy.allowed_env_vars`, and the whole `[terminal]` section, which the dashboard declares a page for but could never populate.
  The `[channels]` file-transfer limits are added in the same pass without being part of that count: `channels.` accepts depth-2 paths only, so its depth-1 scalars are edit-on-disk, but the dashboard declares a `channels` section and had nothing to render in it.
  Deriving the body from `Serialize` instead was rejected on evidence rather than effort: the dashboard's chat page reads `media.stt_available` and `web.search_available`, keys no serializer would ever emit, and the body redacts secrets in place (`api_key`, `network.shared_secret`, `vertex_ai.credentials_path`) rather than dropping them, so a wholesale switch would have broken working UI and needed an exhaustive deny-list to stay safe.
  The hand-written shape is kept and a guard added instead: `config_read_write_parity_tests` walks a serialized `KernelConfig`, filters the paths through the real `is_writable_config_path` — not a copy of its allowlists, which would become the next thing to drift — and fails the build for any writable leaf the read path omits.
  `redacted_config_json` is split out of the handler so that guard runs without booting a kernel (#6596) (@nevgenov)
- Emit the serde spelling rather than the Rust variant name for enum-valued config fields on `GET /api/config`, so their dashboard dropdowns show the configured value instead of appearing unset.
  `mode`, `reload.mode`, `exec_policy.mode`, `broadcast.strategy`, `docker.mode`, `docker.scope`, and `web.search_provider` were rendered with `format!("{:?}", …)`, which yields `Allowlist` and `DuckDuckGo` where `config.toml`, the write path, and the schema's own `select` options all say `allowlist` and `duck_duck_go`.
  The dashboard matched the value against the options it had just been handed, found nothing, and rendered an empty control for a field that was set.
  Four sibling fields in the same handler already went through `serde_json::to_value`, so this restores the house style rather than introducing one; an integration test now rejects any upper-case character in those values, which catches a new field reintroducing `Debug` formatting and not merely the seven that were fixed (#6596) (@nevgenov)
- Correct the API reference for `GET /api/config` and `POST /api/config/set` in both locales.
  The documented response example shared not one key with the real body — it advertised `default_provider`, `listen_addr`, `api_key_set`, and `channels_configured`, none of which the endpoint has ever returned — and the `config/set` example named its field `key` where the handler reads `path`, so a copied request failed with `missing 'path' field`.
  The same example also used a bare table name as the path, which the write allowlist rejects with `403`; it now uses a leaf, and the page states the three-segment limit and the edit-on-disk exclusions (#6596) (@nevgenov)
- Stop `[approval] trusted_senders` from waiving human approval on every tool a connected MCP server exposes.
  `classify_risk` matched a closed list of built-in names, so any `mcp_*` tool fell into the `Low` default and the trusted-sender branch of `requires_approval_with_context` returned before the operator's `require_approval` globs were consulted — the globs were dead config for exactly the tools least able to be audited, since server-side tool code is third-party and its effects are not enumerable from a name.
  Real MCP servers ship tools that move money or take a plaintext credential as an argument, so `mcp_*` now grades as high risk and the bypass stays scoped to the built-ins it was written for; an operator who wants an MCP tool unattended can still allow it per-channel or per-user (#6592) (@houko)
- Refuse to replace a populated EveryAPI provider entry when the gateway's model list cannot be fetched.
  Both persistence paths rewrite `providers/everyapi.toml` wholesale, so a transient outage on a re-run downgraded a working catalog to zero models and still reported success.
  A first run, which has nothing to lose, still proceeds and fills the models in later (#6583) (@houko)
- Distinguish a rejected relay key from an unreachable gateway when fetching the model list.
  Both collapsed into the same outcome, so a revoked key printed "check that the gateway is reachable" and was then saved as if valid; a 401 or 403 now stops before anything is written and points at `everyapi login` (#6583) (@houko)
- Stop the daemon-write path from aborting the process on the failure its own fallback exists for.
  `write_provider_via_daemon` went through a helper whose transport-error arm calls `std::process::exit(1)`, so an unreachable daemon killed the command before the direct file write could run.
  A daemon that parses the payload and *rejects* it is now terminal rather than falling back, because the registry route deletes the file it refused and rewriting it would leave a definition that fails to parse on every boot (#6583) (@houko)
- Pin the gateway base URL into `[provider_urls]` when `--set-default` switches the default provider.
  The default route persists only provider, model and `api_key_env`, while the daemon resolves the boot-time driver from `default_model.base_url` or `provider_urls` several hundred lines before the model catalog that holds the address is built — so the next boot produced a default driver with no endpoint (#6583) (@houko)
- Declare the media capabilities the synthesised EveryAPI provider actually implies.
  `media_capabilities` was hardcoded empty while the same run registered image, audio and video entries, leaving all of them unreachable through the media paths despite appearing in the catalog (#6583) (@houko)
- Rank the `--set-default` choice by capability instead of taking the first model by id.
  Entries are id-sorted for output determinism and ASCII orders uppercase first, so the CLI picked `MiniMax-M3` over `claude-opus-5` while its own help text promised "this gateway's best model" (#6583) (@houko)
- Treat a published context window as evidence of a text model when a gateway row omits `supported_endpoint_types`.
  The empty case defaults to video because every observed empty row is a video model, but the field is optional — a gateway that stopped sending it would have had its chat models registered as video, exempt from the token-limit validation and unusable for chat (#6583) (@houko)
- Point the no-daemon `--set-default` message at re-running connect rather than `librefang models set`.
  That command writes only `default_model.model`, leaving the previous provider and `api_key_env` in place — a combination that resolves the wrong driver for the chosen model (#6583) (@houko)
- Classify an OpenAI-compatible `insufficient_quota` response as a billing error rather than a malformed request.
  OpenAI signals an exhausted account with that code on a 403, so every OpenAI-compatible endpoint does too, but it was not in `BILLING_PATTERNS` and fell through to the generic 4xx arm.
  The operator was told to check their request format while the real cause was an empty account, and because `is_billing` stayed false the long billing cooldown never applied, so a provider with no funds left kept being retried.
  Affects every OpenAI-compatible provider, not just gateways (#6583) (@houko)
- Stop `librefang doctor` reporting a healthy EveryAPI wiring when the relay key is gone.
  The check returned `Pass` as soon as the provider file existed, so `credentials_usable` was never consulted — after `everyapi logout`, a key rotation, or a revocation it stayed green while every request through the gateway failed authentication.
  It now warns and names the remediation, since a provider entry is a file that persists while the credential behind it is not (#6583) (@houko)
- Point skill search and install at the synced registry checkout so the FangHub surface works on a stock install: every remote path targeted `github.com/librefang-skills`, an organization that does not exist, so `librefang skill search` got `422 Unprocessable Entity` (GitHub's answer to an `org:` qualifier naming a missing org) and `librefang skill install <name>` got a 404 for every skill.
  The real source is `~/.librefang/registry/skills/`, synced by `registry_sync` from `librefang/librefang-registry` and forge-agnostic (it honours `registry.registry_host`, so a Codeberg mirror works) — 61 skills, `web-search` among them.
  `MarketplaceConfig` gains a `registry_dir`; `search_registry` scans that checkout (name and description, case-insensitive, sorted) and `install` copies from it before falling back to the GitHub-releases path, normalizing `SKILL.md` into a native `skill.toml` and running the same supply-chain audit as a remote bundle.
  `librefang skill install ./dir` now accepts a `SKILL.md`-only directory, which the registry loader already auto-converted but the CLI rejected with "No skill.toml found", making every registry skill uninstallable by path.
  `librefang skill install <git-url>` is implemented rather than advertised-only: the CLI's own help text documents the form, but a URL fell through to the name-based install, which pasted it into `{org}/{name}` and requested a nonsense URL.
  `GET /api/marketplace/search` reads `SKILL.md` as well as `skill.toml` — it looked only for the latter, so it returned an empty list on every install, unlike the sibling `GET /api/skills/registry` which had already solved this (#6569) (@houko)
- Fix the dashboard's agent Clone button, which failed on every click: `POST /api/agents/{id}/clone` requires `new_name` (no serde default, and the request struct is `deny_unknown_fields`), but the client posted `{}` and nothing in the click handler ever collected a name, so the backend answered 422 and the UI showed a generic "Failed to clone agent" toast.
  Clone now opens a small dialog that pre-fills `<source>-copy`, lets the operator edit the name, and carries the `include_skills` / `include_tools` toggles the endpoint already accepted; `cloneAgent()` and `useCloneAgent()` take the payload instead of posting an empty body (#6566) (@houko)
- Make the agent Tools tab reflect MCP servers granted through the `mcp_servers` allowlist, which it previously reported as "AVAILABLE / click to assign" with 0 tools recognised even while the agent was actively calling those tools.
  The tab derived every group's assigned/available state from `capabilities_tools` alone, but the kernel grants MCP tools through `mcp_servers` and explicitly skips the declared-tools filter for them (`available_tools`, Step 3) — `capabilities_tools` governs builtin tools only, and `tool_blocklist` is the per-tool exception mechanism.
  MCP group state now comes from `mcp_servers` / `mcp_servers_mode` (already emitted by `GET /api/agents/{id}`, just missing from the dashboard's `AgentDetail` type) minus `tool_blocklist`, with the blocklisted tools labelled inside the expanded group.
  MCP groups are also read-only on this tab now: `PUT /api/agents/{id}/tools` carries no `mcp_servers` field, so the old group toggle wrote MCP tool names into `capabilities_tools` where the kernel ignores them — it looked like it worked and changed nothing.
  The grant semantics are mirrored into a tested `src/lib/toolGrants.ts` (glob matching, MCP name normalization, grant-mode resolution) rather than re-derived inline (#6565) (@houko)
- Fix goal creation from the dashboard always failing: the create form seeds `parent_id` and `agent_id` as empty strings and posted them verbatim, so `POST /api/goals` read `Some("")` for the parent, looked for a goal literally named `""`, and answered `404 Parent goal '' not found` for every goal a user tried to create.
  The blank `agent_id` was persisted too, and later made `POST /api/goals/{id}/start` reject the goal with "Assign an agent to this goal before starting a run" on a goal the user never assigned.
  Both layers are fixed: the API now normalizes a blank (or whitespace-only) `parent_id` / `agent_id` to "absent" on create and treats it as the same clear-this-link signal as `null` on update, and the dashboard stops sending the empty fields at all.
  A non-blank `agent_id` that is not a valid UUID is now rejected up front with `400 Invalid agent_id` on both create and update, instead of being silently stored and only surfacing later as the same misleading "Assign an agent to this goal before starting a run" message on a goal that was in fact assigned.
  The six built-in goal templates rendering on every visit is by design — they are a static catalog, and `handleApplyTemplate` already posts `{title, description, status}` with no `parent_id`, so applying one was never affected (#6562) (@houko)
- Fix interactive menu button callbacks (`/models` provider and model pickers, "back to providers") being silently dropped on Telegram, which made every press a no-op with no reply and no error.
  The bridge's menu interceptor read the pressed menu's id from `metadata["message_id"]` with `as_str()` only, so it resolved `None` and hit an early `return` on both sidecars: the Rust adapter wrote the id as a JSON number (`serde_json::Value::as_str` never matches a number), and the Python adapter never puts the id in callback metadata at all — it sets the canonical top-level `message_id`, which the daemon stores as `ChannelMessage.platform_message_id`.
  The resolver now accepts a string or a number from metadata and falls back to `platform_message_id`, so both adapters work; the Rust adapter also emits the id as a string in both slots, matching the wire contract already used by `message_event`, by `SidecarMessageParams.message_id: Option<String>`, and by the Python adapter.
  Populating the top-level slot additionally stops the daemon from synthesising a random UUID for a callback's `platform_message_id`, which could never address a real Telegram message.
  The unresolvable case is now a `warn!` carrying both candidate values instead of a `debug!`, so a future regression of this class is visible rather than silent (#6564) (@houko)
- Honour `agent.toml: [model] context_window` on the paths that actually run a turn, so the field the runtime's own warning tells operators to set is no longer inert.
  All three execution paths — the non-streaming `execute_llm_agent`, the streaming turn, and the ephemeral `/btw` turn — resolved the window from the model catalog only, so an agent on a model the catalog does not know (a custom or proxied model reached through a `provider_urls` override) stayed pinned to the 8192-token `UNKNOWN_MODEL_CONTEXT_WINDOW` fallback no matter what the manifest said, and trimmed history aggressively on every turn.
  Only the read-only context report honoured the override, and its comment claimed to mirror "the same precedence chain the agent loop uses" — so the dashboard showed the operator's value while every real turn ignored it.
  The chain (manifest override, then catalog, then persisted session value) now lives in one `manifest_helpers::resolve_context_window` used by all four call sites; the compaction gate uses the same helper but deliberately skips the session level, whose stale value would otherwise rank above its 200K default window.
  A session `/model` switch clears `model.context_window`, since that number annotates the manifest's model and would otherwise size the budget from the wrong model; the streaming path also resolves the window *after* the session override is applied, where it previously read the pre-override manifest (#6568) (@houko)
- Stop `agent kill` / agent removal (the `purge_identity=true` / canonical-UUID purge path) from deleting the agent's rows in `audit_entries`, which broke the append-only WORM Merkle chain that `security verify` walks and made routine test-agent purges report a chain break indistinguishable from tampering.
  `execute_structured_agent_deletes` — the single canonical agent-scoped delete cascade — listed `audit_entries` alongside genuinely per-agent tables (memories, kv_store, sessions, …), so purging an agent opened a `seq` gap whose downstream `prev_hash` no longer resolved.
  The audit trail is now excluded from the cascade: its rows survive with their now-orphaned `agent_id` (an audit trail is supposed to record what happened, including to agents later removed), while the rest of the per-agent purge is unchanged; `approval_audit` (a separate flat table with its own time-based retention, not the Merkle chain) stays in the cascade. Remediating a pre-existing gap left by the old behavior — a non-destructive re-anchor / attestation path instead of the fully-destructive `security audit-reset` — is tracked separately as a follow-up feature (#6553) (@houko)
- Make `POST /api/skills/reload` report an honest result in Stable mode instead of always returning `{"status":"reloaded"}` on a no-op.
  When the skill registry is frozen (Stable mode) the kernel `reload_skills` previously bailed with only a `warn!`, so new skill directories added after boot were silently invisible and the HTTP handler still reported success — indistinguishable from a real reload without daemon-log access.
  The kernel now returns a `SkillReloadOutcome`: while frozen it refreshes the on-disk content of already-loaded skills via the freeze-safe `SkillRegistry::reload_skill` and reports any brand-new skill directories it is deliberately not loading (via a new `SkillRegistry::unloaded_on_disk_dirs`), and the handler surfaces `{"status":"partial"|"refreshed","frozen":true,"refreshed":[...],"skipped_new":[...],"detail":...}` so an operator can see that a restart is needed to pick up new skills; the intentional Stable-mode freeze boundary is unchanged (#6540) (@houko)
- Stop losing tool results in the 10–16 KB band, where a result was too small to spill to a recoverable artifact yet large enough to be lossily truncated by the sanitizer.
  Artifact spill (`spill_fresh_result`, default `spill_threshold_bytes = 16_384`) preserves an oversized result's full bytes as a recoverable stub, but the sanitizer's `strip_tool_result_details` independently hard-cut at a hardcoded `10_000` with a non-recoverable `...[truncated from N chars]` marker, so a result in `[10_000, 16_384)` never spilled and was truncated irrecoverably — defeating spill's documented intent to run "before `sanitize_tool_result_content` truncates".
  The sanitizer's size cut is now tied to `DEFAULT_SPILL_THRESHOLD_BYTES` (a new exported constant that `default_spill_threshold_bytes` also returns) so the two can't drift apart, making the cut a genuine last-resort fallback that only fires when spill did not run (spill disabled or the artifact write failed) rather than a dead band below the spill threshold; the size-independent base64-blob and injection-marker stripping still runs unconditionally (#6545) (@houko)
- Stop the skill/tool-result security scanner from flagging ordinary words as hardcoded secrets: the credential-prefix patterns (`sk-`, `ghp_`, `gho_`, `github_pat_`, `xoxb-`, `xoxp-`, `akia`) matched as free lowercased substrings via Aho-Corasick, so any word merely containing those characters (`task-`, `risk-`, `disk-`, `ask-`, `desk-`, `mask-`) tripped a false "Possible hardcoded secret" warning that `injection_guard::scan_tool_result` then escalated into a `[SECURITY WARNING …]` banner prepended to clean tool output.
  These seven prefixes are now token-anchored (the preceding character must be start-of-text or a non-token separator) and require a plausible key body after the prefix (a run of at least 12 key-ish `[a-z0-9_-]` characters containing a digit), mirroring the `starts_with` prefix check in `tool_runner::taint`; real leaked keys (`sk-` + high-entropy body, `ghp_…`, `AKIA…`, `xoxb-…`) still fire, while the non-prefix members of the group (`api_key`, `-----begin rsa`, …) keep matching as plain substrings (#6541) (@houko)
- Apply the `[approval] auto_approve = true` shorthand when the policy is installed into the `ApprovalManager`, fixing a silent no-op where the flag cleared nothing and every tool stayed gated.
  `ApprovalPolicy::apply_shorthands` (which clears `require_approval` when `auto_approve` is set) was only ever called from a unit test — the daemon-boot path (`ApprovalManager::new_with_db`) and the hot-reload path (`update_policy`) both installed `config.approval` verbatim, so an operator who set `auto_approve = true` to disable gating got no effect and the field's own doc comment ("clears the require list at boot") was false.
  The shorthand is now applied at all three policy-install entry points (`new`, `new_with_db`, `update_policy`), so `auto_approve` takes effect at boot and on `POST /api/config/reload`; the separate `trusted_senders` / `[[users]]` RBAC layering is documented above and unchanged (#6492) (@houko)
- Return a deterministic `409 Conflict` when a client resolves an approval that was already resolved, instead of a non-deterministic `400`-or-`404` that depended on whether the in-memory `recent` ring still held the entry.
  `ApprovalManager::resolve` reported "Already {decision} by {who}" (mapped to `400`) only while the resolution sat in the 100-slot buffer, and degraded to "not found" (`404`) once the buffer evicted it or the daemon restarted, so the same double-resolve returned different statuses depending on load and uptime.
  `resolve` now falls back to the durable `approval_audit` log to recognize an already-terminal request, and the api boundary maps the "Already …" verdict to a new typed `LibreFangError::Conflict` (`409`, code `conflict`) a client can act on, while a genuinely-unknown id still returns `404` and a missing second factor still returns `400` (#6492) (@houko)
- Stop `KnowledgeStore::delete_by_agent` from silently orphaning another agent's knowledge when a shared entity's first-writer agent is deleted (#6521).
  Entities are keyed on `(id, peer_id)` — not `agent_id`, which is only first-writer provenance — so a deterministic-id entity (a well-known org/person name) first written by agent A can be referenced by agent B's live relations; deleting every `agent_id = A` entity on A's deletion removed that shared row, and B's relations quietly stopped resolving (the JOIN just stopped matching — no error, data vanished from future reads).
  `delete_by_agent` now deletes A's relations wholesale (they are strictly per-agent) but only removes A's entities that NO surviving relation still references by id or name, keeping shared, still-referenced entities in place.
  Surfaced in review of #6519 (#6521) (@houko)
- Scope entities/relations created by the MCP `knowledge_add_entity` / `knowledge_add_relation` tools to the calling agent instead of the empty-string sentinel, so tool-created knowledge is no longer split-brained from the rest of the graph; `MemorySubstrate::add_entity` / `add_relation` hardcoded `agent_id = ""`, while the proactive-extraction write path and the agent-scoped read (`GET /api/memory/agents/{id}/relations` → `query_graph_scoped(.., Some(agent_id), ..)`) both key on the real agent uuid — so anything an agent stored via its own `knowledge_add_*` tools could never be returned by the agent-scoped relations endpoint or the dashboard relations view, and `delete_by_agent` never removed it (it orphaned on agent deletion); the caller agent id (already available at the tool dispatcher as `caller_agent_id`) is now threaded through the `KnowledgeGraph` handle trait and the `Memory` trait into the store, and an absent caller id keeps the historical shared/unscoped (`""`) write; existing rows already written under `""` stay shared/unscoped (they cannot be retroactively attributed to an agent), while new writes are agent-scoped; entity rows remain shared across agents by their composite `(id, peer_id)` key (which does not include `agent_id`), so an entity's `agent_id` is first-writer provenance and the relation carries the load-bearing scoping — the pre-existing `delete_by_agent` / `ON CONFLICT` interaction for a shared entity whose first-writer agent is later deleted is unchanged here and tracked as a follow-up (#6519) (@houko)
- Serialize an explicit `session_id_override` that targets the agent's canonical session on the SAME lock as the no-override persistent path, closing a lost-update race on session history.
  The message-dispatch lock was selected purely on whether an override was supplied — `Some(sid)` → `session_msg_locks[sid]`, `None` → `agent_msg_locks[agent_id]` — so a REST `POST /message` passing `session_id = <canonical entry.session_id>` and a concurrent no-override persistent dispatch to the same agent (plain `send_message` / a workflow step / a trigger fire on an agent with no home channel) took two different mutexes, ran concurrently, and both loaded → appended → blind-`save_session`'d the one canonical session, losing one write.
  An override equal to the canonical session now collapses to the per-agent lock namespace (with matching re-entrancy / held-lock tracking) at both the non-streaming (`send_message_full_inner`) and streaming (`send_message_streaming_with_sender_and_opts`) sites; the raw override still flows to `resolve_dispatch_session_id`, so the resolved session id is unchanged.
  A no-override channel dispatch keeps the per-agent lock (narrow fix — the channel-derived override variant, which the cron-prune path already hand-guards, is unchanged) (#6518) (@houko)
- Stop the streaming LLM paths from silently swallowing a provider error delivered mid-stream over an already-HTTP-200 SSE body, which turned a recoverable overload / rate-limit into a truncated-or-empty "successful" turn with no retry or failover.
  The Anthropic driver now handles an `event: error` frame (e.g. `overloaded_error` under load) by returning the typed `LlmError::Overloaded` / `RateLimited` / `Api` it maps from `error.type` instead of letting the frame fall into the catch-all match arm and ending the stream `Ok` with partial content.
  The OpenAI-compatible driver (which OpenRouter and Groq route through) now surfaces a terminal `data: {"error": …}` frame as an `LlmError::Api` classified via a new `openai_stream_error_code` helper — so a rate-limit still retries the same provider rather than skipping it — but only when the frame carries a real error signal (a non-null `message` / `code` / `type`, a non-empty string, or a bare number/array), so a benign `"error": null` or an all-null error object riding alongside valid `choices` on a normal content chunk no longer aborts the stream and discards that content; the classifier also accepts the numeric HTTP-status `code` OpenRouter sends (`429`) in addition to the symbolic strings, and the surfaced `LlmError::Api.status` is derived from the typed code (429 / 503 / 402 / …) instead of a blanket 502.
  The same non-null check also filters empty strings, not just null, on the nested `error.message` / `error.code` / `error.type` fields — mirroring the existing bare `"error": ""` exclusion — so a provider that pads a normal content chunk with an all-empty-string (rather than all-null) placeholder object no longer has that chunk's content discarded (#6512) (@houko)
- Apply a content-emitted guard at BOTH streaming layers that can re-run a provider, so surfacing a mid-stream error (above) cannot corrupt the caller's output by concatenating a second full response onto content already delivered.
  `FallbackDriver::stream` (the failover layer) now wraps the inner stream in the same content-emitted intercept relay `FallbackChain::stream` already uses and propagates the error instead of failing over once any observable content (text / tool / thinking deltas) has reached the caller — and no longer health-penalizes or exhaustion-marks a provider that served content before erroring, matching `FallbackChain`.
  `stream_with_retry` (the per-provider retry layer in librefang-runtime), which previously re-streamed a full second response onto the same caller channel after a mid-stream `Overloaded` / `RateLimited` / transient error, now tracks whether content has reached the caller and surfaces the error instead of retrying once it has (retry is still applied when the error precedes any content) (#6512) (@houko)
- Classify a `[browser]` config change as restart-required instead of reporting a false hot-reload success: `POST /api/config/reload` queued a `HotAction::ReloadBrowserConfig` whose apply arm only logged "new sessions will use updated config", but the live `BrowserManager` captures `BrowserConfig` by value at boot with no rebuild path, so every new session kept the boot-time `max_sessions` / `cdp_endpoint` / `headless` until a full daemon restart while the reload reported the change applied.
  `build_reload_plan` now marks browser changes `restart_required` (honest reload report), the misleading hot action and its now-dead `ReloadBrowserConfig` variant are removed, and the ops reference table (`docs/operations/config-reload.md`) is updated `H` → `R` (#6516) (@houko)
- Honor `CompletionRequest.response_format` (JSON / JSON-Schema structured output) in the Gemini and Vertex AI drivers, which silently dropped it — the `GenerationConfig` had no `responseMimeType` / `responseSchema` field and neither `build_request` (used by both Vertex call sites) nor Gemini's own inline `complete()` / `stream()` request builders consulted the field, so a caller that set `response_format = Json` / `JsonSchema` and routed to Gemini/Vertex got free-form prose and a downstream JSON parse failure with no signal, while the OpenAI, Anthropic, and Ollama drivers already honored it.
  `GenerationConfig` gains `response_mime_type` + `response_schema`, a `gemini_response_format` mapper sets `responseMimeType = "application/json"` for `Json` / `JsonSchema` and passes the schema through verbatim for `JsonSchema`, and `complete()` / `stream()` now route through `build_request` so all four call sites (2 Gemini inline + 2 Vertex) share one mapping (Vertex inherits the fix).
  Gemini's `responseSchema` is a restricted OpenAPI-subset, so an invalid schema is rejected by the API like any other bad request (#6515) (@houko)
- Stop a superseded/aborted streaming turn and a timed-out trigger fire from permanently leaking the scheduler's per-agent token reservation, which could lock an agent out of its own quota until the hourly window rolled over (a self-inflicted quota DoS).
  `AgentScheduler::check_quota_and_reserve` pre-charges `total_tokens` and returned a bare `u64`, so the pre-charge was rolled back only by an explicit `settle_reservation` / `release_reservation` call; when the owning future was dropped mid-flight — a follow-up message superseding the in-flight streaming turn (`abort()`, #3739), a `stop`/`kill`, or a trigger fire dropped by its `tokio::time::timeout` — neither call ran and the estimate (the agent's `model.max_tokens`) leaked, while the USD sibling (`MeteringReservation`) was already released on drop.
  Reservations now go through a new `AgentScheduler::reserve_tokens` that returns a `#[must_use]` `TokenReservation` RAII guard whose `Drop` releases the pre-charge unless it was explicitly settled/released, bringing the token side to parity with the USD side; a no-quota reservation carries `estimated_tokens == 0` so its drop is a no-op, preserving the zero-reservation contract.
  Only affects agents with a non-zero effective token quota configured (#6513) (@houko)
- Make the CHANGELOG `(@user)` attribution check recognize attribution anywhere in a bullet's block, not only on the `- ` marker line, so it stays compatible with the repo's own one-sentence-per-line prose rule: a long multi-sentence bullet wraps across lines and carries its trailing `(@houko)` on the final continuation line, which `check-changelog-attribution.py` (and the pre-commit hook + `CHANGELOG Attribution` CI gate that call it) previously flagged as missing attribution, so wrapping a bullet to satisfy the prose rule broke the attribution gate. The three scan modes (diff / `--all-unreleased` / `--staged`) now check the whole bullet block (marker line plus indented continuation lines up to the next blank line, bullet, or heading); a no-attribution bullet is still caught (@houko)
- Route the post-approval agent reply through the account-qualified outbound path so multi-account installs deliver to the correct bot/chat: `wake_agent_after_approval` previously looked the channel adapter up by the bare channel key (`channel_adapters.get(channel)`), ignoring `deferred.account_id`, but adapters are registered under both the bare key and the account-qualified key (`"<channel>:<account_id>"`), so a resumed reply for a non-first account was delivered to the wrong account's adapter or missed entirely; it now reuses `ChannelSender::send_channel_message`, which builds the same account-qualified lookup key the canonical outbound path uses (#6492) (@houko)
- Pin `crates/librefang-api/src/login_page.html` to LF in `.gitattributes`: the file is embedded verbatim via `include_str!` and its inline `<script>` is authorised by an exact SHA-256 baked into the CSP (`middleware.rs`), so a Windows checkout with `core.autocrlf=true` rewrote it to CRLF, shifted the computed hash, and failed `dashboard_login_page_script_is_allowed_by_csp_hash` on the Windows shard only — turning `main` red on every merge since #6486 touched the page (#6481) (@houko)
- Honour glob patterns in the per-agent `tool_allowlist` / `tool_blocklist`, matching the Step 1 builtin filter in the same function: Step 4 of `available_tools` used exact string equality, but MCP tool names are runtime-generated and namespaced (`mcp__<server>__<tool>`), so they can never be enumerated as static literals — a non-empty `tool_allowlist` therefore retained only exactly-named native tools and silently dropped every `mcp__*` tool, and a `mcp__*` glob entry itself matched nothing because `*` was treated literally; both retains now use `glob_matches` (already used by the builtin filter above), so `["file_read", "mcp__notion__*"]` works and a plain exact name still matches unchanged via `glob_matches`' `pattern == value` fast path (#6495) (@houko)
- Fix `librefang approvals approve` failing with a 415 that printed a false `✔ approved`, and hide resolved entries in `approvals list` (#6492): the approve request was a bodyless POST, so it carried no `Content-Type` and axum's `Json<ApproveRequestBody>` extractor rejected it with 415 before the handler ran — it now sends an empty JSON object so the header is present. The CLI also gated success only on a `body["error"]` field, so a 415 (whose body deserializes to `{}`) printed success; success is now gated on the real HTTP status via a new `daemon_json_checked` helper, with the failure reason falling back to the status when the body carries no error. Separately, `approvals list` rendered every entry the API returns — which includes recently-resolved (`approved` / `rejected` / `expired`) requests alongside pending ones — with no status column, so a terminal request looked actionable; it now shows a Status column. Server-side resolution and the re-resolve 400 were already correct (the latter via #6441). (@houko)
- Treat `auto_dream` (and other system-internal) fork tool calls as system-internal so RBAC no longer gates them: dream turns run through `run_forked_agent_streaming` with a `None` sender context and no synthetic channel, so once `[[users]]` was configured their `memory_store` / `memory_recall` / `memory_list` calls fell through `resolve_user_tool_decision` to `guest_gate` — whose allowlist lacks the `memory_*` tools — and hit `NeedsApproval` on every dream cycle; `LoopOptions` now carries a `system_call` flag that the fork path sets and the runtime dispatch forwards to `ApprovalGate::resolve_user_tool_decision` (mirroring the existing cron / autonomous channel escape hatch), so internal forks bypass the per-user gate while an unattributed user call with no sender still fails closed through the guest gate (#6463) (@houko)
- Allow the standalone `/dashboard` login page's inline submit handler under the global Content Security Policy: the CSP's `script-src 'self'` (no `unsafe-inline`, no hash) blocked the page's inline script that posts credentials to `/api/auth/dashboard-login`, while the bundled React SPA at `/` uses only external scripts and was unaffected — making the failure look language-dependent; `script-src` now allows only the exact SHA-256 hash of that static script, `unsafe-inline` stays forbidden for scripts, and a new response-level regression test hashes the served script and asserts the CSP source allows it (#6480) (@houko)
- Stop a partial `[channel_overrides]` table from silently gating group messages to mention-only: `dm_policy` / `group_policy` were bare enums that materialized to their `#[default]` (`GroupPolicy::MentionOnly`) whenever the struct was constructed, so writing one unrelated field (e.g. `threading = true` for Slack) flipped an unset group policy from the absent-`None` "process everything" behaviour to "mention-only" and dropped all non-mention group traffic on every channel the agent served — the reporter lost all Matrix inbound this way (#6444 fixed only the Matrix symptom). Both fields are now `Option<_>`, so an unset policy stays `None` (no gating, matching the historical whole-struct-absent path) and is distinct from an explicitly written `Some(MentionOnly)`; the two bridge gating paths (text + media) treat `None` identically to the old no-overrides case, and an unset policy that still carries `group_trigger_patterns` resolves to mention-only so the patterns keep gating. The reply-intent precheck gate on both paths was carried along too: it previously fired only for the literal `Some(GroupPolicy::All)`, so the common unset-policy "process all" config silently lost its precheck filter and replied unconditionally, while the semantically-identical explicit `group_policy = "all"` still got it; both gates now route through a shared `group_reply_precheck_applies` helper that resolves `None` (no trigger patterns) the same as explicit `all`, so the two paths cannot drift again (#6468) (@houko)
- Stop the `Security` CI job from failing red on an audit-infrastructure outage: npm retired the legacy `pnpm audit` endpoints (`/-/npm/v1/security/audits[/quick]`, now HTTP 410 "endpoint is being retired"), and `xtask deps --web` classified every resulting non-zero `pnpm audit` exit across `web` / `dashboard` / `docs` as a dependency issue — so all three tripped and `main` went red on every push that touched Rust or CI, unrelated to the change under test; `run_pnpm_audit` now captures the audit output and, on the retired-endpoint signature, warns and skips (a `Skipped` outcome not counted against the build) while a genuine advisory still fails as before (#6466) (@houko)
- Stop `sandbox_command` from silently dropping secret-shaped names the operator explicitly listed in `exec_policy.allowed_env_vars` (#6439 regression): the credential-word heuristic (`KEY`/`SECRET`/`TOKEN`/`PASSWORD`/…) meant the allowlist could never carry a credential, breaking every credentialed CLI driven through `shell_exec` / `process_start` (e.g. a keyring-unlock password env var); the two passthrough sources are now filtered by trust level — the operator's own allowlist is refused only the daemon's reserved secrets (`LIBREFANG_VAULT_KEY`, provider API keys, via the new `is_reserved_env_var`), while hand-assembled untrusted lists keep the full heuristic at both assembly and spawn time, and refused names are now surfaced in the tool output instead of only a daemon-log WARN (#6458) (@houko)
- Resolve the model context window provider-aware so an OpenRouter agent stops showing the 8192 unknown-model denominator in the chat context bar (and no longer mis-sizes the live `ContextBudget` / compaction math): live catalog entries are keyed `openrouter/{raw_id}` but the dashboard model-picker (`set_agent_model`) persists the manifest model without the prefix, and the kernel's eight `find_model(&manifest.model.model)` lookup sites were provider-blind, so a bare id (`tencent/hy3:free`) missed the prefixed catalog entry and fell back to `UNKNOWN_MODEL_CONTEXT_WINDOW`; a new `ModelCatalog::find_model_for_manifest(provider, model)` re-qualifies the bare id against the `{provider}/` prefix (the issue's suggested `find_model_for_provider` alone would still have missed), and every lookup site — context report, live budget, compaction, tool-support detection, and the #6398 thinking-backfill gate — now routes through it (#6423) (@houko)
- Fix Matrix sidecar inbound being silently dropped under the default `GroupPolicy::MentionOnly`: the adapter hardcoded `is_group=True` for every room and never set `metadata["was_mentioned"]` (the only mention signal the group gate reads), so both 1:1 DMs and explicit @-mentions of the bot were dropped with `OB-06 group_gating_skip` and the bot appeared dead; the adapter now flags a room non-group when it is listed in `m.direct` or its `/sync` summary reports two joined members, and sets `was_mentioned` when the bot's own user id is in the MSC3952 `m.mentions` list, restoring parity with the Discord / Feishu adapters (#6444) (@houko)
- Derive the fallback `agent.toml` path from the agent's UUID instead of the literal `"agent"` when the DB row has no `source_toml_path`: `safe_path_component` strips every non-ASCII character, so an agent whose display name is entirely Cyrillic / CJK / accented-Latin sanitized to the empty string and every such agent collapsed onto the shared path `workspaces/agents/agent/agent.toml` — TOML edits were silently ignored and two such agents overwrote each other; the three fallback sites (boot re-sync, `persist_manifest_to_disk`, `reload_agent_from_disk`) now pass the UUID, matching the spawn-time `resolve_workspace_dir` derivation so distinct agents never collide and auto-discovery finds the real directory (#6442) (@houko)
- Cap the `FallbackChain` non-streaming rate-limit backoff: a provider's `Retry-After` (or the shared rate guard's persisted ~1h RPH lockout) was slept verbatim before failover, stalling the turn for minutes-to-an-hour and defeating the chain's purpose; a backoff over a 10 s cap now fails over to the next provider immediately, matching the streaming path and the sibling `FallbackDriver` (#6446) (@houko)
- Stream attachment-URL fetches with a running size cap instead of buffering the whole body first: `resolve_url_attachments` read `resp.bytes()` and only checked the 20 MB limit afterward, so a server streaming an unbounded body (bounded only by the 30 s timeout) could allocate multiple GB and OOM the daemon; it now pre-rejects on Content-Length and aborts mid-stream once the cap is exceeded, bounding peak memory (#6446) (@houko)
- Mark a workflow run `Failed` when a Conditional / Loop / DAG step references a deleted agent: those paths returned `Err` via a bare `?` without touching run state (only the Sequential arm set `Failed`), so the run hung in `Running` until the next stale-recovery sweep at daemon boot; all agent-resolution failure paths now share a `mark_run_failed` helper, and the DAG concurrent layer skips resolving the agent of a step whose dependency already failed (#6446) (@houko)
- Bound ClawHub/Skillhub skill-zip extraction with the same decompression-bomb guards as the marketplace bundle path (entry count, per-entry uncompressed size, compression ratio, total uncompressed size) via a shared `write_zip_entry_capped` helper: the install path streamed every entry with an unbounded `std::io::copy`, so a malicious skill zip could exhaust disk / inodes (#6446) (@houko)
- Fix the per-IP WebSocket connection tracker's guard-drop TOCTOU: the `Drop` impl decided to `remove` the entry from a stale read, so a concurrent `try_acquire_ws_slot` could re-increment it between the read-guard release and the `remove`, deleting the entry backing a live connection and letting a single IP exceed `max_ws_per_ip`; it now uses `remove_if` with a re-checked counter under the write lock (covers both the agent and terminal WS paths) (#6446) (@houko)
- Compare canonicalized `serde_json::Value`s (not raw JSON strings) in the config-reload `field_changed` diff: `HashMap`-bearing fields (users, channel_role_mapping, broadcast routes, audit retention, per-skill env, …) serialized in per-instance iteration order, so a content-identical reload spuriously reported a change — emitting a needless `ReloadAuth` and a `restart_required` signal on every reload for any multi-entry deployment; `to_value` normalizes map key order so equal content compares equal (#6446) (@houko)
- Evaluate the session-reset policy on persistent channel sessions even when the agent manifest sets `session_mode = "new"`: the reset skip was keyed on the requested mode, but the channel branch resolves to a persistent `for_sender_scope` session regardless of `session_mode`, so such a session was silently excluded from `config.session.reset`; the skip is now keyed on whether the session id was freshly minted this call (a genuinely ephemeral `SessionId::new()`) (#6446) (@houko)
- Record cached prompt tokens in the ChatGPT / Codex Responses-API driver: the streaming usage parse read only `input_tokens` / `output_tokens` and left `cache_read_input_tokens` at zero, so the metering layer billed the cached prefix at the full input rate (the 10% cache-read discount was lost); it now reads `input_tokens_details.cached_tokens`, mirroring the chat-completions driver (#6446) (@houko)
- Surface a mid-stream error on the OpenAI-compatible streaming endpoint as an in-band error frame instead of a truncated `finish_reason:"stop"` + `[DONE]`: the forwarder discarded the agent-loop `JoinHandle` (the only carrier of a mid-stream failure) and always emitted a clean finish, so clients received partial output framed as a successful completion; also keep the streamed `tool_call` index monotonic across agent-loop iterations and thread image / conversation-history input through to vision models (a streaming request carrying an image now returns an explicit 400 rather than silently dropping it) (#6441) (@houko)
- Key the channel message debouncer and typing-flush buckets on the per-user sender id rather than the group chat id, so within the debounce window one group member's message (and any injected instructions) is no longer coalesced into another member's identity, RBAC resolution, and billing (#6441) (@houko)
- Write memories through to an attached HTTP vector backend on both insert and re-embed so embedding recall is not silently empty against it, and send the OAuth bearer token / configured auth headers to SSE-transport MCP servers so OAuth for the SSE transport is no longer inert (#6441) (@houko)
- Tolerate `redacted_thinking` / server-tool content blocks in the Anthropic non-streaming `complete()` path (the streaming path already did), clear the stale past `expires_at` on an MCP token refresh that omits `expires_in` (which otherwise forced a network refresh every request and burned rotating refresh tokens), and re-rank SQLite semantic recall over a similarity-neutral candidate window so relevant older memories are not dropped (#6441) (@houko)
- Make `GoalRunner::start` atomic so concurrent starts for the same goal cannot leave a second orphaned, unstoppable loop, and make the cron `SummarizeTrim` generation-CAS exclude the concurrent message send so an appended cron turn is not silently lost (#6441) (@houko)
- Map `approve_request`'s typed kernel errors to the correct HTTP status — 404 for a missing / expired id, 400 for a missing second factor or an already-resolved request, and 500 only for a genuine internal failure — instead of collapsing every resolve failure to 400 (#6441) (@houko)
- Gate the `cli_pypi` release job to stable and LTS tags only, in both `release.yml` (the tag-push pipeline) and `release-cli.yml` (the manual `workflow_dispatch` CLI re-run path): every beta/rc pre-release tag was also uploading ~250 MB of per-platform CLI binary wheels to the `librefang` PyPI project, and 46 accumulated pre-releases consumed 10.35 GB of its 10 GB total-size quota, so the `v2026.7.10` stable publish started failing with `400 Project size too large`; pre-release CLI binaries remain attached to the GitHub Release, only the PyPI upload is now skipped for them (#6433) (@houko)
- Stop the desktop release job's asset cleanup from deleting the CLI binary tarballs: its "Delete existing assets for this target" step matched `*<rust_target>*`, the same target triple the `cli_*` jobs embed in `librefang-<target>.tar.gz` (+ `.sha256`), so a desktop job re-running after its CLI counterpart had already uploaded — exactly what the macOS desktop jobs do when they re-run for notarization — deleted the freshly-uploaded `librefang-{x86_64,aarch64}-apple-darwin.tar.gz` as collateral, which then made `cli_pypi` fail with a 404 when it downloaded them to build the wheels; the cleanup now skips CLI-owned `librefang-*` / `SHA256SUMS*` assets (desktop bundles use Tauri's own `LibreFang_<ver>_<x64|aarch64|…>` naming, which never contains the triple, so the guard costs the desktop nothing) (#6433) (@houko)
- Clear 13 more Rust 1.97 stable clippy `useless_borrows_in_formatting` errors (`redundant reference in format! argument`) across the CLI TUI screens (`comms.rs`, `hands.rs`, `settings.rs`, `skills.rs`, `templates.rs`): stable's tightened lint turned the `Quality / Build + clippy` gate red on every PR regardless of which one triggered the `main` run, the same lint class #6421 cleared elsewhere in the workspace; `format!` captures its arguments by reference either way, so dropping the borrow is a pure lint fix with no behavior change, and the genuine `widgets::truncate(&x, n)` function-argument borrows are untouched (#6427) (@houko)
- Stop the channel media gate from spending a billed reply-precheck LLM call on captionless media in a `group_policy=all` group: #6141 ran `classify_reply_intent` on an empty string (`extracted_user_text(...).unwrap_or_default()`) for a caption-less image / voice / video, so the precheck now runs only when there is text to classify (captioned media keeps parity with the text path) and captionless media proceeds without the call, which also reconciles #6141's PR notes that claimed the precheck was not replicated onto the media path when the merged code did replicate it (#6426) (@houko)
- Seed test-booted kernels from a pinned in-repo registry snapshot (`crates/librefang-runtime/tests/fixtures/registry/`, librefang-registry@89d0e4c8) instead of the network: #6410's `LIBREFANG_REGISTRY_OFFLINE=1` export made `sync_registry` a no-op on fresh test homes, turning main red with ~111 registry-content unit tests (model catalog, hand routing, hands registry, MCP catalog/installer, hand activation, metering) plus 3 `librefang-api` integration tests; a new `registry_sync::seed_registry_fixture_for_tests` copies the fixture into the test home and fans it out through the real sync path, `resolve_home_dir_for_tests` and the 27 API-test harnesses use it, `MockKernelBuilder` gains an opt-in `.with_registry_fixture()`, and the env-var test locks are promoted to a crate-wide `test_env::ENV_LOCK` so `tts::test_synthesize_no_provider` no longer races `model_catalog`'s `GOOGLE_API_KEY` setter under threaded `cargo test` (#6421) (@houko)
- Clear the Rust 1.97.0 stable clippy errors (`question_mark`, `useless_borrows_in_formatting`, `for_kv_map`) that turned `cargo clippy -- -D warnings` into a hard failure on the Quality lane of every PR touching a Rust crate — `main` itself stayed green only because its recent commits were docs-only, so CI's per-crate change-detection skipped the Rust lane and never exercised the lint — across `librefang-skills` (`config_injection.rs`), `librefang-runtime` (`a2a.rs`), `librefang-kernel` (`tools_and_skills.rs`), and four `librefang-api` sites (`channel_bridge.rs`, `routes/agents/sessions.rs`, `routes/workflows/workflow.rs`, `tests/totp_flow_test.rs`); folded into this PR because its own Quality lane cannot go green until the lints are fixed, and the registry fixture this PR adds is what keeps the test lanes green alongside the clippy fix (#6421) (@houko)
- Uppercase the character after each separator when deriving the version-pinned Homebrew tap formula's class name, matching Homebrew's `Formulary.class_s`, so a pre-release tag yields `class LibrefangAT2026710Beta1` — the exact casing the `golden_gate` audit requires; #6416's `tr -cd '[:alnum:]'` removed the hyphen but left the channel word lowercase (`...beta1`), which `golden_gate` still rejects, so the next `-beta` / `-rc` release would have generated a versioned formula that fails `brew readall` / install (verified by running the real generation step for a stable and a beta tag → `brew readall` reports zero errors) (#6418) (@houko)
- Correlate dashboard chat turns with the WebSocket terminal frames that own them: the client now sends a `message_id` on every chat message and the daemon echoes it on `response` / `silent_complete` / `error`, so a previous turn's late terminal frame — delayed past the next user send by post-turn proactive-memory extraction — lands on its own bubble instead of overwriting the newer message's bubble and dropping the newer answer (#6419) (@houko)
- Generate valid Ruby class names for the version-pinned Homebrew tap formulae (`librefang@<ver>`): the release pipeline stripped only `.` from the version (`tr -d '.'`), leaving the `-` in pre-release versions so `librefang@2026.6.29-beta.14` produced `class LibrefangAT2026629-beta14 < Formula` — a syntax error that made every beta/rc pin fail `brew readall` / install; it now derives the class name with Homebrew's own `Formulary.class_s` rules (`LibrefangAT2026629Beta14`), and the 40 already-published broken pins are corrected in `librefang/homebrew-tap` (#6416) (@houko)
- Emit a single `conflicts_with` cask stanza in the Homebrew tap cask generator (`release.yml` and `release-desktop.yml`): casks allow only one `conflicts_with`, but the generator wrote one stanza per other channel, so `librefang-beta` / `librefang-rc` casks were rejected as invalid; the other channels are now collected into a single array literal (`conflicts_with cask: ["librefang", "librefang-rc"]`), with the two already-published casks fixed in `librefang/homebrew-tap` (#6416) (@houko)
- Export `LIBREFANG_REGISTRY_OFFLINE=1` in the four CI test lanes (unit, Ubuntu shards, Windows, macOS) so test-booted kernels stop fetching the content registry: each fresh temp home triggered a real git clone per boot, and whether the fetch succeeded changed test outcomes — a registry-pre-installed `assistant` `agent.toml` made restore treat the disk template as authoritative and clobber explicit DB model config, deterministically failing PR #6384's restart test in CI while passing locally under `just test` (#6410) (@houko)
- Request extended thinking through the OpenAI-compatible driver: a configured `CompletionRequest.thinking` budget now emits the OpenAI-style `reasoning_effort` parameter (`low` ≤ 4096 tokens, `medium` ≤ 16384, `high` above; budgets under 1024 stay off, matching the anthropic driver's gate), closing the asymmetry where the stream path parsed `reasoning_content` deltas but the request side never asked for them — emitted only under the default echo policy (Kimi's `EmptyString` disable and the DeepSeek `Strip` / `Echo` families are excluded), an explicit `extra_body.reasoning_effort` override takes precedence, and the global `[thinking]` backfill now skips models the catalog marks `supports_thinking = false` so a mixed fleet does not start sending the parameter to known non-reasoning models (#6407) (@houko)
- Correct the channel docs' claim that all five sidecars read `*_DM_POLICY` / `*_GROUP_POLICY` env vars: only the WhatsApp sidecar reads `WHATSAPP_DM_POLICY` (Cloud API webhook mode only; `WHATSAPP_GROUP_POLICY` is read but inert), the Telegram / Discord / Slack / Teams sidecars read neither, and the working migration target is `agent.toml [channel_overrides]` `dm_policy` / `group_policy` enforced by the channel bridge (with `allowed_only` user gating backed by `[[users]]` channel bindings); also replace invalid `dm_policy = "always"` / `group_policy = "trigger_only"` example values that fail manifest deserialization (#6405) (@houko)
- Exempt fresh `read_artifact` results from every re-spill gate — the post-tool chokepoint, the per-result tool budget (Layer 2), and, except as a last resort, the per-turn aggregate budget (Layer 3) — so paging in a spilled tool result larger than `spill_threshold_bytes` returns real bytes (still subject to the pre-existing ~10 KB sanitize truncation cap) instead of minting a fresh artifact stub that again points at `read_artifact`; previously any read in the 16–64 KiB range could never make progress, and a `spill_threshold_bytes` below the sanitize cap recreated the loop at Layer 2 (#6406) (@houko)
- Add a `LIBREFANG_REGISTRY_OFFLINE` switch that skips the registry network refresh (git clone / tarball fallback) while keeping local pre-install copies, and export it from `just test`: every test-booted kernel previously attempted a real registry fetch from its fresh temp home, and the resulting git fork storm exhausted pid limits in constrained containers (`Resource temporarily unavailable` failures in `just test`) (#6408) (@houko)
- Clear the `cargo-deny` / security-audit advisory RUSTSEC-2026-0204 failing CI on every branch by bumping the transitive `crossbeam-epoch` to 0.9.20 (invalid pointer dereference in the `fmt::Pointer` / `fmt::Display` impls for null `Atomic` / `Shared` pointers; pulled in via `rayon-core` → `crossbeam-deque`, lockfile-only change) (#6400) (@houko)
- Prefer OpenRouter's live `/models` catalog over the checked-in fallback snapshot when selecting or validating a model, refresh stale data when the user opens the model picker, narrow default model synchronization to target only the delisted model during intra-provider free-model migrations, align catalog freshness checks across route handlers, make restart tests hermetic, and treat missing pricing as unknown instead of free (#6384) (@pavver)
- Preserve `provider = "default"` and `model = "default"` as an explicit agent setting that follows future global model changes, expose a dashboard action to restore that mode, and treat every concrete agent model as an explicit pin (#6384) (@pavver)
- Surface the model each CLI passthrough provider (`codex-cli`, `claude-code`, `gemini-cli`, `qwen-code`) is configured to run, read live from the tool's own config, so a custom model — DeepSeek via `~/.codex/config.toml`, a Kimi/Moonshot id via Claude Code's `ANTHROPIC_MODEL` / `~/.claude/settings.json`, a Gemini preview via `GEMINI_MODEL` / `~/.gemini/settings.json`, or an OpenAI-compatible id via `~/.qwen/settings.json` — is recognised on the Providers page and in the agent model picker instead of only the catalog's default models (#6365) (@houko)
- Stop CLI providers (`codex-cli`, `gemini-cli`, `claude-code`, `qwen-code`) from forcing a placeholder `--model <provider-id>` onto their CLI for a bare provider id, so each CLI defers to its own configured default model (#6365) (@houko)
- Clear `cargo-deny` advisory failures on `main` by bumping `anyhow` to 1.0.103 (RUSTSEC-2026-0190) and ignoring the unmaintained `ttf-parser` advisory (RUSTSEC-2026-0192 — transitive via `pdf-extract` → `lopdf`, no safe upgrade available) (#6366) (@houko)
- Clear the `quick-xml` advisories RUSTSEC-2026-0194 / RUSTSEC-2026-0195 by bumping `plist` to 1.10.0 (pulls the patched `quick-xml` 0.41.0) and `tauri-winrt-notification` to 0.7.3 (drops its `quick-xml` dependency), removing both vulnerable versions from the lockfile (#6387) (@houko)
- Raise the Nix Build job timeout to 120 minutes so the now-routine cold builds — the Rust CI lanes churn the repo's 10 GB Actions cache quota daily, evicting the `/nix/store` cache between runs — complete instead of being cancelled at 60 minutes, unbreaking the workflow that had been red on `main` since June 11 (#6389) (@houko)
- Route every dashboard copy button through the clipboard helper, so copying works on a daemon reached over plain HTTP at a LAN or VPN address.
  `navigator.clipboard` is only defined in a secure context, so on `http://<lan-ip>:4545` the property is `undefined` and a bare `navigator.clipboard.writeText(...)` throws before the promise exists — the button produced no clipboard content, no error, and no visual feedback.
  `lib/clipboard.ts` has fixed this since it was written, falling back to `document.execCommand('copy')` through a detached textarea, but only the chat page imported it; the config page, audit detail, agents page, users page, skill install command, and TOML viewer had all reintroduced the raw API.
  The helper reports failure by resolving to `false` rather than throwing, so each call site now branches on the result instead of on a `catch` that could never fire — the users page in particular gated the close button of a one-time-visible rotated API key on a copy it never verified, and two call sites showed a "Copied" toast unconditionally.
  An eslint `no-restricted-properties` rule now rejects `navigator.clipboard` everywhere in the dashboard except the helper itself, because the helper's own comment shows this was already fixed once and regressed (#6668) (@houko)
- Expose `peer_id`, `session_mode`, and `delivery` on `/api/schedules`, which `/api/cron/jobs` already reported for the identical job.
  The two routes are deliberate alternate views over one `CronJob` store — the cron view serializes the struct whole, the schedules view renders a flattened presentation — and the flattened one had fallen behind on all three fields that decide how a fire behaves: which peer's memory it resolves against, whether every fire shares one session or gets an isolated one, and where its output goes.
  The reporter could confirm only `peer_id` because `CronJob::session_mode` carries `skip_serializing_if = "Option::is_none"`, so an unset value is absent from the cron view rather than null and looked the same as a field that does not exist.
  The schedules view therefore emits both as explicit nulls when unset, matching how it already renders `tz`, `last_run`, and `next_run`: a read surface with a stable key set lets a client tell "not configured" apart from "the server is too old to report it".
  `POST /api/schedules` now sets all three as well, where it previously hardcoded `peer_id` to null, forced `delivery` to the fire-and-forget variant, and parsed `session_mode` through an `.ok()` that turned a misspelling into "use the agent's default" behind a `201`; a malformed value on any of them is now a 400 that names the field.
  `PUT /api/schedules/{id}` patches `delivery`, which the kernel has always supported and this route simply never forwarded.
  It cannot patch `peer_id` or `session_mode`, because `CronScheduler::update_job` has no branch for either and its omitted-or-null-means-untouched convention cannot express clearing an optional field, so a request that tries to change one is refused with a 400 explaining that a recreate is needed rather than answered with a 200 for a patch that never applied — echoing the stored value back stays a no-op so the natural read-modify-write round trip still works (#6668) (@houko)
- Report `cost_usd`, `total_tokens`, `duration_ms`, and the derived `label` on `GET /api/sessions/{id}`, which the session list has always computed and the detail endpoint did not.
  The list derives cost and tokens from a `usage_events` join, the duration from the first-to-last stamped message, and a label snippet from the first user message when the column is empty, while the detail handler hand-built a fixed object carrying none of that — so the same session answered differently depending on which route you asked, and an unnamed session that reads as "hello…" in the list read as `null` in the detail.
  The same root cause as #6596, so both views now share one helper per value rather than a second copy of the derivation, since a copy is what let them diverge in the first place.
  The shapes match the list exactly: cost and tokens are numeric zeros for an unmetered session rather than null, the duration is null below two stamped messages, and an explicit label still wins over the snippet.
  A failed usage aggregate now returns 500 rather than a zero, matching the session load beside it — reporting no spend for a session that spent money is the same silent-wrong-value failure this issue is about (#6668) (@houko)
- Render `approval.trusted_senders` as a read-only card on the Approvals page, so the approval-bypass roster is auditable from the dashboard and not just from the API.
  A sender on that list skips the approval prompt for every tool the risk classifier does not rank high, and it reached no operator-facing surface at all — #6637 exposed it on `GET /api/config` along with the rest of the non-writable `ApprovalPolicy` fields, but nothing rendered it, so auditing who holds the waiver still meant shell access to read `config.toml`.
  It stays out of the config write allowlist for the reason it needed exposing in the first place: adding yourself to an approval-bypass list over HTTP is precisely the escalation the approval gate exists to prevent, so holding an API key must not be enough to do it.
  The card sits above the pending queue because it explains the requests that never arrive, and an empty list is presented as the reassuring state it is — every sender goes through the gate (#6668) (@houko)
- Reject unknown keys inside `[mcp_servers.transport]` instead of silently discarding them, closing a gap between an MCP server entry and the transport table nested one level inside it.
  `McpServerConfigEntry` has carried `deny_unknown_fields` since #5130, because the `detect_unknown_nested_fields` walker bails on array-of-table paths and serde is the only layer that can see a typo in a `[[mcp_servers]]` element at all — but `McpTransportEntry` and the two structs under its `http_compat` variant did not, so the guard stopped exactly one level above where operators hand-write the most config.
  A reporter's `[mcp_servers.transport.env]` table was dropped whole at load: `env` is real, but it belongs to the parent entry as a `Vec<String>` of variable *names*, not to the transport as a key/value table, and nothing said so.
  The subprocess ran with neither variable set and `GET /api/mcp/servers/{name}` reported `"env": []`, while the server's own script fell back to hardcoded defaults that happened to match — so the misconfiguration was invisible until a credential rotation, at which point the rotated secret would have been written to an inert table and the stale default kept working.
  `HttpCompatHeaderConfig` and `HttpCompatToolConfig` had the same gap and are guarded too; both sit under `[[mcp_servers.transport.headers]]` / `[[mcp_servers.transport.tools]]`, arrays of tables nested inside an array of tables, and every field on them except `name` and `path` has a `serde(default)`, so a misspelled `responce_mode` left the tool wired to the default JSON response mode rather than the operator's choice.
  On an internally-tagged enum serde applies the attribute per variant against the buffered content, which is not obvious from the attribute alone and is not true of adjacently- or untagged-tagged containers, so the behaviour is pinned by tests over the reporter's exact TOML rather than assumed from the fact that it compiles.
  Read this before upgrading, because the failure is loud and total rather than local: a stray key under `[mcp_servers.transport]` now makes `librefang start` **exit non-zero without starting the daemon** — the deserialize error propagates out of `load_config`, and `cmd_start` short-circuits on it precisely so the diagnostic naming the offending key reaches stderr instead of being swallowed by a tolerant default.
  It is not "that one server is skipped"; nothing boots until the key is removed.
  Because the rejection happens inside serde it applies whether or not `strict_config` is set, the same way the parent entry has already behaved for a misspelled scalar such as `timout_secs`.
  The hard stop is boot-only: `POST /api/config/reload` maps the same error to a `400` and leaves the running config in place, so a live daemon surfaces the typo without dropping its MCP connections — worth knowing if you edit `config.toml` on a running host, since reload tells you about the mistake at no cost while a restart on the same file will not come back up.
  Two read-side keys the API synthesised into the `transport` object, a derived `source` discriminator on each `http_compat` header and a `tools_count` duplicating the length of the array beside it, are gone: neither is a field of the guarded types, so with the guard in place they turned any `GET` → `PUT` of an `http_compat` server into a `400`, and serde short-circuits at the first unknown key so both had to go.
  The same read route also omitted `input_schema`, which has a `serde(default)`, so that round trip had been quietly overwriting a hand-authored JSON Schema with `{"type":"object"}` — it is now emitted, and a static `http_compat` header `value`, which stays redacted because it is a credential, is merged back from the stored entry on write exactly as an inline `env` value already was.
  Writing an `http_compat` server through the default `config.toml`-backed store also corrupted its headers, which the tests for the round trip above found: that path goes `serde_json` → `json_to_toml_value` → TOML, TOML has no null, and the converter maps an absent `Option` to an *empty string* rather than dropping the key, so an env-sourced header came back from disk as `value = ""` instead of unset.
  The runtime checks `value` before `value_env`, so it then sent an empty header and never resolved the variable — a silent credential failure with nothing logged.
  `value` and `value_env` now carry `skip_serializing_if`, the same fix already documented as load-bearing on the entry's own `template_id` and `oauth` fields for the identical reason.
  A header that carries *both* a static `value` and a `value_env` lost the static one on every read-modify-write, which the merge above initially reproduced rather than fixed: the read route redacts `value` and emits `value_env`, and the merge treated that returned `value_env` as "nothing to restore here".
  Since the runtime resolves `value` first, the header silently stopped sending the operator's static credential and started resolving the variable instead — a `200` with nothing logged and a different request on the wire.
  The merge now keys the decision on `value` alone, so the presence of a `value_env` beside it no longer suppresses the restore (#6666) (@houko)
- Request `/json/new` with `PUT` when discovering a target over HTTP, so `cdp_endpoint` works against Chrome 111 and newer.
  Chrome moved the endpoint to `PUT` as CSRF hardening; librefang still sent `GET` and got back a `405` page whose HTML then failed to parse, so the operator saw `Invalid JSON from /json/new: expected value at line 1 column 1` — a parser error that says nothing about the verb that caused it.
  The request now tries `PUT` first and falls back to `GET` on `405`, which keeps older builds and proxies that only route `GET` working, and the response status is checked before the body is parsed so a non-2xx reply is reported as itself rather than as malformed JSON.
  The two verb-negotiation tests pin the call counts rather than only the returned target, since a GET-first implementation reaches the same result and would otherwise pass both (#6619) (@nevgenov)
- Substitute `MAX_CONTENT_CHARS` into the page-extraction script instead of leaving the cap hard-coded in the JavaScript, so the constant that documents the limit is the one that enforces it.
  `MAX_CONTENT_CHARS` was declared, marked `#[allow(dead_code)]`, and read by nothing; the script truncated at a literal `50000` written twice inside the JS string.
  Editing the constant to change how much page text reaches the model therefore did nothing at all, and the `#[allow]` suppressed the one warning that would have said so.
  The script is now a template with a `__MAX_CONTENT_CHARS__` placeholder substituted once through a `LazyLock`, and a test asserts both that the placeholder is gone from the built script and that it is still present in the template — the second half is what keeps the two from silently drifting apart again (#6623) (@nevgenov)
- Carry the curated `## [Unreleased]` section into the dated release section, which nothing had ever read.
  `cargo xtask changelog` built the whole `## [VERSION]` section from PR metadata — `git log` for the numbers, `gh pr view` for the titles — and inserted it *below* `## [Unreleased]` without touching it, and the `awk` extractors in `release.yml` and `release-notify.yml` slice only that dated section for the release notes, the announcement article, and the social post.
  So the section was write-only: 160 hand-written bullets, the part that explains why a change was made rather than restating its title, and the part a pre-commit hook and two CI jobs enforce `(@user)` attribution on, reached nothing at all.
  The release cut now lifts that body out — subsections and their order verbatim, because a human chose them — composes the dated section as stats, breaking changes, highlights, the curated prose, then the generated entries, and leaves the `## [Unreleased]` heading behind and empty, since in-flight PRs append under it and the `changelog.d/` fold errors outright without it.
  Generated entries fill only the gaps: a PR whose number appears in the trailing `(#N)` group of a curated bullet gets no title line, so every PR is described exactly once.
  That group is read from the bullet's last non-empty line only, which is what keeps a mid-bullet cross-reference (`the latter via #6441`) from being mistaken for the PR the bullet documents and suppressing a real entry, and it accepts the `(#6594, #6595)` form one bullet already uses for two PRs.
  A bullet carrying no reference at all cannot be matched to anything, so that PR keeps its generated line and the bullet is named in a warning — a duplicated entry is cosmetic, a silently dropped PR is not.
  The fallback is per bullet, not global: one unreferenced bullet out of 160 leaves the other 159 suppressing their own entries, where discarding the whole set would have turned three such bullets into no deduplication at all.
  Draining is destructive, and a lossy CHANGELOG surfaces only after the tag exists, so the composed section is checked against the section as it stood on disk before anything is written and a missing bullet aborts the release naming it.
  The check compares whole bullets rather than `- ` marker lines, because the sentence-per-line prose rule makes multi-line bullets the norm — 67 of the 160 — and a marker-line comparison would call a bullet preserved while every sentence after its first had been dropped.
  It parses the section to the next *dated* heading while the drain stops at the next `## [` of any kind, so the two disagree exactly where a drain would truncate — on a bullet continuation line that starts in column 0 with a release heading — rather than restating what the drain happened to take.
  A second release run for one version aborts as well, and needs its own check: regenerating an existing `## [VERSION]` section rebuilds it from PR titles, and by then the first run has already emptied `## [Unreleased]`, so the prose is in no section of the file and the primary guard is comparing against nothing.
  It fires on attribution found anywhere in the bullet, not just the marker line, and names the two ways out — move the prose back under `## [Unreleased]`, or delete the dated section and cut it again.
  Nothing prunes `## [Unreleased]`, so the cut now also reports how many curated bullets reference a PR in the release's own commit range and how many reference only older ones, and warns on the latter: the section accumulates across releases, and carrying an already-shipped bullet into a dated section announces it as new alongside a `Full diff` link that contradicts it (#6628) (@houko)
- Stop the daemon from aborting on macOS when a child process exits while the WASM sandbox is live.
  Wasmtime defaults to Mach exception ports on macOS, which parks a handler thread in a blocking `mach_msg` receive flagged `MACH_RCV_INTERRUPT`; the kernel then returns `MACH_RCV_INTERRUPTED` instead of restarting the call when a signal lands on that thread, and wasmtime treats anything but "port closed" as fatal — it prints one `mach_msg failed with ...` line and calls `abort()`.
  The daemon delivers such signals constantly (SIGCHLD from MCP stdio servers, sidecar channels, exec-tool subprocesses, and provider CLI probes), and a process-directed signal can be routed to any thread, so the whole process could die with no panic, no unwind, and no log beyond that line.
  The sandbox now selects POSIX signal handling, which needs no thread parked in an interruptible syscall and is inherited across `fork()`; Mach ports only buy coexistence with Mach-port-based crash reporters, and nothing in the dependency tree uses one.
  It surfaced as a hard SIGABRT partway through `cargo test -p librefang-kernel --lib`, and CI could not have caught it because nextest runs each test in its own process — a `cargo test` guard step on the macOS lane now covers that gap (#6635) (#6638) (@houko)
- Keep an edit to a registry-shipped hand by writing it to a new operator override directory, `<home>/hands/`, instead of into the registry checkout that the next sync erases.
  `update_manifest_persisted` wrote the edit back to `registry/hands/<id>/HAND.toml` because that was the only way to win the load-order race, but that path is inside a git checkout the registry sync fast-forwards with `git reset --hard origin/main` — so the supported way to customise a built-in hand erased itself on the next daemon start, which is what the #6636 reporter hit when even marking the file read-only did not help.
  The override directory sits outside both `registry/` (upstream's copy, and only upstream's) and `workspaces/` (which already means "installed here" rather than "customised here"), and nothing writes into it but an explicit edit, so an override exists only if someone made one.
  `scan_hands_dir` reads it first and the kernel router follows the same precedence, because routing that resolved against upstream's definition while the registry served the edited one would route a renamed hand by rules nobody could see in the UI.
  An id now counts as claimed only once a manifest has actually been read, so a half-written override directory shadows nothing — before, it would have dropped the registry's hand from the scan entirely rather than overriding it.
  Minting an override seeds the shadowed copy's `SKILL.md` and `SKILL-{role}.md` alongside the manifest, since the override replaces the whole directory the scan reads and a manifest-only override would silently strip the skill content that becomes the agents' system prompts; a skill file the operator has since edited is never overwritten.
  Uninstall reaches an override-only hand as well, which is otherwise un-uninstallable once upstream drops an id someone had customised — the hand reports as built-in while the override resurrects it on every reload — and it still refuses, without touching the override, while a registry copy exists (#6669) (@houko)
- Report every `[approval]` field on `GET /api/config`, not the seven of fourteen the response builder happened to enumerate.
  The approval section declares no explicit field list, so the dashboard renders a control for whatever the derived schema says `ApprovalPolicy` has, and the missing seven rendered blank and read back as their JSON zero value.
  `cache_approvals_per_session` is the one an operator noticed: it defaults to `true`, so the box showed unchecked even against a `config.toml` that said `true`, leaving no way to confirm whether per-session approval caching was on.
  `trusted_senders`, `channel_rules`, `timeout_fallback`, `routing`, `totp_tools`, and `audit_retention_days` had the same gap.
  All seven stay non-writable — approval policy is deliberately not adjustable over HTTP, so an Owner-role caller with a leaked API key cannot relax it — which is exactly why the existing `writable ⊆ readable` guard was blind to them, and the new check uses the serialized struct as its oracle so a field added later fails a test instead of quietly joining the gap (#6637) (@houko)
- Make a hand's `[[settings]]` values reach the agent that is already running, not just the one that boots next.
  Settings only reach an LLM as the rendered `## User Configuration` tail on each role's system prompt, and that tail is materialized when the hand is activated.
  Saving from the dashboard wrote the instance config and persisted `hand_state.json` but never touched the live agents, so the change took effect on the next daemon restart — boot replays every persisted hand through the activation path, which does re-render — while the running agent kept answering from the HAND.toml defaults with no error to explain the discrepancy.
  Reported against the Trading Hand, where the prompt branches on `trading_mode` and `approval_mode`: selecting Live Trading and disabling approval silently got neither.
  The save path now persists the config and rewrites each role's prompt under the same per-instance lock the runtime-override path uses, rolling the config back if either step fails so the persisted file and the live registry cannot disagree.
  Re-rendering a live prompt needed its own helper: the three rendered tails are appended in a fixed order and each one strips from its own marker, so re-applying them in sequence to a prompt that already carries all three drops the reference and team blocks on the way past.
  The env-var passthrough allowlist is re-resolved on the same path, since changing a select changes which `provider_env` the subprocess sandbox should admit, and it now narrows as well as widens.
  Two `librefang hand` subcommands that were broken in a way that made the CLI no help in diagnosing any of this are fixed too: `hand set` posted a wrapped `{"config": …}` body at a handler expecting a flat map and never touched the named setting, and `hand settings` read a response field that does not exist and reported "no configurable settings" for every hand.
  A stored value that is not a JSON string — `false` for a toggle, `100` for a numeric field, both valid from any API client — also reverted the setting to its schema default, and scalars are now coerced (#6637) (@houko)
- Stop an explicitly configured EveryAPI gateway from being repointed at whatever account the EveryAPI CLI happens to be logged into — including mid-way through the dashboard's own "Connect EveryAPI gateway" flow, which registers the provider entry before storing its relay key.
  Credential provenance was inferred from `is_custom` and `auth_status`, neither of which carries that meaning — the catalog loader leaves `is_custom` false for every provider when the registry cache is unreadable, and an explicit configuration whose key env var is simply unset looks exactly like an unresolvable CLI login — so a provider file installed before its key was set had its `base_url` rewritten from the CLI account until the next daemon restart.
  Provenance is now recorded explicitly and cleared by every explicit source, so an entry with no reachable credential stays inert instead of falling through to the CLI credential process (#6647) (@houko)
- Show the official EveryAPI square logo in the dashboard sidebar instead of a plain "E" letter placeholder.
  The asset is served from the dashboard's public path and a partner logo asset contract test pins the URL EveryAPI links resolve against (#6648) (@houko)
- Stop reporting an untrackable `agent_send` as a delegation, which handed the model the answer in a field named `task_id` and told it to wait for a callback that would never fire.
  `send_to_agent_async_tracked` has two legitimate outcomes and they meant opposite things: a task id on the tracked path, the callee's whole response body when no parseable caller session made tracking possible.
  Both arrived through one undiscriminated `Result<String, _>`, so the tool that consumes it labelled the response body `task_id`, set `status: "delegated"`, and instructed the model to end its turn and wait — for a reply it was already holding, and which no registered task would ever deliver again.
  The blocking fallback itself is correct and stays; with nowhere to route a completion event, an inline answer beats an orphaned delegation.
  It now returns an `AsyncSendOutcome` the caller must branch on, so a tracked delegation still renders its task id and an inline one returns the reply exactly as the blocking path does.
  The path is reached in production by the MCP HTTP bridge and the REST tool bridge, both of which pass no session by construction, and its log moves from `debug!` to `warn!` with the caller fields attached — an operator whose agents lose async delegation should not have to raise the log level to find out (#6662) (@houko)
- Call `set_self_handle` in the CLI's in-process ACP and MCP backends, which boot a kernel of their own and previously aborted the process the moment they needed a kernel handle.
  `LibreFangKernel::boot` returns a bare kernel and leaves the `self_handle` slot empty — filling it is the caller's job, discharged by seven other production sites but not by these two.
  ACP failed at startup: `KernelAdapter::new` reads `kernel_handle()` as its first action, and that accessor is an `.expect`, so `librefang acp` aborted before serving a request.
  MCP failed later and less visibly, on the first `librefang_agent_*` tool call, because `send_message` resolves the same handle to plumb kernel tools into the agent turn.
  A comment on `KernelAdapter::new` asserting that `boot` wires the handle up before returning is corrected — that claim is what made both omissions look deliberate, and `boot.rs` contains no such call.
  Three tests in `crates/librefang-kernel/tests/self_handle_bootstrap_test.rs` pin the kernel-side contract from both directions, including the idempotence that makes a defensive call safe for any future surface that boots its own kernel, and a source-scanning guard in `librefang-cli` asserts the call sites themselves so deleting either line fails rather than silently restoring the abort.
  Found while triaging the unrelated concern reported in #6651, which does not reproduce on `main` (#6686) (@houko)
- Salt the delegation dedupe hash with the caller and conversation, so two sessions delegating the same message to the same agent no longer share one async task and one reply.
  `register_async_task` dedupes delegations on `(target agent, prompt_hash)` and deliberately ignores the caller — a #5033 decision its own docstring records, along with the instruction that callers needing per-session isolation must salt `prompt_hash` themselves.
  The kernel's only production caller did not: it hashed the message text alone, making the hash a pure function of the prompt.
  Two independent agents asking the same agent the same question therefore collided on one registry entry, the second received the first's handle, and the completion event was delivered only to the first caller's session — while the second, already told "delegation started asynchronously; do not wait", waited for a reply that would never arrive.
  The hash now covers the caller agent, the caller session and the conversation key as well as the message, which is a superset of the registry's `(agent, session)` delivery key plus the field that selects a different callee session.
  The fix is caller-side by design: the kind-only match key is the documented contract and stays, pinned by `register_dedupe_is_cross_session_for_delegation_kind`, and what still dedupes is exactly the intended idempotency case of one caller re-sending the same message on the same conversation while the first is in flight (#6662) (@houko)
- Report a goals storage failure as a 500 instead of an empty page, a leaked error string, or a missing goal.
  `GET /api/goals` folded a substrate read error into the same empty array it returns when nothing has been created yet, so a corrupt blob or an unreadable SQLite file reached the operator as `200 {"items": [], "total": 0}` — indistinguishable on the wire from a daemon with no goals, and the dashboard drew its template-picker empty state over live data it had simply failed to read.
  `GET /api/goals/{id}/children` was worse in both directions at once: a 200 carrying an empty list *and* a raw `format!("{e}")`, so a client checking the status saw success while the body handed out the SQLite path and error chain.
  `POST /api/goals/{id}/start` had the same swallow on a write path — its catch-all `_ => Vec::new()` turned an unreadable store into `404 Goal not found`, sending the operator to re-create a goal that exists.
  All three now log the full error and return the scrubbed 500 envelope that `GET /api/goals/{id}` next to them already used; an absent or non-array key stays a 200 empty result, because that is the genuine not-yet-created shape (#6662) (@houko)
- Add `[registry] auto_sync` so the daemon can be told to stop overwriting the registry checkout.
  `~/.librefang/registry/` is a git clone the sync fast-forwards with `git reset --hard origin/main`, so every local modification under it is destroyed — including the ones `PUT /api/hands/{id}/manifest` writes, which land in `registry/hands/<id>/HAND.toml` whenever the hand shipped with the registry, making the supported way to customise a built-in hand self-erasing.
  Reported as "local edits are overwritten on daemon start, even with the file set read-only, and no config option disables the registry sync".
  The culprit was not boot: `boot_with_config` honours `cache_ttl_secs` (86400 by default) and normally skips, while the background catalog task passes a hard-coded TTL of `0` into `refresh_registry_checkout`, which makes `should_refresh` true for any marker older than a second and forces the reset on its first tick and every 24 h after — so gating boot alone would have changed nothing.
  Setting `auto_sync = false` freezes the checkout on both automatic paths and leaves the explicit ones (`librefang init`, `POST /api/catalog/update`) fetching as before, so freezing does not strand an operator who wants an update.
  `POST /api/hands/reload` never fetched from upstream to begin with — it only reloads hand definitions already on disk into memory — so it is unaffected either way.
  The catalog rebuild is deliberately not gated with the fetch: an operator who froze the registry still gets the models already on disk rather than an empty catalog (#6661) (@houko)
- Stop the config page offering edits the write endpoint refuses, and stop it captioning every `mode` field with the wrong description.
  `GET /api/config/schema` now reports `x-non-writable`, the resolved list of paths `POST /api/config/set` rejects, and the dashboard renders those fields visible but not editable with a line pointing at `config.toml`.
  Reported against `approval.require_approval`, which is deliberately excluded from the write allowlist so a leaked Owner-role API key cannot relax approval policy — the defect was that the UI offered the edit anyway and the save came back as a bare 403, which reads as "saved but not applied".
  The server sends the verdict rather than the allowlists because writability is decided by an exact-path list, section prefixes, a depth-2-only rule and a secret-suffix scrub; re-deriving that in the SPA would make it a third place to keep in sync, and an integration test cross-checks the emitted set against the write endpoint itself.
  Field descriptions are now looked up section-first (`desc_<section>_<field>`, falling back to the bare leaf name), so `exec_policy.mode`, `reload.mode`, `docker.mode`, `privacy.mode` and `sanitize.mode` stop showing the root-level "Kernel operating mode" text while genuinely section-neutral names like `enabled` and `timeout_secs` keep sharing one string.
  Also fixes an edit-loss path in the hand Agent tab: the prompt and tool drafts were overwritten when `useAgentDetail` resolved, so anything typed between opening the tab and the query landing vanished — a new saved value is now adopted only while the draft is untouched (#6663) (@houko)
- Re-enable the `KernelConfig` JSON Schema golden guard, which had been `#[ignore]`d long enough for real schema changes to land unreviewed.
  The attribute was added as a temporary measure over a 490-byte drift, with a comment saying to drop it once the fixture was regenerated; the fixture was never regenerated, so the guard reported "ignored" rather than failing and three subsequent schema changes reached `GET /api/config/schema` without the reviewable diff the fixture exists to force.
  Regenerating shows what slipped through: the `providers` property and its `ProvidersConfig` definition (the org-wide provider allowlist), `ExecPolicy.full_mode_skips_approval` (also reflected in `exec_policy`'s reshaped `default` value), and a reworded `ApprovalPolicy.auto_approve` description.
  Everything else in the diff is key-ordering churn from the schemars output, which is why it is 13,716 lines for three semantic changes and why it is landing on its own rather than buried inside a feature PR (#6664) (@houko)
- Fixed the `#6631` plugin route-classification guard panicking on a Windows checkout, where `include_str!` reflects `routes/plugins.rs` back with CRLF line endings and the guard's `\n}\n` terminator search matched nothing.
  Route extraction now normalises CRLF to LF before parsing and is factored into a shared helper covered by its own regression test that builds the CRLF fixture directly, so the platform-divergent case is provable without a Windows runner (#6665) (@houko)
- Make flipping `[registry] auto_sync` take effect on `POST /api/config/reload`, which #6661 documented but did not deliver.
  `registry` was classified restart-required as a whole section, and `should_store_config` swaps the reloaded config only when a plan carries a hot action or a live-read change — so a registry-only reload was reported as needing a restart and then discarded, and the 24 h catalog task kept reading the old value out of the previous snapshot and clobbering the checkout the operator had just asked it to leave alone.
  `auto_sync` is now classified on its own as a live-read field, matching how the task consumes it, while `cache_ttl_secs` / `registry_mirror` / `registry_host` stay restart-required because they are read when the checkout is set up.
  Boot's `auto_sync` gate also gets its first test, and it asserts on the fan-out copy `sync_registry` performs with no network rather than only on the checkout's contents — an offline runner leaves the checkout alone by accident, which would have made the obvious version of the test pass whether or not the gate existed (#6671) (@houko)
- Reject a commit whose *author identity* attributes it to Claude / Anthropic, not just one whose message does.
  The `commit-msg` hook read only the message, so `GIT_AUTHOR_NAME=Claude git commit -m "fix: …"` passed a spotless message check and still put `Claude <noreply@anthropic.com>` into `git log`, `git blame`, and the GitHub commit list — the same attribution the message rule exists to keep out of the history, arriving through the one field nobody reads while reviewing a diff.
  Nine such commits reached `main` across six PRs merged on the same day before this was noticed.
  Separators are squashed before matching so `Claude Code`, `Claude-Code` and `ClaudeCode` are one case rather than three, whole names are matched rather than substrings so a contributor called Claudia or Claude Dubois is unaffected, and the address test is pinned to the bot mailbox so an Anthropic employee committing under their own address is not blocked either.
  The three `scripts/tests/` corpora for the git-side hooks now run in CI, which they never did — a regression in either attribution predicate was previously catchable only by running them by hand, which is not what anyone does before pushing a hook edit.
  That wiring also exposed `pre-commit-sha-fallback.sh` passing only where `gitleaks` was absent: it built its throwaway repo without a `.gitleaks.toml`, so wherever the tool was installed the hook aborted on a config-load error and the test reported failure (#6672) (@houko)
- Warn instead of silently failing when `api_key_hash` is set with no transmittable key, which is the posture the hash-only documentation itself recommends.
  `build_mcp_bridge_cfg` sends the master key to the daemon's own `/mcp` endpoint on behalf of CLI-based drivers, and a hash cannot stand in for it — a verifier does not yield the secret it verifies.
  So a daemon configured the way the `api_key_hash` doc, the upgrade hint, and `librefang hash-api-key` all advise — hash set, plaintext removed — built the bridge with no bearer, and its own middleware answered 401 on every driver tool call, precisely because #6613 made a hash count as configured auth.
  Nothing reported it: the driver surfaced a failed tool call, and the "nothing transmittable configured" case fell through to an empty string with no log line anywhere on the path.
  The field doc and the CLI hint now name the exception and the two workarounds that keep the secret out of `config.toml` — `api_key = "vault:NAME"` or `LIBREFANG_API_KEY` — and the kernel logs a `WARN` naming the same at every driver rebuild (#6673) (@houko)
- Regenerate the `KernelConfig` schema golden fixture, which #6664 landed one merge behind the source it is generated from.
  #6664 re-enabled the guard and regenerated the fixture, but its branch was cut after #6667 added `api_key_hash` and before #6661 added `registry.auto_sync`, so the fixture it committed described a `KernelConfig` that no longer existed by the time it merged.
  The two PRs never touched the same lines, so nothing flagged the staleness — a regeneration and a new config field conflict semantically without conflicting textually, and the fixture is only correct relative to the commit it was generated at.
  `main` went red on the first push after that, which is the guard doing exactly the job #6664 restored it for: it was off long enough for four schema changes to reach `GET /api/config/schema` unreviewed, and the first thing it caught once re-enabled was real drift.
  The regenerated fixture is reviewed rather than taken on trust, by the same order-insensitive comparison #6664 used, and every one of the sixteen semantic changes traces to a merged PR: `registry.auto_sync` and its default (#6661), `api_key_hash` and the rewritten `api_key` description (#6667), and `additionalProperties: false` on the four `McpTransportEntry` variants plus both `HttpCompat` structs (#6666), which also drops the rendered `default: null` from the two `Option` fields on `HttpCompatHeaderConfig` because schemars stops emitting it for a `deny_unknown_fields` container (#6676) (@houko)
- Accept `mp4` / `mov` / `mkv` / `avi` video containers in `media_transcribe` and `speech_to_text`, extracting and transcribing the audio track server-side instead of rejecting the file on its extension alone.
  The audio track behind these containers already transcribed fine — renaming a `.mp4` to `.m4a` was enough to get a correct transcript — because the only obstacle was an extension allowlist that happened to admit `.webm` (a dual-purpose audio/video container) through a side door while turning away containers that are exclusively video.
  The video-only extensions are budgeted against the existing `MAX_VIDEO_BYTES` limit rather than the tighter `MAX_AUDIO_BYTES` audio uses, and the extraction reuses the ffmpeg piping infrastructure the `.oga` re-mux path already established, factored into a shared helper so both transcodes share one spawn / timeout / kill-on-timeout implementation.
  Unlike the `.oga` re-mux, this always re-encodes to Ogg/Opus rather than copying the source codec: a video container's audio track can be AAC, PCM, Opus, AC3, or anything else ffmpeg can decode, and one deterministic target format is what lets the same Whisper-upload path handle all of them without per-codec branching.
  `.webm` is unaffected — it already reaches the provider unchanged through the existing audio path, and does not need the extraction hop (#6679, #6683) (@houko)

### Changed

- Upgrade `agent-client-protocol` from 0.11.1 to 1.3.0 in the `librefang-acp` crate, migrating the ACP adapter to the 1.x API (supersedes the version-only dependabot bump that left the crate failing to compile).
  The 1.x SDK moved the wire-schema types under a versioned namespace, so every `agent_client_protocol::schema::X` import is now `agent_client_protocol::schema::v1::X` (with `ProtocolVersion` re-exported at the `schema` root); the connection, router, and JSON-RPC surface (`Agent`, `Client`, `ConnectionTo`, `Builder`, `ByteStreams`, `Responder`, `on_receive_*`, `util`) is unchanged at the crate root.
  The companion `agent-client-protocol-tokio` crate has no 1.x release and was pulling a second, older copy of `agent-client-protocol` (and a stale `rmcp` 1.8) into the tree, so it is dropped entirely: its sole use, `agent_client_protocol_tokio::Stdio`, is replaced by `agent_client_protocol::Stdio`, which 1.x exposes at its own crate root with the same `Stdio::new()` constructor (#6526) (@houko)
- Bump `agent-client-protocol` from 1.3.0 to 2.0.0 and update the one call site the rename broke.
  2.0.0 renames `ResponseRouter::respond_with_result` to `route_with_result` (the request-side `Responder::respond_with_result` is untouched and easy to confuse with it), so the catch-all `on_receive_dispatch` handler in `crates/librefang-acp/src/server.rs` that routes `Dispatch::Response` failed to compile against the bumped pin until updated; `cargo check -p librefang-acp --lib` and `cargo check -p librefang-api --lib` both pass clean against the new version (#6600) (@houko)
- Normalize on-disk upload naming so every producer that writes into the shared upload directory names the file `<uuid>.<ext>` instead of today's three divergent schemes (bare `<uuid>`, `image_<uuid>.png`, `<uuid>.<ext>`), keeping the file's type at rest for extension-sniffing tools and for any flow that persists or re-dispatches the bytes.
  A single deterministic `librefang_types::media::on_disk_name(file_id, content_type, filename)` helper (extension from `ext_for_content_type`, then a safe filename extension, else a bare UUID) is now the one naming authority: the API upload / media / session-image / generated-image / browser-screenshot producers all route through it, and the client-facing `file_id` stays a bare UUID so the path-traversal and #3361 owner guards still `uuid::Uuid::parse_str` it.
  `serve_upload` and `resolve_attachments` reconstruct the name through a shared resolver that also tolerates legacy bare-`<uuid>` files and probes `<uuid>.*` for generated images not in the upload registry, so existing uploads keep serving (#6530) (@houko)
- Thread the RBAC-resolved user identity (#3054) into the audit trail for human-initiated API actions that previously wrote un-attributed rows: direct `POST /api/tools/{name}/invoke` (ToolInvoke), backup create / restore, MCP-server add / update / taint-patch / delete, all user & RBAC management mutations (create / update / delete / bulk-import / rotate-key / policy, routed through the shared `persist_users` helper, plus per-user budget set / clear), and dashboard skill-evolution (`skill_evolve:*`); each handler now reads the `AuthenticatedApiUser` from request extensions and passes `user_id` + `channel = "api"` through `record_with_context`, so `/api/audit/query?user=…` (which already filtered) now returns these events attributed rather than only the auth / budget / config paths that were already threaded — the survey confirmed daemon-internal (`DreamConsolidation`, `RetentionTrim`), cron, and agent-to-agent events are inherently userless and stay `user_id = None`, and the new `docs/architecture/audit-user-attribution.md` documents the attributed vs. userless classes and how they are labeled (#6461) (@houko)
- Thread `CanvasConfig.max_html_bytes` / `allowed_tags` into the canvas tool: the `CANVAS_MAX_BYTES` task-local was never `.scope()`d and `allowed_tags` was read nowhere, so both operator knobs were dead config and the tool always used the hardcoded 512 KiB cap + built-in tag allowlist; the agent loop now scopes both task-locals from the resolved config (kernel-populated `LoopOptions.canvas_config`), and an empty `allowed_tags` still falls back to the built-in list (#6446) (@houko)
- Consume `ParallelToolsConfig.mcp_default_safety` / `mcp_readonly_allowlist` in the parallel-tool dispatcher: an unannotated `mcp__server__tool` fell through to `Unknown` → `WriteShared`, so both knobs were inert and read-only MCP calls never parallelised; `plan_batch` now consults them (allowlist → `ReadOnly`, else the `mcp__`-prefix default), with an explicit schema annotation still winning (#6446) (@houko)
- Correct the misleading `channels.file_upload_max_bytes` doc and warn at boot when it is tuned away from the default: the in-process Matrix / Telegram adapters it configured were replaced by out-of-process sidecars (#5317–#5459) and it is no longer threaded into the sidecar send path, so the cap silently did nothing; the doc now states it is unenforced pending a sidecar-crate change and the kernel logs a WARN so an operator is not misled (#6446) (@houko)
- Correct the `prompt_intelligence.hash_prompts` doc and warn at boot when set to `false`: the `content_hash` is load-bearing (it drives prompt-version dedup and is stored NOT NULL) so it is always computed, and the flag was silently ignored; setting it `false` now logs a WARN and the doc reflects that it is effectively always-on (#6446) (@houko)
- Consolidate the four duplicated invisible/format code-point lists that #6141 added — the skills prompt-injection scanner, the runtime injection guard, the prompt-builder sanitizer, and the kernel prompt-context sanitizer — into a single source of truth `librefang_types::text::INVISIBLE_FORMAT_CHARS`, so the set can no longer silently drift between crates and reopen the scanner bypass in the un-updated copy: the three char-only copies now alias the shared const directly and the skills labeled `(char, &str)` table (it needs a per-code-point label for its warning) is guarded by an equality test that fails the build on divergence (#6426) (@houko)
- Stop the release pipeline from generating a stable `librefang` formula in `librefang/homebrew-tap`: the stable CLI now ships from homebrew-core (Homebrew/homebrew-core#290413), so the tap keeping its own copy shadowed the core formula and duplicated maintenance on every release; a stable tag now cascades the newest build into the `beta` / `rc` tap channels only, the keg-only `librefang@<ver>` versioned formula and the desktop cask sync are unaffected, and the release-notes install block now installs the stable CLI directly from core (#6416) (@houko)
- Migrate the Linux system tray implementation in `librefang-desktop` from Tauri's default `tray-icon` to a pure D-Bus implementation using `ksni` 0.3.6.
  This removes `libappindicator` / `libappindicator-sys` from the Linux build graph, eliminating the runtime `dlopen` of `libayatana-appindicator3` and the corresponding CI/Docker package install.
  It does not remove the GTK3 dependency tree (`gtk`, `gdk`, `atk`, …) or resolve the advisories tracked in `deny.toml`'s `ignore` list (RUSTSEC-2024-0411..0420) — those stay transitive via `tauri-runtime-wry` on Linux regardless of the tray implementation, and RUSTSEC-2024-0429 was never in that list to begin with.
  The new implementation re-registers with the `StatusNotifierWatcher` via `ksni` on D-Bus reconnect, updates status properties every second, and supports toggling window visibility on left-click activation (#6572) (@pavver)
- `deploy/docker-entrypoint.sh` now detects whether it started as root and adapts, so the published image satisfies Kubernetes `restricted` Pod Security without a second rootless tag to maintain.
  Under Docker it still starts as root, chowns a bind-mounted `/data`, and drops to uid 1001 through `gosu` — unchanged, so existing Compose deployments keep working.
  Under a pod that pins `runAsUser: 1001` it skips both the chown and `gosu` (neither is possible unprivileged) and verifies `/data` is writable up front, failing with a message that names the `fsGroup` requirement instead of letting SQLite die later on an opaque `EACCES`.
  The image `HEALTHCHECK` now probes `/api/ready` rather than `/api/health`: its consumer is Compose's `depends_on: service_healthy` gate, which is readiness semantics, and `/api/health` can never fail that check (#6632) (#6638) (@houko)
- Refresh the checked-in OpenRouter model snapshot used as the offline fallback catalog.
  The runtime's live catalog remains authoritative whenever OpenRouter is configured, so this update only affects lookups made before the first live fetch completes (#6642) (@houko)
- Move every CI and release workflow off Node 20 onto Node 24, which is what the dashboard's own test runner now requires.
  Node 20 left active LTS support in April 2026, and the dependabot bump of jsdom 29 → 30 (#6658) surfaced the consequence: jsdom 30 pulls an undici that calls `webidl.util.markAsUncloneable`, an API that only exists from Node 21, so every one of the 91 dashboard test files failed to start its vitest worker with `TypeError: webidl.util.markAsUncloneable is not a function`.
  That is a runtime incompatibility rather than a test failure — the same suite passes on a Node 24 host — so pinning jsdom back would have deferred the upgrade rather than fixed anything.
  All eleven `node-version: 20` pins move together instead of only the one that runs the tests: `dashboard-build`, `mobile-smoke`, `release-cli`, `release-desktop` and four jobs in `release` all build the same dashboard, so leaving any of them on 20 would have turned one red check into a release-time failure discovered later.
  Six pins in the repository were already on 24, so this makes the runtime uniform rather than introducing a new one (#6675) (@houko)
- Ship debug symbols as a separate release asset, so a crash report from a released build can actually be symbolized.
  `[profile.release]` carried `strip = "symbols"` and set no `debug` key at all, so nothing about a shipped binary was recoverable — which is where #6659 stalled: its crash report shows a six-function cycle repeating inside a 52-frame window, an unbounded recursion whose culprit is one `atos` invocation away, and that invocation could not be run.
  Offsets from an existing crash report cannot be mapped onto a later rebuild, and `lto = "fat"` with `codegen-units = 1` makes a local symbolized rebuild expensive enough that the reproduction window closed first.
  The profile now also sets `debug = "line-tables-only"` and `split-debuginfo = "packed"`, which puts function names and `file:line` for every frame into a `.dSYM` bundle on macOS and a `.dwp` file on Linux, beside the executable rather than inside it.
  `strip = "symbols"` still applies to the executable, so the binary users download is unchanged; the release workflow uploads the split file as its own `librefang-<target>-debug-symbols.tar.gz` asset, which only someone symbolizing a crash needs to fetch.
  `line-tables-only` matches what `[profile.dev]` has used since #1805 and is what keeps the cost small — it omits the variable-inspection DWARF that dominates debug-info size while keeping every frame nameable.
  The macOS job fails if the bundle is missing, since its absence there means the profile change silently regressed; the Linux job warns instead, because those targets are cross-compiled and an absent split file must not take a release down over a diagnostic aid (#6677) (@houko)
- Refresh the checked-in OpenRouter model snapshot used as the offline fallback catalog.
  The runtime's live catalog remains authoritative whenever OpenRouter is configured, so this update only affects lookups made before the first live fetch completes (#6682) (@houko)

### Security

- Add LibreFang daemon lifecycle commands and mutating writes against the daemon's own SQLite database to the dangerous-command denylist, which screens every `shell_exec` under all exec modes including `full`.
  The list already blocked `python[23]? -c`, so a diagnostic one-liner scoped to the calling agent was refused while `librefang stop` — which bounces every other agent and channel adapter sharing the daemon — passed unimpeded; #6594 reports an agent taking exactly that route ten times in one morning, leaving some channel adapters with a broken outbound path afterwards.
  The lifecycle entry matches `start` / `stop` / `restart` by bare name, by path (`target/release/librefang`, `/usr/local/bin/librefang`), with the Windows `.exe` suffix, through the `gateway` alias subcommand, and behind the CLI's one global option and its separate value token (`librefang --config <path> stop`); `librefang status` and every other read-only subcommand are deliberately not matched.
  The database entry matches mutating SQL statement forms (`insert into`, `replace into`, `update <table> set`, `delete from`, `drop table|index|view|trigger`, `alter table`) against `librefang.db`, plus output redirection that would truncate the file.
  It is written against statement forms rather than bare verbs, and scoped to the daemon's own database rather than any `.db`, so that read-only inspection stays allowed: a match is a hard block in `manual` mode, and blocking `sqlite3 librefang.db "select … from usage_events …"` would have blocked the investigation that produced #6606.
  `select`, `.schema`, `.dump`, a redirect into a separate backup file, and a `select` that merely mentions `'delete'` as a value are all covered by regression tests as staying safe (#6594) (@houko)
- Attribute an authenticated API caller's own-key spend to that caller and enforce their per-user budget, closing a bypass the per-user provider-credential feature (#6460 / #6483) opened: the owner threaded into `resolve_driver_for_owner` selects and bills the user's own vault key upstream, but usage attribution and the per-user budget gate keyed only on the request-body `sender_context`, which is absent on a plain authenticated POST (`identify("api","")` → `None`). So an authenticated user with a stored key and a configured `[budget]` could POST `/api/agents/{id}/message`, `/message/stream`, or an ephemeral `/btw` with no `sender_id`, spend on their own key past their cap indefinitely, and show zero on `/api/budget/users`; worse, because attribution was taken from the unauthenticated request body, a caller could pass a `channel_type` + `sender_id` mapping to another user's binding and have their own-key spend booked against that user. The three post-call write sites (`execute_llm_agent`, `send_message_ephemeral`, and the streaming task) now record `UsageRecord.user_id` and run the per-user budget check on `owner.or(attribution_user_id)` — the authenticated owner wins when present, sender-derived attribution stays the fallback for owner-less paths (channel / cron / agent_send). The streaming path keys on the fork-nulled `effective_owner` (not the raw owner) so a sub-agent's spend is never mis-attributed to the parent turn's user, and a plain authenticated POST that spoofs a `sender_id` can no longer book spend against someone else (#6514) (@houko)
- Canonicalize the provider name before the per-user key lookup so an alias-configured agent bills the user's own key instead of silently falling back to the operator's global credential (a chargeback leak). `resolve_driver_for_owner` looked the owner's stored key up by the raw manifest provider string (`get_user_provider_key(uid, "google")`), but the Owner CRUD's `validate_provider` only accepts canonical `known_providers()` names, so a user can only store the key under the canonical `gemini` — the lookup for an agent with `provider = "google"` (an alias of `gemini`) missed, `user_scoped_key` was `None`, and the operator's global `GEMINI_API_KEY` was billed for the user's turn while per-user attribution/budget for that turn was wrong. A new `librefang_llm_drivers::drivers::canonical_provider_name` (aliases → canonical registry name, unknown names pass through) is applied to the manifest provider before the vault lookup, so the read shares the one namespace the write surface stores under. Reads and writes were already canonical for non-alias providers, so this is a no-op there; only alias-configured agents change behavior (#6517) (@houko)
- Stop the outbound taint/DLP heuristic from false-positive-blocking legitimate MCP tool arguments that contain a long unformatted numeric id (#6499). Two rules over-blocked (fail-closed): (1) a bare export-attachment filename like `note-<id>-<id>.pdf` (no directory separator) tripped `OpaqueToken` because `looks_like_path` only recognized strings with a `/` as structured — `looks_like_path` now also excludes a bare basename with a letter-initial extension (`^[\w.\-]+\.[A-Za-z][A-Za-z0-9]{0,7}$`), so a numeric-suffixed token like `abc123.9876543` is still scanned; (2) the phone regex makes every separator optional, so any contiguous digit run matched its digit-group floor and was flagged `PiiPhone` — a match is now rejected only when the full contiguous digit run it sits in exceeds 16 digits (E.164 caps a real number at 15), leaving 10-15 digit numbers to the phone rule and 13-16 digit runs to the card rule. Genuine opaque tokens, real phone numbers, and card numbers still fire (counter-tests included); the card rule's own handling of >16-digit runs is out of scope here. The runtime `McpTaintPolicy` per-path skip-rules remain, but the default heuristic no longer requires operators to allowlist their own filenames and ids. (#6499) (@houko)
- Scope the knowledge graph (entities/relations) per user on multi-user agents, closing a cross-user data leak that mirrored the memories one #6493 fixed: the `entities`/`relations` tables were keyed on `agent_id` only (no per-user column, unlike `memories.peer_id`), so every user routed through one agent could read every other user's KG facts. A new migration (v47) adds a `peer_id` column to both tables — the shared/unscoped peer is the empty-string sentinel `''`, not SQL NULL, because NULL is distinct-from-NULL in a UNIQUE/PRIMARY KEY and would let the same shared entity duplicate — and rebuilds `entities` with a composite `PRIMARY KEY (id, peer_id)` so two users' same-named entities (which normalize to the same deterministic id) coexist as distinct rows instead of one silently overwriting the other. `peer_id` threads from the tool dispatcher (using the turn's `sender_id`, exactly as the memory tools do) through the `Memory` / `KnowledgeGraph` traits into the store, where the read query filters `r.peer_id` and the entity JOIN ties a matched entity to the relation's peer so a shared id never resolves across users. An unset peer is an unscoped read returning every peer's rows (shared semantics, matching memories); existing rows migrate to `''` and single-user agents are unaffected. The dashboard/admin relations endpoints stay agent-wide, with an optional `?peer_id=` filter on the read. (#6494) (@houko)
- Stop a compacted-session summary from leaking across users on a multi-user agent: `SessionStore::canonical_context` — the reader on the prompt-assembly path — returned the agent-scoped `compacted_summary` unconditionally, ignoring the `compacted_summary_session_id` owner column that #6225 added, so when several `[[users]]` shared one agent, a summary built from user A's conversation was injected verbatim into user B's turn (up to a 4000-char excerpt of A's private messages at prompt index 0). The #6225 fix had scoped only the dashboard `/session` banner (`compacted_summary_for_session`) and left this injection reader ungated; `canonical_context` now applies the same owner check — a summary is surfaced only to the session that owns it, while a legacy ownerless row or an unscoped (`None`) agent-wide read is unchanged. Exposed by default (the leak path runs whenever `stable_prefix_mode` is false, the default). The default heuristic summary builder in `append_canonical` (used when no LLM summary is configured) had the same class of leak from the write side: it folded every compacted message — across all sessions — into one summary and then stamped it with the single session that happened to cross the threshold, so on a busy multi-user agent user A's private messages could be folded into a summary stamped `owner = B`, which the read gate would then hand to B. The fold now includes only the triggering session's own entries (plus legacy untagged ones), mirroring the read-path filter, and only carries forward an existing summary that the same session owns — so the summary's content matches its owner stamp and the gate is honest. When the compacted batch contains none of the triggering session's messages the prior summary and owner are left untouched rather than an empty summary being stamped; the message trim is unchanged (#6493) (@houko)
- Extend the #6443 cross-account `channel_send` guard to the `/mcp` bridge: the subprocess `claude-code` driver now forwards the turn's originating bot account on the bridge connection as `X-LibreFang-Current-Account-Id` (a new `CompletionRequest.sender_account_id`, populated from the same kernel metadata stamp as the in-process path) and `mcp_http` rehydrates it into the tool execution context, so an explicit `account_id` targeting another tenant's bot account is rejected on the bridge path exactly as in-process — closing the parallel surface the #6449 fix left unchanged (#6443) (@houko)
- Reject a cross-account (cross-tenant) `channel_send`: the tool's `account_id` parameter — which selects the registered bot instance a message routes through — was passed straight from the model to the kernel send path with no validation, so on a multi-tenant daemon (several bot accounts, each with its own `default_agent` serving a different customer) an agent induced via model hallucination or prompt injection could dispatch content into a different tenant's chat; the kernel now stamps the turn's originating account into the manifest and the in-process tool rejects an explicit `account_id` that differs from it on the same channel, mirroring the #6117 recipient guard's explicit-value-only scoping (the `/mcp` bridge path is a noted parallel surface, unchanged here) (#6443) (@houko)
- Scrub the daemon environment before `process_start` spawns a persistent process, mirroring `shell_exec` / `LocalBackend`: `ProcessManager::start` built the child command without `env_clear()` + the safe-var allowlist, so under the default `Allowlist` posture (where `env` is a safe_bin) an agent could `process_start {"command":"env"}` then `process_poll` and read back the daemon's full environment — including `LIBREFANG_VAULT_KEY` (offline decryption of every stored credential) and env-provided provider API keys; the child now inherits only the safe baseline plus the agent's resolved `allowed_env_vars` (#6446) (@houko)
- Enforce caller ownership on `process_poll` / `process_write` / `process_kill`: they looked a process up by the globally-sequential, guessable `proc_N` id with no owner check, so a second agent could read another agent's stdout (secret disclosure), inject into its stdin (code execution in the victim's interpreter), or kill it (DoS); the caller's agent id is now threaded down and a mismatched owner is reported as "not found" (no cross-agent existence oracle), matching the ownership model `start` / `list` already enforced (#6446) (@houko)
- Stop a persistent plugin hook subprocess from inheriting the daemon's entire environment: `PersistentProcess::spawn` did `env_clear()` then re-added every var from `std::env::vars()`, so a plugin that merely set `[hooks] persistent_subprocess = true` received `LIBREFANG_VAULT_KEY` and all provider keys while the default non-persistent path allowlists only a safe baseline; both spawn paths now share a `hook_baseline_env` helper that re-adds only PATH/HOME/runtime-passthrough/`allowed_env_vars` (filtered through `is_blocked_env_var`) + `plugin_env` (#6446) (@houko)
- Owner-gate `GET /api/config/export`: it returned the raw on-disk `config.toml` verbatim — including the inline plaintext master `api_key`, `network.shared_secret`, and provider/channel credentials that the sibling `GET /api/config` redacts — and, as a plain GET, was reachable by any authenticated role (Viewer / User / Admin), so a leaked master `api_key` (which re-presents as `Owner`) was a full privilege escalation; it now requires `Owner` via `min_role_for_privileged_get`, matching the Owner-only gating of `/api/config[/set|/reload]` (#6446) (@houko)
- Authorize channel slash-commands through the RBAC gate before dispatch: the typed-command and text/button command paths `return`ed above the chat-path `authorize_channel_user` gate, so with RBAC / an allowlist configured an unauthorized user — rejected on any normal message — could still `/approve` pending tool calls, spawn/switch agents, reset another user's session, or run control-plane commands; both command paths now run the same gate first (a no-op when RBAC is disabled) (#6446) (@houko)
- Enforce `webhook_triggers.max_payload_bytes` at the wire level on `/hooks/wake` and `/hooks/agent` (both the `/api/hooks/*` and unversioned aliases): the endpoints previously inherited only the global 8 MiB body cap, so the documented per-webhook cap was dead config and the routes were ~128x more permissive than advertised; a `RequestBodyLimitLayer` sized from the config now bounds them (#6446) (@houko)
- Gate the `process_start` tool through the same exec allowlist + dangerous-command checks as `shell_exec` before spawning: it was dispatched straight to the process manager with no gate, so a default agent — under both the `Allowlist` and the operator-hardened `Deny` exec postures — could `process_start /bin/sh -c '…'` for arbitrary command execution that `shell_exec`'s allowlist / metacharacter / taint / approval chain would have blocked (#6441) (@houko)
- Require Owner (not Admin) for MCP-server config mutations (`POST /api/mcp/servers`, `PUT` / `DELETE /api/mcp/servers/{name}`): a persisted stdio-transport server is a raw command spawned under the daemon UID — the same process-spawn privilege `install-deps` is Owner-gated to protect — so an Admin "config write" role could reach RCE; reads and the non-spawn `reconnect` / `taint` / `auth` sub-resources keep their Admin gate (#6441) (@houko)
- Role-gate the terminal and agent WebSocket GET upgrades before the blanket "GET is read-only" RBAC rule: `GET /api/terminal/ws` (and tmux window management) require Admin+ because they spawn a PTY under the daemon UID, and `GET /api/agents/{id}/ws` requires User+ because it drives full agent turns (tool execution, budget spend) — a Viewer per-user key previously obtained an interactive shell and could trigger LLM turns the REST path denied it (#6441) (@houko)
- Scope `GET /api/memory/agents/{id}/relations` to its path `agent_id` behind the proactive-namespace read guard: the handler discarded the id and ran an unscoped knowledge-graph query, returning every agent's relation triples to any authenticated caller and bypassing the namespace ACL (#6441) (@houko)
- Add a post-canonicalize `starts_with(workspaces_root)` containment check to named-workspace relative `path` declarations (mirroring the `mount` branch) and guard `create_dir_all` against escaping symlinks, so a symlink inside the workspaces tree can no longer resolve a `[workspaces]` path to an arbitrary host directory that is then trusted as a sandbox root (#6441) (@houko)
- Scan the skill `description` and tool name / description through the prompt-injection scanner at the load boundary and at create time, closing the gap where only `prompt_context` was scanned even though `description` is inlined into the `<available_skills>` prompt block (#6441) (@houko)
- Make per-approval TOTP replay prevention atomic (hold the claim lock across check → verify → record) so a single single-use code cannot authorize more than one concurrent approval, and widen the replay-record window so it covers the code acceptance / skew window (#6441) (@houko)
- Write migrated OpenClaw channel secrets `0600` from creation instead of via a post-hoc chmod, closing the world-readable umask window (#6441) (@houko)
- Bound the marketplace skill download and zip decompression (compressed size, uncompressed size, ratio, entry count) before the audit / extract, and bound the desktop-app installer download while no longer stripping the macOS Gatekeeper quarantine (#6441) (@houko)
- Apply the reserved-system-channel defense (`resolve_scope_channel`) on the non-streaming and `execute_llm_agent` session resolvers so an external caller cannot poison the internal cron / autonomous / webui sessions, warn on sidecar instance names that collapse to the same per-instance secret prefix, and collapse unmatched-route requests to a single Prometheus `path` label to remove an unauthenticated unbounded-cardinality memory-exhaustion DoS (#6441) (@houko)
- Close the WebSocket and terminal auth bypass that a hash-only master credential would otherwise have opened.
  Both upgrade paths derived "is auth configured?" as `!valid_api_tokens(..).is_empty()`, which was a sound proxy only while `api_key` was the single way to configure a master key: a daemon whose key exists solely as `api_key_hash` lists no plaintext token, so that test reported it as unauthenticated.
  The terminal path then drove `decide_auth` past its reject branch into `LocalBypass` — unauthenticated shell access on a daemon the operator believes is bearer-gated — and the WebSocket path skipped its whole auth block, re-opening the openfang #1034 B2 branch for the new config shape, while a caller presenting the *correct* key was rejected because there was nothing to compare against.
  Both sites now share one `master_auth_required` derivation and fall back to verifying the presented token against `api_key_hash`, so the next auth surface cannot re-derive it wrongly.
  That shared derivation reads the live auth handles the HTTP middleware already reads, rather than re-resolving the credential from a config snapshot per connection, which also stops a `vault:NAME` master key from costing an OS keyring read plus a vault-file decrypt on every WebSocket and terminal upgrade — twice each, on paths reachable before any credential has been presented (#6613) (@houko)
- Bind `POST /api/auth/refresh` to the session the caller can prove it owns.
  The endpoint used to fall back to scanning the process-global token store for any entry matching a `provider` hint — or, with neither field supplied, for literally any entry that had a refresh token — and it is reachable by any Admin, so a caller could refresh a different local user's upstream session and be handed their access and rotated refresh tokens with every scope that user had granted.
  Both fallbacks are removed: a request now presents either its own `refresh_token` — which `/api/auth/callback` returns to the client, deliberately, so this is the ordinary path — or, for a client that kept only the other half, the `access_token` that callback issued it, which the server matches in constant time against the stored entry it belongs to.
  Neither is a caller-supplied assertion like `sub` or `provider`, and no ownership can be inferred from the store itself, which is keyed by upstream OIDC subject with no record of which local user owns an entry.
  A blank string in either field counts as absent rather than selecting a branch, and the store lookup rejects an empty access token outright so that an identity provider returning one cannot leave an entry that matches a caller who presented nothing — a request that proves nothing gets a 400 instead of someone else's credentials (#6629) (#6639) (#6644) (@houko)
- Stop `GET /api/mcp/servers` and its `{name}` detail sibling from serializing MCP environment values.
  The `env` list is documented as variable names to pass through, but the supported representation also accepts an inline `KEY=VALUE`, so an operator could put a live credential there and any reader got it back verbatim.
  The report describes a Viewer-role caller; it is worse than that — the list route sits in `PUBLIC_ROUTES_DASHBOARD_READS`, so with `require_auth_for_reads` left at its default an unauthenticated caller could read those values too.
  Both routes now return variable names only, and the write path merges a submitted bare name against what is stored: redacting the read side alone would have been a data-loss bug worse than the disclosure, because the dashboard hydrates its edit form from the list response and submits every field back on save, which would have wiped the very credentials the caller was never shown (#6630) (#6639) (@houko)
- Require Owner authorization for every plugin route that can put plugin-controlled code on an execution path.
  Admin is "config write" by design, but it could previously install a Git-backed plugin and then invoke dependency installation, so npm / pip / Bundler / Composer ran attacker-supplied package lifecycle scripts under the daemon UID — crossing the Admin/Owner boundary into arbitrary code execution with access to daemon secrets.
  Owner is now required for `install`, `install-with-deps`, `install-deps`, `test-hook`, `upgrade`, `enable`, `reload`, `prewarm`, and `sign`; `sign` is included because load-time integrity verification rejects a hook whose hash no longer matches, so re-signing is what makes a tampered script loadable again.
  `install-with-deps` and the batch `prewarm` route are gated alongside their per-name/singular siblings: both call the identical underlying function (`install_plugin_with_deps`, `reload_plugin`) that the singular `install` and `reload` gates already cover, just through a top-level path with no `{name}` segment to match on.
  `uninstall`, `disable`, and `scaffold` deliberately stay at Admin: the first two *remove* code from the execution path, and gating them would leave an Admin unable to shut a malicious plugin off during an incident (#6631) (#6639) (@houko)

### Documentation

- Add the missing YAML front matter to the v2026.7.27 release article.
  The publish workflows both key off it: dev.to skips an article with no `title`, and the release Discussion body is extracted with `awk` that prints only after the second `---`, so it came out empty.
  The generator `scripts/changelog-to-article.sh` emits the block correctly — this article was written without it (#6602) (@houko)
- Fix the v2026.7.27 highlight that described `service install --system` as starting LibreFang "at login".
  The feature is a boot-time LaunchDaemon, which is the whole reason it exists, and the authoritative entry a few hundred lines above already said so (#6602) (@houko)
- Correct the NixOS off-host recipe, which annotated its `environmentFile` as "must export an API key".
  An environment file cannot set `api_key`; the only authentication the daemon reads from the unit environment is a dashboard credential pair (#6602) (@houko)
- Document the EveryAPI MCP bridge on the MCP/A2A integrations page (EN + zh mirror), and fix the `[[mcp_servers]]` examples that page already carried.
  The [EveryAPI](https://github.com/everyapi-ai/everyapi) CLI ships an MCP stdio server as `everyapi mcp` and LibreFang is an MCP stdio client, so the integration is two config stanzas and no code: a `[[mcp_servers]]` entry plus the server's name in the target agent's `mcp_servers` allowlist, with `env = []` because the stdio transport already forwards `HOME` and the server reads `~/.config/everyapi/credentials.json` itself.
  The page also carries an optional `~/.librefang/mcp/catalog/everyapi.toml` entry for `librefang mcp add everyapi`, the 15-tool read/write inventory, and the fact that LibreFang's `mcp_{server}_{tool}` namespacing does not de-duplicate — EveryAPI's already-`everyapi_`-prefixed tools become `mcp_everyapi_everyapi_*`, so an approval glob written against the single-prefix form silently matches nothing.
  Four `require_approval` globs gate all 8 write tools and leave the 7 read tools free, which is what keeps the unattended balance-check use case working; a blanket `mcp_everyapi_*` would stall it on an approval prompt.
  The security section states the bypasses rather than implying the globs are a hard block: `require_approval` pauses for a human instead of denying, `trusted_senders` does NOT waive it for MCP tools (see the classifier fix below), a channel `allowed_tools` rule short-circuits ahead of the list, a `hand:`-tagged agent auto-approves everything, `cache_approvals_per_session` defaults to `true` so one approval covers the rest of the session, and per-user RBAC `NeedsApproval` is the only gate all of those respect.
  A new `crates/librefang-extensions/tests/everyapi_catalog_entry.rs` extracts the fenced TOML straight out of the MDX and runs it through the real loader and the production `format_mcp_tool_name`, so the documented catalog entry, config stanza, and approval globs cannot drift from the code (#6589) (@houko)
- Fix every unparseable `[[mcp_servers]]` example in the docs and add a test that walks the whole tree so the class cannot come back.
  `McpServerConfigEntry` is `deny_unknown_fields`, so a stanza in the pre-`transport` schema is not a stylistic slip that degrades gracefully — it is a hard parse error that takes the daemon's entire `config.toml` down with it, and a user copying one got a daemon that would not boot.
  Twelve stanzas across six page pairs were broken in three distinct shapes, all copy-paste descendants of an older schema: top-level `command` / `args` with `env = { … }` as a table on the MCP/A2A integrations page, top-level `command` / `args` on the operations FAQ, a bare top-level `url` on the core configuration page, and a `[mcp_servers.transport.Http]` sub-table naming the enum variant in the table path instead of carrying the `type` tag that `#[serde(tag = "type")]` requires on the features configuration page.
  `crates/librefang-extensions/tests/docs_mcp_servers_examples.rs` now walks every `.mdx` under `docs/src/app`, extracts each stanza out of the fenced TOML (absorbing `[mcp_servers.…]` sub-tables so a multi-table entry is not falsely reported), and deserializes it through the real type; all 36 pass, new pages are covered with no list to maintain, and the test was mutation-checked against all three broken shapes (#6592) (@houko)
- Document how `[approval].trusted_senders` composes with `[[users]]` RBAC on the approvals security page (EN + zh mirror): the two are separate trust surfaces and the per-user RBAC gate is evaluated first, so an ID listed in `trusted_senders` that is not also a registered `[[users]]` on the `api` channel still has its low-risk tools (e.g. `memory_*`) gated by the `guest_gate`, because the forced-approval verdict short-circuits before the `trusted_senders` bypass is consulted; the new subsection gives the concrete fix (register the operator as a `[[users]]` bound to the `api` channel with a `tool_policy` covering the tools it drives) and notes that with no `[[users]]` configured `trusted_senders` works standalone (#6492) (@houko)
- Document per-user provider-key precedence on the provider-management page (EN + zh mirror), ratifying #6460 OQ#5 that a user's own stored key wins over the operator's rotation: the new "Per-user keys and rotation precedence" subsection states the full resolution order (org allowlist > agent-pinned `api_key_env` > the user's stored key via `PUT /api/users/{name}/provider-keys/{provider}` > `credential_pools` > `provider_api_keys` / `auth_profiles` rotation > catalog / convention env), explains that a user key bypasses the pool and rotation for that provider so upstream spend and chargeback attach to the human who brought the key while an agent-pinned `api_key_env` still overrides it, and names the two operator-facing failure modes — a user key is a hard single point of failure on the primary provider with no same-provider fall-through to the operator's pool, and a configured fallback chain can fail a user's key over onto the operator's credential for a different fallback provider they have not supplied a key for; docs-only, the `resolve_driver_for_owner` resolver already implements this behavior (#6460) (@houko)
- Document installation from the signed project-maintained Arch Linux pacman repository while AUR account registration is unavailable (#6386) (@pavver)
- Add `docs/architecture/multi-replica-rfc.md`, which enumerates every singleton subsystem that blocks running more than one daemon replica and proposes a four-phase path through them.
  Replacing SQLite is necessary and nowhere near sufficient: 24 named background workers, the in-process session locks, the audit hash chain, and the in-memory cost-reservation ledger each break in a different way under a second replica, and the document assigns a coordination mechanism to each rather than leaving "HA" as an open aspiration.
  The storage and coordination decisions are explicitly marked as needing maintainer approval before any implementation starts (#6634) (#6638) (@houko)

### Added

- Give the master api_key env/vault indirection and a hashed form, and close the hash-only WS/terminal auth bypass (#6667) (@houko)

### Fixed

- Inherit global exec_policy on hand activation and stop inferring autonomy from max_iterations (#6603) (@houko)
- Expose writable config fields in GET /api/config and guard against read/write drift (#6604) (@houko)
- Require an explicit [autonomous] or schedule declaration to start a hand loop (#6610) (@houko)
- Merge partial identity PATCHes and flag tool_allowlist entries that cannot grant (#6615) (@houko)
- Expose non-writable security config on read and guard writable paths against missing fields (#6618) (@houko)
- Base the status indicator on supervisor liveness instead of shared per-type traffic (#6620) (@houko)
- Render every approval decision distinctly instead of labelling unknown states "Edited" (#6621) (@houko)
- Let the global require_approval list survive exec_policy Full, and guard daemon lifecycle commands (#6622) (@houko)
- Repair the kubernetes workflow YAML, which never parsed, and guard the whole class (#6643) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Documentation

- Fix typo in prompts.rs comment (#6655) (@houko)

### Maintenance

- Update model snapshot (#6591) (@houko)
- Bump the cargo-minor-patch group with 10 updates (#6597) (@app/dependabot)
- Bump serial_test from 3.5.0 to 4.0.1 (#6598) (@app/dependabot)
- Bump jsonwebtoken from 10.4.0 to 11.0.0 (#6599) (@app/dependabot)
- Bump base64 from 0.22.1 to 0.23.0 (#6601) (@app/dependabot)
- Update model snapshot (#6616) (@houko)
- Bump docker/login-action from 4.4.0 to 4.5.2 in the actions-minor-patch group (#6626) (@app/dependabot)
- Bump actions/stale from 10.4.0 to 11.0.0 (#6627) (@app/dependabot)
- Bump the web-minor-patch group in /web with 4 updates (#6656) (@app/dependabot)
- Bump the dashboard-minor-patch group in /crates/librefang-api/dashboard with 6 updates (#6657) (@app/dependabot)
- Bump jsdom from 29.1.1 to 30.0.1 in /crates/librefang-api/dashboard (#6658) (@app/dependabot)
- Bump the docs-minor-patch group in /docs with 5 updates (#6685) (@app/dependabot)

</details>


## [2026.7.27] - 2026-07-27

_33 PRs from 2 contributors since v2026.7.21._

### Highlights

- **EveryAPI integration** — connect EveryAPI as a model provider via `librefang models connect everyapi` from the CLI or the new Providers page connect action, with a built-in wiring doctor check
- **macOS boot-time service** — `service install --system` installs a LaunchDaemon so LibreFang starts automatically at login without a running user session
- **NixOS & additional Linux distros** — first-class NixOS deployment support with deepin/Debian distro awareness out of the box
- **Audit log integrity** — audit entries are now preserved (WORM-protected) when an agent is purged, preventing accidental evidence loss
- **Tool result and context fidelity fixes** — stops lossily truncating tool results in the spill dead band, and correctly honors `context_window` from `agent.toml` on all turn-execution paths

### Added

- Emit a failure metric for media-understanding (vision/STT) (#6538) (#6551) (@houko)
- First-class NixOS deployment and deepin/Debian distro awareness (#6582) (@houko)
- Add `librefang models connect everyapi` and an EveryAPI wiring doctor check (#6583) (@houko)
- Add `service install --system` for a boot-time LaunchDaemon on macOS (#6584) (@houko)
- Add an EveryAPI connect action to the Providers page (#6586) (@houko)

### Fixed

- Override sharp to >=0.35.0 to clear the libvips high advisory (#6546) (@houko)
- Include openrouter-models.snapshot.json in the flake source (#6547) (@houko)
- Anchor credential-prefix secret patterns to stop false positives (#6541) (#6548) (@houko)
- Make skills/reload honest in frozen Stable mode (#6540) (#6549) (@houko)
- Stop lossily truncating tool results in the spill dead band (#6545) (#6550) (@houko)
- Bump next to 16.2.11 to clear the App Router security advisories (#6557) (@houko)
- Don't delete audit_entries when purging an agent (WORM integrity) (#6553) (#6558) (@houko)
- Apply busy_timeout to every pooled connection in modify() test helper (#6561) (@houko)
- Treat blank parent_id / agent_id as absent instead of 404 (#6577) (@houko)
- Collect new_name on clone, reflect mcp_servers grants in Tools tab (#6578) (@houko)
- Resolve callback message_id from either metadata shape or the native id (#6579) (@houko)
- Honour agent.toml context_window on the paths that run a turn (#6580) (@houko)
- Install and search from the synced registry checkout (#6581) (@houko)
- Give the TUI a process-lifetime runtime (startup panic + silent session-summary loss) (#6585) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Documentation

- Move the misplaced `### Fixed` block out of the `### Added` list (#6587) (@houko)

### Maintenance

- Update model snapshot (#6539) (@houko)
- Bump the actions-minor-patch group across 1 directory with 4 updates (#6542) (@app/dependabot)
- Bump actions/labeler from 6.2.0 to 7.0.0 (#6543) (@app/dependabot)
- Bump actions/setup-python from 6 to 7 (#6544) (@app/dependabot)
- Update model snapshot (#6552) (@houko)
- Bump the web-minor-patch group in /web with 10 updates (#6554) (@app/dependabot)
- Bump the dashboard-minor-patch group across 1 directory with 12 updates (#6555) (@app/dependabot)
- Bump @testing-library/jest-dom from 6.9.1 to 7.0.0 in /crates/librefang-api/dashboard (#6556) (@app/dependabot)
- Update model snapshot (#6563) (@houko)
- Bump the docs-minor-patch group in /docs with 6 updates (#6567) (@app/dependabot)
- Update model snapshot (#6571) (@houko)
- Update model snapshot (#6574) (@houko)
- Update model snapshot (#6576) (@houko)

</details>


## [2026.7.21] - 2026-07-21

_61 PRs from 4 contributors since v2026.7.11._

### Highlights

- **Per-user LLM provider credentials** — users can bring their own API keys with per-owner spend attribution, org-wide provider allowlists, and budget enforcement on authenticated turns
- **MCP resources primitive** — agents can now access MCP-exposed resources, rounding out the MCP integration beyond tools alone
- **Multi-user knowledge graph scoping** — the knowledge graph is now partitioned per user on shared agents, closing a cross-user data leak
- **Slack multi-step progress** — long-running agent tasks surface live phase updates in Slack via Block Kit message edits instead of a single final reply
- **HAND.toml online editing & expanded delivery targets** — edit hand manifests directly from the Hands panel in the dashboard, and delivery-target channel presets now reach all sidecar adapters

### Added

- Env opt-out (TELEGRAM_STREAMING) for the streaming path (#6482) (@houko)
- Per-user LLM provider credentials with per-owner usage attribution (initial) (#6483) (@houko)
- Org-wide LLM provider allowlist (fail-closed at driver resolution) (#6484) (@houko)
- Slack multi-step progress display via AgentPhase-driven Block Kit updates (#6487) (@houko)
- Per-user attribution survey + API-level user filtering of audit queries (#6488) (@houko)
- Process_start completion notification via the async task tracker (#6489) (@houko)
- Edit HAND.toml online from the Hands panel (#6490) (@houko)
- Scope the knowledge graph per user (peer_id) on multi-user agents (#6494) (#6502) (@houko)
- Expand delivery-target channel presets to all sidecar adapters (#6506) (@houko)
- Owner-gated CRUD for per-user provider credentials (#6460) (#6509) (@houko)
- Implement the MCP resources primitive (#6501) (#6532) (@houko)

### Fixed

- Prefer the live model catalog with a build fallback (#6384) (@pavver)
- Security and correctness hardening from repo-wide audit (#6438) (@houko)
- Second-pass security and correctness hardening from repo-wide audit (#6439) (@houko)
- Third-pass security and correctness hardening from repo-wide audit (#6441) (@houko)
- Fourth-pass security and correctness hardening from repo-wide audit (#6446) (@houko)
- Resolve four reported bugs (#6423, #6442, #6443, #6444) (#6449) (@houko)
- Enforce cross-account channel_send guard through the /mcp bridge (#6443) (#6455) (@houko)
- Trust operator env allowlist in sandbox_command (#6465) (@houko)
- Treat retired pnpm audit endpoint as skip, not a dependency issue (#6466) (@houko)
- Field-scope dm_policy/group_policy so a partial override stops silently gating groups, and expose them on [[sidecar_channels]] (#6445) (#6468) (@houko)
- Distinguish context and budget limits (#6479) (@houko)
- Allow dashboard login script under CSP (#6480) (@houko)
- Treat auto_dream fork tool calls as system-internal so RBAC does not gate them (#6485) (@houko)
- Login page unreadable in light theme (CSS cascade source-order bug) (#6486) (@houko)
- Pin login_page.html to LF so the CSP-hash test passes on Windows (#6481) (#6496) (@houko)
- Gate compacted summary by owning session to stop cross-user prompt leak (#6493) (#6497) (@houko)
- Honour glob patterns in per-agent tool_allowlist/tool_blocklist (#6495) (#6498) (@houko)
- Approvals approve 415 false-success + status column in approvals list (#6492) (#6500) (@houko)
- Stop over-blocking MCP arguments that carry a long numeric id (#6499) (#6503) (@houko)
- Route post-approval reply through account-qualified outbound (#6492) (#6511) (@houko)
- Surface mid-stream provider errors instead of empty/garbled turns (#6512) (@houko)
- Release the token reservation on drop to stop a quota self-DoS (#6513) (@houko)
- Attribute owner-key spend and enforce per-user budget on authenticated API turns (#6514) (@houko)
- Honor response_format in the Gemini and Vertex AI drivers (#6515) (@houko)
- Treat [browser] config-reload as restart-required, not a false hot-reload (#6516) (@houko)
- Canonicalize provider before the per-user key lookup to stop an alias chargeback leak (#6517) (@houko)
- Serialize a canonical-session override on the per-agent lock to stop a lost-update race (#6518) (@houko)
- Scope MCP knowledge_add_* writes to the calling agent, not agent_id="" (#6519) (@houko)
- Recognize CHANGELOG attribution on a bullet's continuation lines (#6520) (@houko)
- Don't orphan another agent's relations when deleting a shared entity's first-writer (#6522) (@houko)
- Honor auto_approve and return 409 on double-resolve (#6492) (#6528) (@houko)
- Surface on-disk upload path to agents for every file type (#6531) (@neo-wanderer)

### Changed

- Normalize on-disk upload naming to <uuid>.<ext> (#6530) (#6536) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Documentation

- Explain trusted_senders vs [[users]] RBAC composition (#6492) (#6507) (@houko)
- Document per-user key precedence over operator rotation (#6460) (#6510) (@houko)

### Maintenance

- Bump the cargo-minor-patch group with 10 updates (#6452) (@app/dependabot)
- Bump tokio-tungstenite from 0.29.0 to 0.30.0 (#6453) (@app/dependabot)
- Update yanked spin 0.9.8 to 0.9.9 (#6454) (@houko)
- Bump the actions-minor-patch group with 3 updates (#6456) (@app/dependabot)
- Bump actions/setup-node from 6.4.0 to 7.0.0 (#6457) (@app/dependabot)
- Lock in env trust split across defer→approve→resume (follow-up to #6465) (#6467) (@houko)
- Bump the web-minor-patch group in /web with 5 updates (#6472) (@app/dependabot)
- Bump the dashboard-minor-patch group in /crates/librefang-api/dashboard with 6 updates (#6473) (@app/dependabot)
- Bump @eslint/js from 9.39.4 to 9.39.5 in /crates/librefang-api/dashboard (#6474) (@app/dependabot)
- Bump serde_with from 3.18.0 to 3.21.0 (#6475) (@app/dependabot)
- Bump the docs-minor-patch group in /docs with 4 updates (#6491) (@app/dependabot)
- Update model snapshot (#6523) (@houko)
- Bump wasmtime from 46.0.1 to 47.0.1 (#6527) (@app/dependabot)
- Bump the cargo-minor-patch group across 1 directory with 17 updates (#6533) (@app/dependabot)
- Migrate librefang-acp to agent-client-protocol 1.3.0 (supersedes #6526) (#6534) (@houko)

</details>


## [2026.7.10] - 2026-07-10

_40 PRs from 4 contributors since v2026.6.29._

### Added

- Surface the model the Codex CLI is configured to run (#6365) (@houko)
- Complete and proofread dashboard and website translations (#6376) (@houko)

### Fixed

- Clear cargo-deny advisory failures on main (#6366) (@houko)
- Keep [Unreleased] at the top; prune stale buried entries (#6367) (@houko)
- Clear quick-xml RUSTSEC-2026-0194/0195 advisories (#6387) (@houko)
- Convert Markdown to Slack mrkdwn in the Slack sidecar (#6397) (@neo-wanderer)
- Clear crossbeam-epoch RUSTSEC-2026-0204 advisory (#6400) (@houko)
- Never re-spill read_artifact results at the post-tool chokepoint (#6406) (@houko)
- Request extended thinking via reasoning_effort in the OpenAI-compat driver (#6407) (@houko)
- Add LIBREFANG_REGISTRY_OFFLINE to skip registry network refresh in tests (#6408) (@houko)
- Correlate chat turns with their WS terminal frames (#6419) (@houko)
- Single-source the invisible-char set; skip reply-precheck on captionless media (#6426) (@houko)
- Drop redundant refs in TUI format! args (Rust 1.97 clippy) (#6428) (@houko)
- Reliable CLI→PyPI publishing — stable-only gate + stop desktop deleting CLI assets (#6433) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Documentation

- Add Arch Linux pacman installation instructions (#6386) (@pavver)
- Only the WhatsApp sidecar reads DM/group policy env vars (#6405) (@houko)
- Install CLI from official homebrew-core (#6414) (@houko)
- Wrap homebrew-core note at sentence boundaries (#6415) (@houko)

### Maintenance

- Bump rmcp from 1.7.0 to 2.0.0 (#6364) (@app/dependabot)
- Adopt tera 2.0 (#6368) (@houko)
- Adopt aes-gcm 0.11 and the cargo-minor-patch bumps (#6369) (@houko)
- Gitignore the AUR CI deploy key pair (#6370) (@houko)
- Bump the actions-minor-patch group with 2 updates (#6373) (@app/dependabot)
- Bump tauri-apps/tauri-action from 0.6.2 to 1.0.0 (#6374) (@app/dependabot)
- Bump the web-minor-patch group in /web with 10 updates (#6377) (@app/dependabot)
- Bump the dashboard-minor-patch group in /crates/librefang-api/dashboard with 15 updates (#6378) (@app/dependabot)
- Bump the docs-minor-patch group in /docs with 10 updates (#6380) (@app/dependabot)
- Bump @types/node from 25.9.3 to 26.1.0 in /docs (#6381) (@app/dependabot)
- Raise job timeout to 120 minutes for cold builds (#6389) (@houko)
- Bump the cargo-minor-patch group with 5 updates (#6394) (@app/dependabot)
- Bump the actions-minor-patch group with 5 updates (#6401) (@app/dependabot)
- Make webhook-agent happy-path test hermetic (#6402) (@houko)
- Export LIBREFANG_REGISTRY_OFFLINE in the workspace test lanes (#6410) (@houko)
- Bump the web-minor-patch group in /web with 3 updates (#6411) (@app/dependabot)
- Bump typescript from 6.0.3 to 7.0.2 in /web (#6412) (@app/dependabot)
- Bump the dashboard-minor-patch group in /crates/librefang-api/dashboard with 6 updates (#6413) (@app/dependabot)
- Move stable CLI to homebrew-core and fix tap sync bugs (#6416) (@houko)
- Match Homebrew class_s casing for versioned tap formula (#6418) (@houko)
- Seed registry content from a pinned in-repo fixture instead of the network (#6421) (@houko)
- Bump the docs-minor-patch group in /docs with 4 updates (#6424) (@app/dependabot)

</details>


## [2026.6.29] - 2026-06-29

_14 PRs from 4 contributors since v2026.6.26-beta.24._

### Highlights

- **Korean language support** — full UI, CLI/TUI, and error message translations added (233 keys covered)
- **ARM64 Linux packages** — aarch64 binaries now published alongside x86_64 via AUR and the project's pacman repo
- **Telegram setup resilience** — the setup form stays available after a describe timeout instead of disappearing
- **Codex CLI flexibility** — Codex CLI can now be used outside of Git repositories
- **Mixed-media message enrichment** — coalesced batches with mixed content types are now correctly enriched on the debounced path

### Added

- UI Korean translation (#6349) (@seungjin)
- Complete Korean error translations (43 → 233 keys) (#6353) (@houko)
- Add Korean (ko) translation for the CLI/TUI (#6356) (@houko)
- Publish aarch64 packages alongside x86_64 (#6334) (#6358) (@houko)

### Fixed

- Bump pdf-extract 0.10→0.12 to patch lopdf RUSTSEC-2026-0187 (#6339) (@houko)
- Keep Telegram setup form available after describe timeout (#6345) (@pavver)
- Allow Codex CLI outside Git repositories (#6347) (@pavver)
- Enrich coalesced mixed-media batches on the debounced path (#6348) (#6351) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Maintenance

- Symlink legacy NDK binutils so vendored OpenSSL cross-compiles for Android (#6335) (@houko)
- Put NDK bin on PATH so openssl-src finds the legacy ranlib symlink (#6338) (@houko)
- Enable auto-merge instead of forcing --admin (#6340) (@houko)
- Publish AUR packages on release (#6334) (#6341) (@houko)
- Publish project-maintained pacman repo to R2 (#6334) (#6352) (@houko)

### Other

- Fix[flake.nix]: Add perl to nativeBuildInputs (#6346) (@FrantaNautilus)

</details>


## [2026.6.26] - 2026-06-26

_10 PRs from 2 contributors since v2026.6.24-beta.23._

### Added

- Add Ukrainian localization and extract hardcoded copy (#6312) (@pavver)
- Add AUR packaging for Arch Linux (#6314) (@pavver)
- Surface run params, errors, and one-click re-run (#6292) (#6324) (@houko)
- Allow a custom model id when editing an agent (#6318) (#6327) (@houko)

### Fixed

- Disable redirect following on OAuth HTTP clients (SSRF + credential leak) (#6315) (@houko)
- Block separator-less secret env names from WASM guests (#6316) (@houko)
- Guard gc_sweep running_tasks removal with task_id (TOCTOU) (#6317) (@houko)
- Describe inbound images on the debounced channel path (#6321) (#6323) (@houko)
- Accept empty-recipient HMAC so bootstrap_peers can connect (#6330) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Maintenance

- Pin claude_code resolved-model parsing (#6318) (#6331) (@houko)

</details>


## [2026.6.24] - 2026-06-24

_33 PRs from 5 contributors since v2026.6.22-beta.22._

### Added

- Localize TUI Onboarding Wizard and Agents screen (#6253) (@pavver)
- Pluggable context-rewrite modules — per-agent engine + host-run request_llm_summary (closes #6264) (#6287) (@houko)
- Add scriptable tool result transform hook (#6291) (@pavver)
- Per-step errors and re-run with same parameters (#6292) (#6302) (@houko)
- Code_search workspace regex search tool (#6295) (#6307) (@houko)

### Fixed

- Gate provider hourly token budget pre-call so exhaustion flags the fallback chain (#5980) (#5988) (@DaBlitzStein)
- Persist agent skill & MCP-server assignments to agent.toml (#6046) (@DaBlitzStein)
- Embed developer-loop placeholder in first result delivery (closes #6251) (#6254) (@maoxin1234)
- Kill sidecar child on shutdown + forward async delegation result to channel (#6267) (@DaBlitzStein)
- Complete dashboard i18n coverage (#6271) (@pavver)
- Vendor OpenSSL unconditionally so cross-compiled release targets link (#6279) (@houko)
- Merge updater plugin config into base tauri.conf.json (closes #6270) (#6283) (@houko)
- Handle /new in the TUI chat surfaces (closes #6265) (#6284) (@houko)
- Merge the dual [Unreleased] sections into one at the top (#6285) (@houko)
- Forward web-UI-initiated delegation results to the home channel (refs #6266) (#6286) (@houko)
- AUTH PLAIN fallback for SMTP + expose input/error in workflow runs list (#6293) (@DaBlitzStein)
- Scope GET /api/workflows/{id}/runs to the path workflow (#6298) (@houko)
- Block sandbox escape via intermediate-ancestor symlink on writes to non-existent dirs (#6299) (@houko)
- Reject newline/CR/NUL in secret key to prevent secrets.env line injection (#6300) (@houko)
- Graceful 400 for non-table [memory]/[proactive_memory] in PATCH /api/memory/config (#6301) (@houko)
- Apply api_key_env/base_url in PATCH /api/agents/{id}/config instead of dropping them (#6303) (@houko)
- Wire provider cooldown breaker into the LLM retry path and fix its probe gate (#6305) (@houko)
- Gate scriptable transform-hook tests behind cfg(unix) to unbreak Windows CI (#6306) (@houko)
- Gate use std::sync::Arc behind cfg(unix) to complete the Windows-red fix (#6308) (@houko)
- Install libdbus-1-dev in the release Bump Version job (#6309) (@houko)

### Performance

- JSON-aware token estimation for tool paths (#6281) (@maoxin1234)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Maintenance

- Token-estimation accuracy benchmark with multi-tokenizer baselines (#6269) (@maoxin1234)
- Bump the cargo-minor-patch group across 1 directory with 8 updates (#6276) (@app/dependabot)
- Bump wasmtime from 45.0.2 to 46.0.0 (#6277) (@app/dependabot)
- Bump cron from 0.16.0 to 0.17.0 (#6278) (@app/dependabot)
- Bump vulnerable/yanked lockfile deps to clear global CI failures (#6280) (@houko)
- Bump actions/checkout from 6 to 7 (#6296) (@app/dependabot)
- Bump actions/cache from 5.0.5 to 6.0.0 (#6297) (@app/dependabot)

</details>


## [2026.6.22] - 2026-06-22

_1 PR from 1 contributor since v2026.6.22-beta.21._

### Highlights

- **Safer upgrades** — the installer now falls back to the last known-good release and automatically rolls back a failed upgrade instead of leaving the app in a broken state.

### Fixed

- Fall back to installable release, roll back bad upgrades (#6272) (@houko)


## [2026.6.17] - 2026-06-17

_22 PRs from 3 contributors since v2026.6.16-beta.19._

### Added

- Per-conversation agent routing for multi-agent groups (#5323) (#6127) (@houko)
- Passkey (WebAuthn/FIDO2) dashboard login (#5981) (#6129) (@houko)
- Deterministic inbound dispatch — channel-instance binding lookup (#5671 Model A) (#6131) (@houko)
- GitHub/Codeberg registry source selector (#6142) (@houko)
- Gate auto-routing on AutoRouteStrategy, not the "assistant" name (#6139) (#6148) (@houko)
- Propagate W3C traceparent on outbound MCP tool calls (#6128) (#6153) (@houko)
- Report the model codex actually used (#6134) (#6157) (@houko)
- Dock the agent panel as a resizable sidebar with a larger prompt editor (#6154 #6155) (#6164) (@houko)
- The cron-management tool disables jobs instead of deleting them (#6159) (#6165) (@houko)
- Enlarge TOML view, edit agent system prompt and tools with reset-to-default (#6150 #6151 #6152) (#6166) (@houko)
- Central prompt repository page with versions and agent binding (#6160) (#6167) (@houko)

### Fixed

- Enforce cross-chat dispatch guard through the /mcp bridge (#6117) (#6125) (@houko)
- Take over a stale conversation-ownership claim from a channel-ineligible holder (#5323) (#6132) (@houko)
- Respect `LIBREFANG_HOME` when resolving plugin directory (#6136) (@HuaGu-Dragon)
- Close channel media RBAC bypass and audit findings (#6141) (@houko)
- Keep Save actionable after a passing Test (#6144) (#6146) (@houko)
- Refetch hand settings after save so inputs persist (#6145) (#6147) (@houko)
- Show the correct Hand agent name in the sessions view (#6156) (#6162) (@houko)
- Build vendored OpenSSL on Windows so webauthn-rs links (#6161) (#6163) (@houko)
- Pin vendored OpenSSL to Strawberry Perl on the Windows test lane (#6171) (@houko)

### Changed

- Lift tool dispatch table to typed ToolError (#3576 slice 5) (#6124) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Maintenance

- Bump the actions-minor-patch group with 2 updates (#6140) (@app/dependabot)

</details>


## [2026.6.16] - 2026-06-16

_18 PRs from 3 contributors since v2026.6.11-beta.18._

### Highlights

- **External Skill Registry** — agents can now discover and consume skills hosted on a Codeberg registry, with diff and propose-to-registry support for pending evolution drafts
- **Persistent MCP Server Config** — MCP server configurations are stored in SQLite and survive restarts; runtime writes to `/api/mcp/servers` are also persisted
- **Ukrainian Language Support** — backend and web UI are now fully localized in Ukrainian
- **DeepSeek V4 Pro Reasoning** — DeepSeek v4-pro is now treated as a thinking-with-tools model so `reasoning_content` is correctly echoed through
- **WhatsApp Voice Notes & Matrix Memory** — ElevenLabs voice notes send as Ogg/Opus with proper MIME sniffing; Matrix peers with colons in their IDs can now use the Memory tool

### Added

- Consume a Codeberg-hosted skill registry via registry.registry_host (#6095) (#6103) (@houko)
- Diff + propose-to-registry for pending evolution drafts (#5819) (#6104) (@houko)
- SidecarChannelConfig.agent + available_agents (#5671 PR-A) (#6105) (@houko)
- SQLite-backed MCP server config storage + boot merge (#6021) (#6106) (@houko)
- Add Ukrainian language support for backend and web UI (#6109) (@pavver)
- Persist /api/mcp/servers writes to a DB store via mcp_runtime_store (#6113) (#6115) (@houko)

### Fixed

- Accept `version` field in ClawHubInstallRequest (#6038) (#6039) (@DaBlitzStein)
- Stage Skills-tab edits behind a Save button (#6042) (@DaBlitzStein)
- Refresh detect-secrets baseline for migrated Cloudflare account_id (#6093) (@houko)
- Treat deepseek-v4-pro as thinking-with-tools so reasoning_content is echoed (#6098) (@DaBlitzStein)
- Preserve caller-supplied channel name case in channel_send (#6078) (#6101) (@houko)
- Percent-encode colons in peer_id so Matrix peers can use Memory (#6100) (#6102) (@houko)
- Pin brace-expansion override to 2.0.2 to unbreak the Cloudflare docs build (#6110) (@houko)
- Send ElevenLabs voice notes as Ogg/Opus and sniff audio mime (#6116) (#6118) (@houko)

### Changed

- Migrate web_search.rs to ToolError (#3576 slice) (#6107) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Maintenance

- Migrate worker config to librefang Cloudflare account (#6092) (@houko)
- Scope frontend pnpm audit to production deps (#6108) (@houko)
- Free runner disk space before the integration shard build (fixes ENOSPC on main) (#6112) (@houko)

</details>


## [2026.6.11] - 2026-06-11

_8 PRs from 2 contributors since v2026.6.10-beta.17._

### Added

- **mcp/api: `mcp_runtime_store = "db"` persists `/api/mcp/servers` writes to SQLite instead of `config.toml`, so MCP servers can be managed at runtime when the config file is read-only** (#6113) (@houko).
  #6106 added the DB-backed `mcp_server_configs` store and a boot-time merge, but the API write-path (`POST` / `PUT` / `DELETE /api/mcp/servers`, the taint patch) and the read-path still only saw `config.toml`, so a DB-managed server was invisible to the API and could not be added at all when `config.toml` was a read-only Kubernetes ConfigMap (the #6021 motivation).
  The new `config.toml: mcp_runtime_store` knob (default `file`, byte-for-byte the prior behaviour) routes writes to the store when set to `db`.
  The boot overlay and `reload_mcp_servers` now share one `McpConfigStore::merge_over` helper — previously the hot-reload path dropped DB-backed servers the boot merge had applied — and the handlers read the effective (file + DB) set, so DB-backed servers are listed, fetched, updated, and deleted like file-backed ones and take effect without a restart.
  Tests: `mcp_config_store::tests::merge_over_*` and the `mcp_runtime_store_db_test` API integration suite.

### Fixed

- **llm-drivers(deepseek): recognise `deepseek-v4-pro` as a thinking-with-tools model so its `reasoning_content` is echoed back** (@DaBlitzStein).
  `deepseek-v4-pro` was excluded from `is_deepseek_v4_thinking_with_tools` on the #4842 assumption that it "works out-of-the-box", but production multi-turn tool-call conversations on it return `400 "The reasoning_content in the thinking mode must be passed back to the API."` — the same echo requirement as V4 Flash.
  A delegated agent running `deepseek-v4-pro` failed every turn once its history contained a tool-call thinking turn, so `agent_send` / shared-queue tasks to it never executed; a sibling agent on the same model only avoided it by never trimming its history.
  The model is now matched (Flash + Pro) so the `Echo` reasoning-echo policy applies and the thinking text is round-tripped intact. Regression in `test_is_deepseek_v4_thinking_with_tools_matches_v4_flash`.
- Persist run state outside the state lock so GET /run never spuriously reports running:false (#6083) (@houko)
- Inject embedded SDK into the sidecar --describe probe so the configure form isn't empty without pip install (#6085) (@houko)
- Encode qrcode_img_content so the login QR is scannable (#6086) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Maintenance

- Bump @whiskeysockets/baileys from 6.7.21 to 6.7.22 in /packages/whatsapp-gateway (#6077) (@app/dependabot)
- Bump @types/react from 19.2.16 to 19.2.17 in /web in the web-minor-patch group (#6079) (@app/dependabot)
- Bump the dashboard-minor-patch group in /crates/librefang-api/dashboard with 3 updates (#6080) (@app/dependabot)
- Free runner disk space before nix build (#6082) (@houko)
- Free runner disk space before the unit-test build (fixes ENOSPC on main) (#6089) (@houko)

</details>


## [2026.6.10] - 2026-06-10

_78 PRs from 6 contributors since v2026.5.31-beta.16._

### Highlights

- **Parallel tool-call dispatch** — agents can now execute multiple tools concurrently (opt-in via config flag), reducing round-trip latency for multi-tool turns.
- **Remote Hand marketplace installs** — Hands can be installed directly from the remote marketplace without manual packaging.
- **Skill evolution approval gate** — `auto_evolve` updates now flow through an approval step, and a new `evolution_mode` gives you control over how skills self-improve.
- **Shell execution trusted-binary shortcut** — opt into `safe_bins_skip_approval` to skip approval prompts for a strict allowlisted set of shell commands.
- **Security hardening across the board** — fixes for SSRF allowlist gaps (IMDS/CGNAT addresses), TOML/query-string injection in agent manifests, OOM vectors in streamed tool calls and sidecar stderr, DNS-rebinding in WASM `net_fetch`, supply-chain audit bypass in zip installs, and a pre-handshake memory-exhaustion DoS; plus credential-redaction and vault KDF correctness fixes.

### Added

- Externalize template routing rules to an overridable TOML (#5946) (@houko)
- Persist goal runs and recover stale runs at boot (#5947) (@houko)
- Activate parallel tool-call dispatch behind config flag (#5948) (@houko)
- Wire RL rollout export producer into AgentLoopEnd hook (#5950) (@houko)
- Execute WASM hooks in the sandbox as pure-compute (#5951) (@houko)
- Remote marketplace install for Hands (#5954) (@houko)
- Opt-in safe_bins_skip_approval for shell_exec (#6000) (@houko)
- Creator_match filter for TaskClaimed / TaskCompleted triggers (#5960) (#6001) (@houko)
- Skill evolution_mode + gate auto_evolve updates through approval (#5844, #5819) (#6003) (@houko)
- Emit cron-fire and auto-disable observability metrics (#6029) (@neo-wanderer)

### Fixed

- Gate skill_evolve_* tools on auto_evolve + skill_workshop flags (#5678) (@DaBlitzStein)
- Correct stale openapi.sha256 baseline to repair main red (#5945) (#5953) (@houko)
- Stop Cargo.lock changes from busting the rust-cache (cold compile) (#5958) (@houko)
- Pre-flight hand role spawns before reactivation teardown (#5959) (@houko)
- Cron day-of-week follows POSIX convention (0 and 7 = Sunday) (#5967) (@DaBlitzStein)
- Atomic compare-and-swap in task_claim to prevent double-claim (#5961) (#5968) (@houko)
- Ship MCP caller context via _meta instead of arguments (#5965) (#5969) (@houko)
- Retry past lost CAS race in task_claim + post-review nits (#5961, #5965) (#5973) (@houko)
- Memory/wiki ACL denials degrade gracefully instead of killing the turn (#5984) (@houko)
- Trigger evaluator self-deadlocks when per-event budget is exhausted (#5977) (#5987) (@DaBlitzStein)
- History fold preserves tool-result content on omit AND parse failure (#5978) (#5991) (@DaBlitzStein)
- Loop-guard block is soft, and a persistent block stall degrades to a real reply (#5979) (#5992) (@DaBlitzStein)
- Propagate per-sidecar account_id for multi-bot isolation (#5955) (#5996) (@houko)
- Make safe_bins_skip_approval a strict subset of the allowlist gate (#6004) (@houko)
- Tolerate <think> preamble in history_fold summary parsing (#6009) (#6011) (@houko)
- Redact images for text-only models via catalog supports_vision (#6010) (#6013) (@houko)
- Assign approved workshop skill to the creating agent (#5989) (#6014) (@houko)
- Cron enable/disable now PUTs with an {enabled} body instead of POSTing a PUT-only route (#6018) (@neo-wanderer)
- Resolve channel_send mirror owner via bindings, not just default_agent (#6023) (@neo-wanderer)
- Daemon_json surfaces error-less 4xx instead of silent success (#6019) (#6024) (@houko)
- Stabilize non-headless Chrome startup under env isolation (#6028) (@app/copilot-swe-agent)
- Explain empty sidecar form + warn on legacy [channels.*] config (#6030) (@houko)
- Chrono_lite_date() returns wrong dates for most of the year (#6048) (@houko)
- Quota/budget time windows compare RFC3339 text lexicographically, ignoring time-of-day (#6049) (@houko)
- Unbounded Vec growth from attacker-controlled streamed tool-call index (OOM) (#6050) (@houko)
- Self-referential $ref in a tool schema overflows the stack (DoS from untrusted MCP/skill schemas) (#6051) (@houko)
- Redact_secrets leaks a real token that follows a short match (#6052) (@houko)
- SSRF allowlist omits 0.0.0.0, CGNAT/Alibaba IMDS, 192.0.0.192, and AWS IMDS hostnames (#6053) (@houko)
- Single-quote dotenv value panics credential resolution (#6054) (@houko)
- WASM net_fetch follows redirects without per-hop SSRF re-validation (DNS-rebinding); misses Azure IMDS (#6055) (@houko)
- TOML injection via unescaped system_prompt / name / tags in generated agent manifests (#6056) (@houko)
- Unauthenticated pre-handshake read can pin a 16 MiB buffer (memory-exhaustion DoS) (#6057) (@houko)
- Non-ASCII snippet offset misalignment; body cap not enforced on rendered bytes (#6058) (@houko)
- Query-string injection via unescaped MiniMax task_id/file_id (#6059) (@houko)
- Apply_patch files_moved counter incremented before the move write succeeds (#6060) (@houko)
- Vault staging-file race across processes; OAuth deny hangs 5 minutes (#6061) (@houko)
- Trim/prune drop in-memory entries even when the SQLite DELETE fails (#6062) (@houko)
- Exec timeout leaks docker process; bind-mount validation never runs (#6063) (@houko)
- Taint_scanning=false silently disables documented always-on credential key-name blocking (#6064) (@houko)
- Auto-update script TOCTOU/symlink exec; skill-install path traversal (#6065) (@houko)
- ClawHub/Skillhub zip install bypasses the supply-chain audit (.pth RCE) (#6066) (@houko)
- Permission bridge serializes all sessions, dropping approval events on broadcast lag (#6067) (@houko)
- Channel error truncation panics on multi-byte UTF-8 boundary (#6068) (@houko)
- Sidecar stderr read is unbounded — same OOM vector already capped for stdout (#6069) (@houko)
- Describe_event panics on multi-byte Custom payload; correct false test-env safety claim (#6070) (@houko)
- Vault KDF uses volatile Argon2::default() while on-disk format stores no params (#6071) (@houko)
- Allow unused_mut on chromium launch args off-Linux (#6072) (@houko)

### Changed

- Split role-trait god-file into per-domain modules (#5970) (@houko)
- Split the 14.6k-line main.rs into per-command modules (#5971) (@houko)
- Derive task_claim retry budget from pool size (#5974) (@houko)
- Split routes/agents.rs into per-concern modules (#5975) (@houko)
- Split routes/workflows.rs into per-concern modules (#5985) (@houko)
- Split routes/skills.rs into per-concern modules (#5986) (@houko)
- Split routes/config.rs into per-concern modules (#5993) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Documentation

- Guard against editing a re-created worktree on a stale base (#6002) (@houko)

### Maintenance

- Populate sessions.peer_id on save (#5286) (@f-liva)
- Make required-status-checks enforceable — CI Gate, aarch64 lane, openapi-drift fix (#5943) (@houko)
- Merge_group support (prereq for merge queue) [stacked on #5943] (#5944) (@houko)
- Extract heartbeat de-dup transition into a testable helper (#5949) (@houko)
- Faster + reliable docker dev iteration — mold linker + per-worktree target (#5952) (@houko)
- Auto-commit regenerated codegen on same-repo PRs (#5994) (@houko)
- Ignore skill scaffolder template TODOs (#5982, #5983) (#5995) (@houko)
- Bump the cargo-minor-patch group with 11 updates (#6006) (@app/dependabot)
- Bump the web-minor-patch group in /web with 9 updates (#6007) (@app/dependabot)
- Bump the dashboard-minor-patch group in /crates/librefang-api/dashboard with 12 updates (#6008) (@app/dependabot)
- Ignore .github self-scan that spawns false-positive issues (#6012) (@houko)
- Bump the docs-minor-patch group in /docs with 6 updates (#6015) (@app/dependabot)
- Bump next from 15.5.18 to 16.2.7 in /docs (#6016) (@app/dependabot)

</details>


## [2026.5.31] - 2026-05-31

_16 PRs from 2 contributors since v2026.5.30-beta.15._

### Added

- Inline skill assignment on the agent Skills tab (#4917) (#5930) (@houko)
- Port command-policy and message coalescing to sidecar channels (#5931) (@houko)
- Propose evolved skill as PR to registry (#5932) (@houko)
- Ship librefang-sidecar-telegram binary in release tarballs (#5937) (@houko)

### Fixed

- Tool_runner shell — timeout clamp, streaming output, process group kill, Windows compat (#5763) (@leszek3737)
- Tool_runner knowledge — confidence clamp, input validation, result limits, property bounds (#5767) (@leszek3737)
- Tool_runner image — extension whitelist, 50MB limit, BMP i32, JPEG markers, PNG sig (#5768) (@leszek3737)
- Enable agent model Save on any field change (#5917) (#5925) (@houko)
- Empty mcp_servers = [] grants no MCP tools, not all (#5855) (#5928) (@houko)
- Move getpgrp to the x86_64-only seccomp block to unbreak aarch64 (#5929) (@houko)
- Patch rand (0.8.6/0.9.3) and link-preview-js (4.0.1) security advisories (#5934) (@houko)
- Migrate ssh-backend to russh 0.61.1 (clears 5 RustSec advisories) (#5935) (@houko)

### Changed

- Migrate read_artifact to ToolError (error-contracts slice 2) (#5926) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Maintenance

- Regression test for #5857 Windows provider-key path validation (#5927) (@houko)
- Skip deleted (410 Gone) issues in auto-close reconciler (#5933) (@houko)
- Rustfmt knowledge.rs to unbreak main Quality (post #5767) (#5938) (@houko)

</details>


## [2026.5.30] - 2026-05-30

_68 PRs from 5 contributors since v2026.5.28-beta.14._

### Added

- Add source attribution to GET /api/tools response (#5679) (@DaBlitzStein)
- Tools tab in agent detail with grouped view (closes #5677) (#5680) (@DaBlitzStein)
- Expose auto_evolve toggle in Skills tab (#5741) (@DaBlitzStein)
- Kanban task board page (#5745) (#5805) (@houko)
- Support custom-URL self-hosted STT/TTS providers (fixes #5740) (#5814) (@houko)
- Rust Telegram sidecar adapter (parity with Python) (#5831) (@houko)
- Just dev --docker + TELEGRAM_LOG tracing (#5833) (@houko)
- Run WASM skill runtime via the runtime WasmSandbox (#5835) (@houko)
- Autonomous long-horizon goal runner (#5840) (@houko)
- Out-of-process `engine = "sidecar"` (#5849) (@houko)
- Scan tool-result content for indirect prompt injection (#5859) (@houko)

### Fixed

- Strip ANTHROPIC_API_KEY when OAuth credentials present (#5292) (@f-liva)
- Reconcile cascade-leak THEMATIC_HEADERS with post-#5053 prompt builder (#5351) (@f-liva)
- Tool_runner sandbox — RAII cleanup, TOCTOU removed, container_id redacted (#5757) (@leszek3737)
- Tool_runner workflow — artifact type check, deterministic sort, recursion limit (#5758) (@leszek3737)
- Tool_runner schedule — AM/PM parsing, minute precision, owner verification, cron validation (#5759) (@leszek3737)
- Tool_runner system — URL const, client reuse, error diagnostics (#5760) (@leszek3737)
- Tool_runner media — size limits, async fs, UUID filenames, ffmpeg deadlock, extension allowlist (#5761) (@leszek3737)
- Tool_runner web_legacy — SSRF protection, streaming body limit, unified UA, status check (#5764) (@leszek3737)
- Tool_runner canvas — XSS escape, whitelist parser, data: URI block, size limit (#5766) (@leszek3737)
- Tool_runner memory — truncation, pagination, key validation (#5770) (@leszek3737)
- Tool_runner agent — taint all inputs, narrow capabilities, deny None, network strict (#5775) (@leszek3737)
- Tool_runner process — output cap, strict caller_id, arg logging, serde_json (#5778) (@leszek3737)
- Tool_runner fs — backslash rejection, canonicalize, TOCTOU fix, read limit, dir pagination, atomic write (#5783) (@leszek3737)
- Route auto_evolve creates through skill_workshop pending queue (#5800) (@DaBlitzStein)
- Reset taint editor state when server prop changes (#5803) (@houko)
- Use catalog api_key_env for custom provider key resolution (#5807) (@houko)
- Regenerate stale openapi schema baseline to repair main red (#5834) (@houko)
- Make DAG-path step timeout error actionable (#5836) (@houko)
- Finish Option::zip migration in kernel tests (clippy 1.96.0) (#5837) (@houko)
- Keep custom providers across restarts, tolerate unknown tier (#5838) (@houko)
- Audit sweep — 5 CRITICAL + 7 HIGH (split-brain, RBAC, decay, dedup, prompt budget, async consolidate) (#5839) (@houko)
- Apply search filter to FangHub skills grid (#5843) (@DaBlitzStein)
- Use Option::zip for hand timestamp pairing (clippy) (#5845) (@houko)
- Close goal-run self-cleanup race + termination test coverage (follow-up #5840) (#5848) (@houko)
- MEDIUM follow-ups — counter map sweep, hot-reload on PATCH, multi-keyword search, configurable UPDATE thresholds (#5850) (@houko)
- Make extra_params / extra_body BTreeMap for deterministic wire-body key order (#5860) (@houko)
- Close trusted_senders all-or-nothing approval bypass for high-risk tools (#5861) (@houko)
- Make subprocess plugin sandbox secure-by-default (#2) (#5862) (@houko)
- Scrub internal errors from 5xx responses to prevent detail leakage (#5863) (@houko)
- Validate hand id as a safe path component to block traversal (#5865) (@houko)
- Apply config hot-reload for read-live fields, not only hot actions (#5867) (@houko)
- Reserve the global USD budget on the streaming dispatch path (#5869) (@houko)
- Bound consolidation candidate load with a per-agent LIMIT (#5871) (@houko)
- Stop logging API key, account cache tokens, keep stream tool ids (#5875) (@houko)
- Cover all per-agent override keys with a drift-guarded detector (#6) (#5876) (@houko)
- Guard agent_msg_locks GC with Arc::strong_count (symmetry with session_msg_locks) (#5877) (@houko)
- Account prompt-cache tokens in usage normalization (#5879) (@houko)
- Handle no-arg tool calls and UTF-8-safe thinking summary (#5882) (@houko)
- Route attachment download through the redirect-revalidating client (#5884) (@houko)
- Pin every redirect hop in web_fetch to close DNS-rebinding window (#5886) (@houko)
- Clean up per-flow OAuth vault entries on all callback exits (#5895) (@houko)
- Scan prompt context for injection at the load/reload boundary (#5897) (@houko)
- Retry transport-layer errors and make retry count configurable (#10) (#5898) (@houko)
- Detect re-entrant keyed agent_send to prevent session-lock deadlock (#5900) (@houko)
- Delimit all fields in the Merkle entry hash to close ambiguity (#5903) (@houko)
- Enforce RBAC on session auth path; offload workflow template write (#5906) (@houko)
- Low-severity correctness — workshop cap race, token saturating, ephemeral comment (#5910) (@houko)
- Keep anthropic stream block alignment; report effective claude_code timeout (#5913) (@houko)
- Gate media link URLs through safeUrl; share urlTransform with streaming view (#5916) (@houko)
- Allowlist glibc-startup syscalls for exec'd plugin binaries (fixes native_runtime_timeout CI failure) (#5920) (@houko)

### Changed

- Unify the three sidecar bridges onto a shared transport crate (#5852) (@houko)

### Performance

- Offload blocking filesystem/zip IO off the tokio runtime (#5892) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Documentation

- Fix three agent-facing architecture drift points (#5901) (@houko)

### Maintenance

- Bump the docs-minor-patch group in /docs with 2 updates (#5847) (@app/dependabot)
- Cargo fmt recently-merged code (repair main Quality fmt) (#5853) (@houko)
- Fix Windows-only red in shell capability test (path-not-found wording) (#5854) (@houko)
- Raise Test / Windows shard timeout 45 → 60 min to match macOS (#5856) (@houko)

</details>

## [2026.5.28] - 2026-05-28

_46 PRs from 5 contributors since v2026.5.25-beta.13._

### Breaking Changes

- Rust sidecar adapter SDK + AI-codegen-era rationale rewrite (#5821) (@houko)

### Added

- Per-agent channel allowlist (#5738) (@DaBlitzStein)
- Implement describe_image() and wire ImageFile description through channel adapters (#5815) (@houko)
- Rust sidecar adapter SDK + AI-codegen-era rationale rewrite (#5821) (@houko)

### Fixed

- Isolate attachment pre-inject per chat session — close cross-chat image leak (#5334) (@f-liva)
- Make migrate path containment existence-independent (fixes #5716) (#5719) (@houko)
- Repair discussion-to-issue backfill — gh api --jq doesn't take --arg (#5754) (@houko)
- Tool_runner taint — unified SECRET_KEYS, substring match, header trim, single-pass normalization (#5762) (@leszek3737)
- Tool_runner shell_safety — command injection hardening, quote-aware tokenizer (#5765) (@leszek3737)
- Tool_runner definitions — ALWAYS_NATIVE complete, OnceLock caches, schema fixes, tool_name constants (#5771) (@leszek3737)
- Tool_runner error — Upstream message preserved, MissingParameter String, ResourceNotFound 404 (#5772) (@leszek3737)
- Tool_runner cron — sender_id override, TOCTOU reduction, HashSet lookup, empty job_id rejected (#5773) (@leszek3737)
- Tool_runner dispatch — mutex split, fallback ACL, ACP args, spill wiring, snapshot ordering (#5774) (@leszek3737)
- Tool_runner spill — config-based threshold, validation, fast-path (#5776) (@leszek3737)
- Tool_runner wiki — limit cap, input validation, safe usize, caller_agent_id required (#5777) (@leszek3737)
- Tool_runner meta — case-insensitive lookup, Cow optimization, deterministic sort (#5779) (@leszek3737)
- Tool_runner task — typed deserialization, contextual errors, empty validation, status default (#5780) (@leszek3737)
- Tool_runner notify — length limit, control char sanitization, PII removal (#5782) (@leszek3737)
- Tool_runner hand — deterministic sort, empty id reject, config whitelist, output sanitization (#5784) (@leszek3737)
- Tool_runner goal — progress type fix, range validation (#5785) (@leszek3737)
- Tool_runner event — event_type validation, caller identity, reserved prefix guard (#5786) (@leszek3737)
- Tool_runner a2a — session_id taint, SSRF diagnostics, zero-alloc agent check (#5787) (@leszek3737)
- Tool_runner artifact — spawn_blocking, explicit errors, usize safe, zero-length reject (#5788) (@leszek3737)
- Tool_runner channel — poll u8 safe, file size limit, email regex, mirror dedup, thread_id routing (#5789) (@leszek3737)
- Skip bridge-side formatting for sidecar adapters (fixes #5795) (#5796) (@DaBlitzStein)
- Return forward-slash relative path from registry/content on Windows (#5801) (@houko)
- Make step timeout errors actionable with remediation guidance (#5806) (@houko)
- Eliminate Instant subtraction that panics on Windows CI (fixes #5726) (#5808) (@houko)
- Seed Feishu/Lark configure form when Python SDK is absent (#5809) (@houko)
- Unbreak coverage build — thread session_id into two SessionWriter test stubs (#5816) (@houko)
- Wiki.rs lifetime + shell.rs test arity after #5774/#5777 (#5818) (@houko)
- Unbreak main — agent channels in ApiDoc + fmt + secrets baseline (#5820) (@houko)
- Install gh CLI for release flow (#5826) (@houko)
- Run `gh auth setup-git` to unblock git push from container (#5827) (@houko)
- Override host-absolute credential helper path inside container (#5829) (@houko)

### Changed

- Migrate tool_runner tools to ToolError (#3576) (#5737) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Maintenance

- Bump the cargo-minor-patch group with 4 updates (#5748) (@app/dependabot)
- Bump wasmtime from 44.0.1 to 45.0.0 (#5749) (@app/dependabot)
- Bump sysinfo from 0.38.4 to 0.39.2 (#5750) (@app/dependabot)
- Bump which from 7.0.3 to 8.0.2 (#5751) (@app/dependabot)
- Bump tikv-jemallocator from 0.6.1 to 0.7.0 (#5752) (@app/dependabot)
- Bump the actions-minor-patch group with 4 updates (#5790) (@app/dependabot)
- Bump actions/setup-python from 5 to 6 (#5791) (@app/dependabot)
- Bump the web-minor-patch group in /web with 3 updates (#5810) (@app/dependabot)
- Bump the dashboard-minor-patch group in /crates/librefang-api/dashboard with 7 updates (#5811) (@app/dependabot)
- Bump globals from 15.15.0 to 17.6.0 in /crates/librefang-api/dashboard (#5812) (@app/dependabot)
- Docker fallback for `just release` when cargo is missing (#5825) (@houko)

</details>


## [2026.5.25] - 2026-05-25

_308 PRs from 7 contributors since v2026.5.17-beta.12._

### Breaking Changes

- Migrate ntfy from in-process adapter to sidecar (P7) (#5224) (@houko)
- Remove in-process telegram adapter (now sidecar-only) (#5241) (@houko)
- Migrate gotify from in-process adapter to sidecar (#5263) (@houko)
- Migrate mastodon from in-process adapter to sidecar (#5264) (@houko)
- Remove 6 low-value channel adapters (#5265) (@houko)
- Drop 12 unmaintained adapters (#5267) (@houko)
- Migrate bluesky from in-process adapter to sidecar (#5277) (@houko)
- Migrate reddit from in-process adapter to sidecar (#5281) (@houko)
- Migrate twitch from in-process adapter to sidecar (#5297) (@houko)
- Migrate rocketchat from in-process adapter to sidecar (#5298) (@houko)
- Migrate discord from in-process adapter to sidecar (#5299) (@houko)
- Migrate nextcloud from in-process adapter to sidecar (#5301) (@houko)
- Migrate slack from in-process adapter to sidecar (#5302) (@houko)
- Migrate webex from in-process adapter to sidecar (#5309) (@houko)
- Migrate zulip from in-process adapter to sidecar (#5310) (@houko)
- Migrate line from in-process adapter to sidecar (#5312) (@houko)
- Migrate mattermost from in-process adapter to sidecar (#5315) (@houko)
- Migrate signal from in-process adapter to sidecar (#5317) (@houko)
- Migrate qq from in-process adapter to sidecar (#5325) (@houko)
- Migrate matrix from in-process adapter to sidecar (#5368) (@houko)
- Migrate feishu from in-process adapter to sidecar (#5380) (@houko)
- Migrate wecom from in-process adapter to sidecar (WebSocket-only) (#5392) (@houko)
- Migrate email from in-process adapter to sidecar (#5408) (@houko)
- Migrate dingtalk from in-process adapter to sidecar (Stream mode only) (#5417) (@houko)
- Migrate wechat from in-process adapter to sidecar (#5421) (@houko)
- Migrate teams from in-process adapter to sidecar (#5433) (@houko)
- Migrate whatsapp from in-process adapter to sidecar (dual-mode) (#5445) (@houko)
- Migrate webhook from in-process adapter to sidecar (#5455) (@houko)
- Migrate google_chat from in-process adapter to sidecar (#5459) (@houko)
- Delete dead per-channel REST endpoints + their helpers (#5463) (@houko)

### Highlights

- **Channel adapter sidecar migration** — all 27 messaging integrations (Slack, Discord, Telegram, WhatsApp, Signal, Teams, and more) are now isolated sidecar processes instead of in-process adapters; 18 unmaintained adapters were removed. Sidecar adapters can be configured directly from the dashboard.
- **Human-in-the-loop (HITL) approval step** — agents can now pause and request operator approval mid-run; approvals route back to the originating chat with inline keyboard buttons on supported adapters, and the same tool only prompts once per session.
- **Credential pools** — configure multiple API keys per LLM provider for automatic round-robin rotation and instant failover on rate limits.
- **Schedule tab & budget visibility** — the dashboard now has an editable Schedule tab for managing triggers, cron jobs, and continuous mode; a new per-provider budget caps surface shows spend and limits per provider.
- **Security hardening** — session tokens are now hashed at rest, SSRF validation added to URL inputs, path-traversal guards tightened across asset and file routes, SQL bindings replace string concatenation in session cleanup, and request bodies are size-capped against pre-allocation DoS.

### Added

- Credential pools — multi-key rotation per provider with… (#5063) (@Chukwuebuka-2003)
- Add per-agent memory isolation via agent_id parameter (#5071) (@leszek3737)
- Propagate W3C traceparent to outbound LLM HTTP requests (#5190) (@neo-wanderer)
- Implement HITL operator-step — notify dispatch, timeout watchdog, HTTP actions→resume (#5133, #5134, #5135) (#5191) (@houko)
- Caller-controlled conversation_key for agent_send (#5212) (@houko)
- Forced /compact with async spawn, ack+event, summary banner (#5213) (@houko)
- Sidecar channel parity — protocol, supervision, config (P0–P3) (#5219) (@houko)
- Python sidecar channel adapter framework (P4) (#5220) (@houko)
- Hard-block new in-process channel adapters (P5) (#5221) (@houko)
- Migrate ntfy from in-process adapter to sidecar (P7) (#5224) (@houko)
- Compute wasMentioned from group_trigger_patterns when mentionedJids is empty (#5230) (@f-liva)
- Telegram full sidecar parity (formatter + full inbound/outbound), stdlib-only (#5232) (@houko)
- Remove in-process telegram adapter (now sidecar-only) (#5241) (@houko)
- Configure sidecar adapters (telegram/ntfy) from dashboard (#5252) (@houko)
- Editable Schedule tab — triggers, cron, continuous mode (#4924) (#5256) (@houko)
- HITL operator-step dashboard surfaces (#4977) (#5257) (@houko)
- Credential pools for multi-key per-provider rotation (#4965) (#5260) (@houko)
- Migrate gotify from in-process adapter to sidecar (#5263) (@houko)
- Migrate mastodon from in-process adapter to sidecar (#5264) (@houko)
- Migrate bluesky from in-process adapter to sidecar (#5277) (@houko)
- Migrate reddit from in-process adapter to sidecar (#5281) (@houko)
- Migrate twitch from in-process adapter to sidecar (#5297) (@houko)
- Migrate rocketchat from in-process adapter to sidecar (#5298) (@houko)
- Migrate discord from in-process adapter to sidecar (#5299) (@houko)
- Migrate nextcloud from in-process adapter to sidecar (#5301) (@houko)
- Migrate slack from in-process adapter to sidecar (#5302) (@houko)
- Migrate webex from in-process adapter to sidecar (#5309) (@houko)
- Migrate zulip from in-process adapter to sidecar (#5310) (@houko)
- Migrate line from in-process adapter to sidecar (#5312) (@houko)
- Migrate mattermost from in-process adapter to sidecar (#5315) (@houko)
- Migrate signal from in-process adapter to sidecar (#5317) (@houko)
- Migrate qq from in-process adapter to sidecar (#5325) (@houko)
- Migrate matrix from in-process adapter to sidecar (#5368) (@houko)
- Migrate feishu from in-process adapter to sidecar (#5380) (@houko)
- Migrate wecom from in-process adapter to sidecar (WebSocket-only) (#5392) (@houko)
- Migrate email from in-process adapter to sidecar (#5408) (@houko)
- Migrate dingtalk from in-process adapter to sidecar (Stream mode only) (#5417) (@houko)
- Migrate wechat from in-process adapter to sidecar (#5421) (@houko)
- Migrate teams from in-process adapter to sidecar (#5433) (@houko)
- Migrate whatsapp from in-process adapter to sidecar (dual-mode) (#5445) (@houko)
- Migrate webhook from in-process adapter to sidecar (#5455) (@houko)
- Migrate google_chat from in-process adapter to sidecar (#5459) (@houko)
- Restore ChannelsPage as a sidecar-only page (#5470) (@houko)
- Embed librefang-sdk + reconnect WeChat QR flow (#5472) (@houko)
- Approval notifications use inline keyboard on interactive-capable adapters (#5483) (@houko)
- Route approval popup to originating chat (follow-up to #5483) (#5484) (@houko)
- Thread chat_id through approval flow for group-chat support (#5489) (@houko)
- Cache per-session approvals so the same tool only prompts once (#5663) (@houko)
- Per-agent [proactive_memory] extraction_model override (#5475) (#5690) (@houko)
- Bootstrap ESLint with jsx-no-target-blank guard (fixes #5561) (#5701) (@houko)
- Propagate kernel-attested caller context to MCP servers (fixes #5699) (#5704) (@houko)
- Expose per-provider budget caps surface (#5705) (@houko)

### Fixed

- Force HOME so spawned CLI can find its credentials (#4997) (@f-liva)
- Distinguish JoinError cancellation from panic in streaming bridge (#5058) (#5064) (@leszek3737)
- Spill oversized MCP/tool results to artifact store before truncation (#5149) (@neo-wanderer)
- Deny unknown fields in request DTOs to catch body typos (#5131) (#5151) (@houko)
- Validate expression at insert and auto-disable on repeated fallback (#5160) (@houko)
- Unwedge cooldown on wall-clock backstep (#5162) (@houko)
- Respect per-agent fallback_models override — None inherits global, Some([]) opts out (#5167) (@DaBlitzStein)
- Serde/config polish (#5145) (#5172) (@houko)
- AuxClient inherits agent fallback chain when [llm.auxiliary] unset (#5169) (#5173) (@houko)
- Cap rate-limited autonomous loop re-fires (#5168) (#5174) (@houko)
- Time/clock/scheduling robustness (#5136) (#5175) (@houko)
- Surface swallowed errors on persistence/IO paths (#5137) (#5176) (@houko)
- Enforce prompt-cache key determinism (#5143) (#5177) (@houko)
- Security defense-in-depth — symlink/archive/header/IP edge cases (#5141) (#5178) (@houko)
- Enforce per-user memory/wiki ACL at tool dispatch (#5139) (#5179) (@houko)
- Concurrency hazard follow-ups — kill_agent run/abort lifecycle (#5142) (#5180) (@houko)
- Memory substrate data integrity (#5138) (#5181) (@houko)
- Data-layer invalidation + a11y + dead code (#5140) (#5182) (@houko)
- Task lifecycle / resource-leak follow-ups (#5144) (#5184) (@houko)
- Reject same-task re-entrant agent_msg_lock acquisition (#5125, #5126) (#5187) (@houko)
- Show full agent name on hover in chat sidebar (#5188) (@neo-wanderer)
- Regenerate OpenAPI/SDK/schema baselines for #5151 DTO changes (#5165) (#5189) (@houko)
- Prevent history_fold mid-string truncation on verbose-JSON models (#5206) (@houko)
- Re-enable send button on typing:stop (#5207) (@houko)
- /context reports real model context window (#5208) (@houko)
- Surface config deserialize errors and fail closed on hard parse failure (#5209) (@houko)
- Honor token-trigger in inner compaction gate (#5210) (@houko)
- Canonical session pointer recovery on restart (#5198, #5199) (#5211) (@houko)
- Cover ChainExhausted in PooledDriver match (unblock main) (#5215) (@houko)
- Restore rustfmt-clean main after #5209 (#5214) (#5216) (@houko)
- Expose background section + drop stale /api/cron/list allowlist row (#5217) (@houko)
- Sidecar protocol/SDK follow-ups from #5219/#5220 review (#5223) (@houko)
- Move first-party channel adapters out of examples into librefang-sdk (#5228) (@houko)
- Unwrap ephemeral/viewOnce/edited wrappers before reading contextInfo (closes #48) (#5229) (@f-liva)
- Surface producer crash via ProducerCrashed, not SystemExit (#5231) (@houko)
- Handle inbound poll_answer in telegram adapter (sidecar parity) (#5242) (@houko)
- Close kill_agent/dispatch race + break HITL self-cycle (#5244 follow-ups) (#5244) (@houko)
- Unblock main — pass force=false in compact gate test (#5210/#5213 collision) (#5245) (@houko)
- Sidecar channels visible AND read-only on the dashboard (no 404 actions) (#5249) (@houko)
- Surface telegram/ntfy discovery rows on the channels page (#5250) (@houko)
- Auto-pin agentId-only sessions + bind dropdown active to live connection (#5199) (#5253) (@houko)
- Cron picker click no longer closes schedule form (#5247) (#5254) (@houko)
- Agent wizard tools/skills selectable + MCP servers dropdown (#5246) (#5255) (@houko)
- Follow-ups from third sidecar-configure review (#5261) (@houko)
- Block cross-chat memory bleed via chat-scoped recall (#5227) (#5262) (@houko)
- Patch Baileys executeInitQueries to non-blocking allSettled (#5268) (@f-liva)
- Align opentelemetry stack on 0.32 to fix main build break (#5279) (@houko)
- Include kernel Bearer token on all REST forwards (#5285) (@f-liva)
- Thread sender context through streaming message handler (#5288) (@f-liva)
- Skip file-upload OCR for image/* mime types (closes #5290) (#5291) (@DaBlitzStein)
- Add default_agent to SidecarChannelConfig — restore inbound routing pin (closes #5294) (#5295) (@DaBlitzStein)
- Restore main — fmt drift, MCP caller_agent_id semantics, openapi baseline (#5300) (@houko)
- Honour Retry-After across sidecar polling adapters (#5303) (@houko)
- Emit poll bursts in chronological order across sidecar adapters (#5305) (@houko)
- Restore main — fmt drift + stale config schema baseline (#5307) (@houko)
- Detect chat-template `[User]` line-leader as cascade leak (#5308) (@f-liva)
- Update openclaw test fixtures after mattermost sidecar (closes #5316) (#5318) (@houko)
- Wrap config sub-tabs + hide number-input spinner buttons (closes #5293) (#5319) (@houko)
- Pin response_format = Json on history_fold + web_augment aux calls (closes #5287) (#5320) (@houko)
- Observability + regression coverage on sidecar reconnect loop (closes #5111) (#5321) (@houko)
- Define api-error-generic across all 6 locales (audit: api-error-generic-missing-fluent-key) (#5322) (@houko)
- Use canonical getStoredApiKey for export download (audit: audit-export-401) (#5324) (@houko)
- Purge pending_approvals on agent cascade-delete + schema-walking guard (audit: agent-cascade-delete-missing-tables) (#5328) (@houko)
- Refuse to boot without LIBREFANG_STATE_SECRET when external_auth.enabled (closes #5336) (#5337) (@houko)
- Validate skill name + hand against path traversal (closes #5338) (#5339) (@houko)
- Wrap upload_routes in route-local RequestBodyLimitLayer (closes #5342) (#5343) (@houko)
- Cap triggers per agent at MAX_TRIGGERS_PER_AGENT = 50 (closes #5345) (#5346) (@houko)
- Verify caller owns from_agent_id before comms_send (closes #5349) (#5350) (@houko)
- SSRF-validate URLs at create + update (closes #5352) (#5353) (@houko)
- Gate require_auth_for_reads=false bypass behind external_auth_proxy (closes #5356) (#5357) (@houko)
- Add /api/auth/callback to rate-limit allowlist (closes #5358) (#5359) (@houko)
- Expect() on serde_json::to_writer in stream_json (closes #5360) (#5361) (@houko)
- Write Argon2id upgrade-hint to 0600 file instead of log (closes #5364) (#5365) (@houko)
- Kernel_err_to_status helper for 404/409 mapping (closes #5366) (#5367) (@houko)
- Require auth on GitHub Copilot OAuth endpoints (closes #5369) (#5370) (@houko)
- Atomic-rename write for secrets.env eliminates 0644 TOCTOU (closes #5371) (#5372) (@houko)
- Scrub raw rusqlite errors before responding (#5378) (@houko)
- Split /api/auth/login allowlist into exact + slash-prefix (#5382) (@houko)
- Always emit Secure on logout cookie clear (#5384) (@houko)
- Anchor [SILENT] cron marker to message prefix (#5386) (@houko)
- Clamp listing endpoints — no more limit=None → full collection (#5388) (@houko)
- Rel=noopener noreferrer + safeUrl on MCP catalog get_url (#5390) (@houko)
- Hand-write Debug to redact OAuthTokens secrets (#5395) (@houko)
- Warn on serde(other) Unknown variants with raw tag (#5397) (@houko)
- Bind cleanup_orphan_sessions IN-clause instead of string-concat (#5401) (@houko)
- Hotfix dangling refs from #5368 + #5380 sidecar migrations (FeishuConfig + pulldown-cmark) (#5402) (@houko)
- Drop dangling channels.feishu access in openclaw roundtrip test (#5404) (@houko)
- Bound regex cache at 4096 entries with FIFO eviction (#5406) (@houko)
- Per-process random anonymous fingerprint (#5410) (@houko)
- Wire check_json_depth into global request middleware (#5412) (@houko)
- Use SHA-256 (128-bit truncated) for DriverCache::cache_key (#5414) (@houko)
- Persist trimmed active_sessions after periodic GC (#5419) (@houko)
- Tighten SQLite database files to 0o600 + data dir to 0o700 (#5422) (@houko)
- Recover sessionWebhook via ChannelUser.librefang_user (#5423) (@houko)
- Sanitize Custom channel names that collide with kernel-internal cron/autonomous/webui (#5425) (@houko)
- Byte + char dual cap on chat-message size (#5427) (@houko)
- Saturating_add inner cache-token sum (#5430) (@houko)
- Recover passive-reply msg_id via ChannelUser.librefang_user (#5431) (@houko)
- Apply foreign_keys=ON + full PRAGMA set to PromptStore second pool (#5434) (@houko)
- Scan raw string for command substitution — close double-quote bypass (#5436) (@houko)
- Recover per-message reply correlation via ChannelUser.librefang_user across 6 sidecars (#5439) (@houko)
- Size-bounded PII regex compilation (#5444) (@houko)
- Repair persisted session after trim+pinned-rescue (#5447) (@houko)
- Recover context_token via librefang_user across sidecar restart (#5448) (@houko)
- Recover req_id via librefang_user across sidecar restart (#5449) (@houko)
- Include traceback + cmd_type when on_command bare-except logs (#5450) (@houko)
- WARN on env-vs-keyring master-key divergence (#5453) (@houko)
- Recover main build from sidecar fallout (missing default + orphans + test drift) (#5456) (@houko)
- Repair test build after #5455 (write_service_account_env removed) (#5460) (@houko)
- Cross-audit follow-ups (Retry-After x4, dedupe x2, LINE reply API) (#5462) (@houko)
- Rewrite ModuleNotFoundError into actionable install hint (#5465) (@houko)
- Preserve specific cause in last_error after circuit-breaker trip (#5468) (@houko)
- Redact WhatsApp JIDs atomically (no partial-redact via phone regex) (#5469) (@f-liva)
- Recover reply context via XRPC on cache miss (closes #5452) (#5471) (@houko)
- Demote /api/metrics 401 from WARN to DEBUG (#5482) (@houko)
- Repair 3 pre-existing main CI breakers inherited by all open PRs (#5486) (@houko)
- Ack duplicate `/approve <id>` instead of error-shaped not-found (#5487) (@houko)
- Wake idle agent after approval resolve so the chat gets the result (#5488) (@houko)
- Suppress redundant /approve|/reject ack on inline-keyboard tap (#5490) (@houko)
- Route agent reply through channel after wake — fixes "tap Approve → silence" (#5491) (@houko)
- Cargo fmt + regenerate sdk/ to repair main CI (Quality, OpenAPI Drift) (#5494) (@houko)
- Log only email domain at INFO in OIDC auth_callback (#5504) (@houko)
- Sanitize reserved channel names at every SenderContext ingress (#5506) (@houko)
- Return path relative to home_dir, not absolute (#5509) (@houko)
- Keyboard nav for NotificationCenter (WAI-ARIA Menu Button) (#5510) (@houko)
- Record comms_send in hash-chained audit log (#5512) (@houko)
- Reject empty code in OAuth callback before token exchange (#5515) (@houko)
- SSRF-validate attachment URLs + DNS-rebind pin (#5517) (@houko)
- Cap bulk-handler Vec::with_capacity to prevent DoS pre-allocation (#5520) (@houko)
- Bound buckets map with hard cap + periodic sweep (#5522) (@houko)
- Never log raw IdP token-endpoint response bodies (#5526) (@houko)
- Detect partial-upgrade drift between migrations table and user_version pragma (#5528) (@houko)
- Never silently default or fabricate from corrupt JSON-in-TEXT columns (#5532) (@houko)
- Reclaim per-session bucket on session delete (#5534) (@houko)
- Bound RoundRobin cursor with cycle-aware iteration (#5536) (@houko)
- Restore main — rustfmt drift + 2 PR-only test failures (#5538) (@houko)
- Release prune lock across try_summarize_trim().await; CAS on messages_generation (#5541) (@houko)
- Validate provider name shape before deriving env var (#5542) (@houko)
- Bijective SHA-256 agent_id suffix to stop container-name collisions (#5545) (@houko)
- Hold ledger mutex across check + add (#5548) (@houko)
- Validate tool args at boundary before forwarding to MCP server (#5550) (@houko)
- Acquire per-agent semaphore in workflow send_message closure (#5554) (@houko)
- Cap system_prompt size and lock down create-handler invariants (#5558) (@houko)
- Allow zero spaces in attribution regex (#5560) (@houko)
- Swap RefCell for parking_lot::Mutex to remove async borrow-panic footgun (#5563) (@houko)
- Reject `..` per-segment in react_asset, not by substring (#5565) (@houko)
- Switch useSessionStream to authenticated WebSocket (#5567) (@houko)
- Make agent_concurrency_for entry construction atomic (#5569) (@houko)
- Hash session tokens at rest in sessions.json — backup-snapshot replay resistance (#5571) (@houko)
- #[serde(skip_serializing)] api_key + proxy_url (#5573) (@houko)
- Escape translator HTML, route via <Trans> (#5576) (@houko)
- Canonicalize + containment-check source/target_dir (#5577) (@houko)
- Warn at boot when declared provider API-key env vars are unset or empty (#5579) (@houko)
- Gate X-Forwarded-Proto on trusted_proxies for session cookie Secure flag (#5581) (@houko)
- Allowlist --network and --cap-add to prevent sandbox collapse (#5583) (@houko)
- Install-deps program allowlist + flag denylist + Owner-only role (#5588) (@houko)
- Warn on manifest swap when session_mode or max_concurrent_invocations changes (#5590) (@houko)
- Remove partial identity files on write failure (#5592) (@houko)
- Evict JWKS + discovery caches on external_auth hot-reload (#5594) (@houko)
- Fail-closed when guard-bash-safety lib is missing (#5596) (@houko)
- Rephrase strip_images placeholder so LLM does not deny image reception (#5597) (@DaBlitzStein)
- Wrap connect_mcp_servers spawns in spawn_supervised (#5599) (@houko)
- Hold Lane::Trigger permit across run_workflow spawn (#5602) (@houko)
- Derive deterministic SessionId for New-mode fires (#5604) (@houko)
- Align missed-fire log with single-catchup behaviour (#5606) (@houko)
- Classify refresh failures, single-flight refresh, drop unwrap (#5609) (@houko)
- Allow known framework source dirs, not just the librefang home (#5614) (@houko)
- Backfill missing #[utoipa::path] handlers + regenerate openapi.json (#5620) (@houko)
- API-surface hygiene — SPA route allowlist, registry id validation, auth/providers gating (#5638) (@houko)
- Non-IdP external_auth edits are a no-op, not a restart (#5646) (@houko)
- Propagate sender peer_id through remember_interaction_b… (#5647) (@Chukwuebuka-2003)
- Persist /sync since_token across restarts (#5651) (@neo-wanderer)
- External_auth IdP change is hot-reload, not restart (restore main) (#5652) (@houko)
- Clear clippy Quality lane (needless borrow, doc indentation, manual char comparison, await-holding-lock) (#5654) (@houko)
- Downgrade boot integrity-check failure to WARN (#5659) (@houko)
- Migrate legacy shared-namespace row on fallback hit (#5660) (@houko)
- Bound graceful shutdown so daemon.lock release isn't blocked by a hung phase (#5662) (@houko)
- Plug data leaks, restore lost state, harden parsing (#5674) (@leszek3737)
- Harden pre-commit + add detect-secrets CI workflow (#5681) (@houko)
- CommsKeys hierarchy + TerminalTabs storage helper + Modal autoFocus (#5682) (@houko)
- Soft-cap in-memory entries between trims at 1.5x max_in_memory_entries (#5683) (@houko)
- Harden build.rs git/date invocation; document pnpm audit ignores (#5684) (@houko)
- WARN when [agents.<name>.proactive_memory] appears in config.toml (real path is agent.toml) (#5687) (@houko)
- Filter /commands dispatch by account_id (multi-bot isolation) (#5688) (@houko)
- Widen exclusions, regenerate baseline, ignore generated_at drift (#5691) (@houko)
- Update audit_retention_test for #5683 soft-cap drain (#5693) (@houko)
- Strip line_number drift from detect-secrets baseline diff (#5695) (@houko)
- Log bot_token fingerprint instead of full token (fixes #5543) (#5700) (@houko)
- Replace removed `all-channels` feature with `telemetry` (#5702) (@houko)
- Add provider_budget_routes_test to detect-secrets baseline (#5707) (@houko)
- Regenerate SDKs for /api/budget/providers to repair main CI drift (#5709) (@houko)
- Include sdk/python/librefang in flake source filter (#5714) (@houko)

### Changed

- Unify error contracts — RFC + ToolError + first migration (#3576) (#5258) (@houko)
- Extract shared helpers + WS client + test fakes (#5335) (@houko)
- Return librefang-types IntegrationError from install_integration (stop leaking ExtensionResult) (#5622) (@houko)
- Return types-owned outcome from install_integration (stop leaking InstallResult) (#5644) (@houko)
- Widen ApiErrorResponse::internal_scrub sweep across routes (#5661) (@houko)

### Performance

- Use count_sessions() on status + snapshot (audit: list-sessions-decode-on-poll) (#5326) (@houko)
- Use list_arcs() in agent_budget_ranking (closes #5347) (#5348) (@houko)
- Evict stale tool-call timestamps on push (closes #5362) (#5363) (@houko)
- Rotate to next key on first RateLimit (closes #5373) (#5374) (@houko)
- Tx-wrap recall access bump + batched IN hydrate (closes #5375) (#5376) (@houko)
- Composite sessions(agent_id, updated_at) + audit_entries(agent_id, timestamp) indexes (#5399) (@houko)
- Stream extract_text_content into a single String to avoid per-save Vec<String> allocation (#5501) (@houko)
- Offload SQLite insert+prune via spawn_blocking, counter-gate prune (#5524) (@houko)
- Block_in_place for ImageFile reads (4 sites) (#5530) (@houko)
- Memoize dashboard_snapshot_inner with 900ms TTL cache (#5552) (@houko)
- Unblock axum executor on create_backup + persist_budget (spawn_blocking) (#5556) (@houko)

<details>
<summary>Documentation, maintenance, and other internal changes</summary>

### Documentation

- Sidecar-first channel documentation (P6) (#5225) (@houko)
- Import audit backlog (120 tracking items) (#5240) (@houko)
- Fix stale telegram.rs reference in custom-channel example (#5248) (@houko)
- Fill in [[sidecar_channels]] samples for all 27 adapters (#5464) (@houko)
- Canonical config-reload field table derived from build_reload_plan (#5642) (@houko)

### Maintenance

- Restore rustfmt-clean main (Quality CI gate) (#5222) (@houko)
- Add Dockerfile.rust-dev with Tauri Linux GTK deps (#5233) (@houko)
- Cross-impl protocol conformance corpus + versioned spec (v1) (#5237) (@houko)
- Remove 6 low-value channel adapters (#5265) (@houko)
- Drop per-merge auto-update trigger from auto-update-branches (#5266) (@houko)
- Drop 12 unmaintained adapters (#5267) (@houko)
- Bump the cargo-minor-patch group with 8 updates (#5269) (@app/dependabot)
- Bump opentelemetry-otlp from 0.31.1 to 0.32.0 (#5270) (@app/dependabot)
- Bump russh-keys from 0.45.0 to 0.49.2 (#5271) (@app/dependabot)
- Bump shlex from 1.3.0 to 2.0.1 (#5272) (@app/dependabot)
- Bump tracing-opentelemetry from 0.32.1 to 0.33.0 (#5273) (@app/dependabot)
- Cargo fmt — fix rustfmt drift on main after channel-removal merges (#5274) (@houko)
- Bump Apple-Actions/upload-testflight-build from 5.1.0 to 5.2.1 in the actions-minor-patch group (#5304) (@app/dependabot)
- Pin silent_response markers against prompt-builder output (#5344) (@f-liva)
- Drop pulldown-cmark workspace dep, orphaned by matrix sidecar #5368 (#5407) (@houko)
- Pin SessionMode strict-variant deserialization (audit-disputed) (#5416) (@houko)
- Bump the web-minor-patch group in /web with 9 updates (#5438) (@app/dependabot)
- Bump the dashboard-minor-patch group in /crates/librefang-api/dashboard with 12 updates (#5440) (@app/dependabot)
- 3 nits from post-merge audit (#5454) (@houko)
- Remove dead in-process channel scaffolding (#5461) (@houko)
- Delete dead per-channel REST endpoints + their helpers (#5463) (@houko)
- Rephrase docstring "stub" mentions to stop bot false positives (#5467) (@houko)
- Prune unused dependencies across the workspace (#5473) (@houko)
- Clean up sidecar migration tails (#5479) (@houko)
- Bump the docs-minor-patch group in /docs with 10 updates (#5493) (@app/dependabot)
- Skip Cloudflare Pages deploy on Dependabot PRs (#5495) (@houko)
- Run Coverage workflow on push:main only, not per-PR (#5496) (@houko)
- Make the per-PR test lane Linux-only (#5498) (@houko)
- Cover LIBREFANG_VAULT_KEY 32-ASCII-vs-32-bytes pitfall (#5611) (@houko)
- Replace fixed 150ms sleeps with condition-based polling (#5613) (@houko)
- Parallel semaphore-contention coverage for trigger concurrency caps (#5616) (@houko)
- Assert every KernelConfig field is reload-classified + backfill (#5619) (@houko)
- Replace unmaintained serde_yaml with serde_yaml_ng (RUSTSEC-2024-0320) (#5626) (@houko)
- Full-router semantic tests for lifecycle routes (suspend/resume/mode) (#5628) (@houko)
- Convert tools integration tests from mock to full router (#5630) (@houko)
- Convert load_test from mock to full router (exercise real middleware) (#5632) (@houko)
- Full-router semantic tests for files (path-traversal) + capabilities routes (#5634) (@houko)
- Convert agent_identity_registry tests from mock to full router (#5636) (@houko)
- Full-router semantic tests for clone/reload/push + bulk routes (#5640) (@houko)
- Delete 65 audit docs whose GitHub issue is closed (#5670) (@houko)
- Rename librefang-migrate → librefang-import + reconcile stale CLAUDE.md + justfile policy (#5668) (#5685) (@houko)

### Reverted

- Roll back v2026.5.25-beta.13 / beta.14 version bumps to 2026.5.17-beta.12 (#5717) (@houko)

### Other

- [Medium] Per-trigger `session_mode_override = New` is throttled by the manifest's `Persistent` clamp (#5624) (@houko)

</details>


