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
  escapeInlineCode,
  escapeMarkdownText,
  isRedConclusion,
  selectOpenAlertIssues,
};
