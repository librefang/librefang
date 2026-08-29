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

function hasFailingStatus(checkRuns, combinedState) {
  return checkRuns.some(
    (run) => run.conclusion && FAIL_CONCLUSIONS.has(run.conclusion),
  ) || combinedState === 'failure' || combinedState === 'error';
}

module.exports = { hasFailingStatus, listCheckRuns, listOpenPulls };
