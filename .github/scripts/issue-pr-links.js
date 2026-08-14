'use strict';

function escapeRegex(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function extractIssueNumbers(body, repo) {
  const repository = `${escapeRegex(repo.owner)}/${escapeRegex(repo.repo)}`;
  const target = [
    '#(\\d+)',
    `${repository}#(\\d+)`,
    `https://github\\.com/${repository}/issues/(\\d+)`,
  ].join('|');
  const pattern = new RegExp(
    `\\b(?:close[sd]?|fix(?:e[sd])?|resolve[sd]?)\\s+(?:${target})\\b`,
    'gi',
  );
  const numbers = new Set();
  for (const match of String(body || '').matchAll(pattern)) {
    const raw = match[1] || match[2] || match[3];
    numbers.add(Number.parseInt(raw, 10));
  }
  return numbers;
}

module.exports = { extractIssueNumbers };
