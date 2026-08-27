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

function collectReconciliationState(openPrs, labeledItems, repo, eventPr) {
  const linkedIssueNumbers = eventPr.state === 'open'
    ? extractIssueNumbers(eventPr.body, repo)
    : new Set();

  for (const candidate of openPrs) {
    if (candidate.number === eventPr.number) {
      continue;
    }
    for (const number of extractIssueNumbers(candidate.body, repo)) {
      linkedIssueNumbers.add(number);
    }
  }

  const issueNumbers = new Set(linkedIssueNumbers);
  for (const item of labeledItems) {
    if (!item.pull_request && Number.isSafeInteger(item.number) && item.number > 0) {
      issueNumbers.add(item.number);
    }
  }

  return { issueNumbers, linkedIssueNumbers };
}

module.exports = { collectReconciliationState, extractIssueNumbers, getIssueOrNull };
