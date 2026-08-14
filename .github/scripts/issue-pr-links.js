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
    const number = Number.parseInt(raw, 10);
    if (Number.isSafeInteger(number) && number > 0) {
      numbers.add(number);
    }
  }
  return numbers;
}

async function getIssueOrNull(github, repo, number) {
  try {
    const response = await github.rest.issues.get({
      owner: repo.owner,
      repo: repo.repo,
      issue_number: number,
    });
    return response.data;
  } catch (error) {
    if (error.status === 404) {
      return null;
    }
    throw error;
  }
}

function hasOpenPeerLink(openPrs, issueNumber, repo, eventPrNumber) {
  return openPrs.some(
    candidate => candidate.number !== eventPrNumber &&
      extractIssueNumbers(candidate.body, repo).has(issueNumber),
  );
}

module.exports = { extractIssueNumbers, getIssueOrNull, hasOpenPeerLink };
