'use strict';

const RED_CONCLUSIONS = new Set([
  'failure',
  'timed_out',
  'cancelled',
  'startup_failure',
  'action_required',
  'stale',
]);

function isRedConclusion(conclusion) {
  return RED_CONCLUSIONS.has(conclusion);
}

function compareRunOrder(left, right) {
  const runDelta = left.run_number - right.run_number;
  return runDelta || (left.run_attempt || 1) - (right.run_attempt || 1);
}

function rustTestLaneSucceeded(jobs) {
  const rustTestLane = /^Test \/ (Windows|macOS|Ubuntu|Unit)/;
  return jobs.some(
    job => rustTestLane.test(job.name) && job.conclusion === 'success',
  );
}

async function selectLatestConclusiveRun(runs, loadJobs) {
  const ordered = [...runs].sort((left, right) => compareRunOrder(right, left));
  for (const run of ordered) {
    if (isRedConclusion(run.conclusion)) {
      return { run, state: 'red' };
    }
    if (run.conclusion === 'success' && rustTestLaneSucceeded(await loadJobs(run))) {
      return { run, state: 'green' };
    }
  }
  return null;
}

function singleLine(value) {
  return String(value).replace(/[\u0000-\u001f\u007f\u2028\u2029]+/g, ' ');
}

function escapeMarkdownText(value) {
  return singleLine(value)
    .replaceAll('@', '@\u200b')
    .replace(/([\\`*_{}\[\]()<>#+.!|~-])/g, '\\$1');
}

function escapeInlineCode(value) {
  return singleLine(value).replaceAll('`', "'").replaceAll('@', '@\u200b');
}

function selectOpenAlertIssues(items) {
  return items.filter(item => !item.pull_request);
}

module.exports = {
  compareRunOrder,
  escapeInlineCode,
  escapeMarkdownText,
  isRedConclusion,
  rustTestLaneSucceeded,
  selectLatestConclusiveRun,
  selectOpenAlertIssues,
};
