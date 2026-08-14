'use strict';

async function listOpenPulls(github, repo) {
  return github.paginate(github.rest.pulls.list, {
    owner: repo.owner,
    repo: repo.repo,
    state: 'open',
    sort: 'updated',
    direction: 'asc',
    per_page: 100,
  });
}

async function removeLabel(github, repo, issueNumber, label, core) {
  try {
    await github.rest.issues.removeLabel({
      owner: repo.owner,
      repo: repo.repo,
      issue_number: issueNumber,
      name: label,
    });
    return true;
  } catch (error) {
    if (error.status === 404) {
      core.info(`#${issueNumber}: "${label}" was already absent`);
      return false;
    }
    throw error;
  }
}

module.exports = { listOpenPulls, removeLabel };
