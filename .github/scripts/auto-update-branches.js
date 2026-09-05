'use strict';

const FAIL_CONCLUSIONS = new Set([
  'failure',
  'timed_out',
  'action_required',
  'startup_failure',
  'cancelled',
  'stale',
]);

async function listOpenPulls(github, repo) {
  return github.paginate(github.rest.pulls.list, {
    owner: repo.owner,
    repo: repo.repo,
    state: 'open',
    per_page: 100,
  });
}

async function listCheckRuns(github, repo, ref) {
  return github.paginate(github.rest.checks.listForRef, {
    owner: repo.owner,
    repo: repo.repo,
    ref,
    filter: 'latest',
    per_page: 100,
  });
}

// Only the checks the `main` ruleset actually requires get a vote on whether a PR is failing.
// `CI Gate` is the sole entry in that ruleset's `required_status_checks`; every other check run on a PR is advisory and cannot block a merge, so it must not be able to trigger a branch update either.
//
// Reading every check run instead is what defeated the "only update failing PRs" guard below.
// A bookkeeping workflow whose runs cancelled each other left a `cancelled` check on nearly every open pull request, `cancelled` is in FAIL_CONCLUSIONS, and so every PR read as failing — which turned each scheduled sweep into the full-repository cascade the guard exists to prevent, roughly thirty branch updates and thirty CI matrices per fire.
// Narrowing the input is the durable half of that fix: even if another advisory check starts reporting `cancelled`, it can no longer reach this decision.
//
// `cancelled` deliberately stays in FAIL_CONCLUSIONS. A timeout-killed job reports as `cancelled` rather than `failure`, so dropping it would blind the updater to a genuinely stuck required check.
const REQUIRED_CHECK_NAMES = new Set(['CI Gate']);

function hasFailingStatus(checkRuns, combinedState) {
  return checkRuns.some(
    (run) =>
      run.conclusion &&
      REQUIRED_CHECK_NAMES.has(run.name) &&
      FAIL_CONCLUSIONS.has(run.conclusion),
  ) || combinedState === 'failure' || combinedState === 'error';
}

module.exports = { REQUIRED_CHECK_NAMES, hasFailingStatus, listCheckRuns, listOpenPulls };
