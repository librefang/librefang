'use strict';

const RED_CONCLUSIONS = new Set([
  'failure',
  'timed_out',
  'cancelled',
  'startup_failure',
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

module.exports = { escapeInlineCode, escapeMarkdownText, isRedConclusion };
