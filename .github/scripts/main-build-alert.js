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

function escapeMarkdownText(value) {
  return String(value)
    .replaceAll('@', '@\u200b')
    .replace(/([\\`*_{}\[\]()<>#+.!|~-])/g, '\\$1');
}

function escapeInlineCode(value) {
  return String(value).replaceAll('`', "'").replaceAll('@', '@\u200b');
}

module.exports = { escapeInlineCode, escapeMarkdownText, isRedConclusion };
