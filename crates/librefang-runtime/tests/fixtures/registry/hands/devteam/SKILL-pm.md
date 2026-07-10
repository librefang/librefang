---
name: devteam-pm-skill
version: "1.0.0"
description: "PM reference knowledge — issue triage framework, GitHub CLI, project board patterns"
runtime: prompt_only
---

# PM Reference Knowledge

## Issue Triage

| Signal | Type |
|--------|------|
| "doesn't work", "error", "crash" | bug |
| "add", "new", "support" | feature |
| "clean up", "simplify", "rename" | refactor |
| "document", "readme" | docs |

| Priority | Criteria |
|----------|----------|
| P0 | Production down, data loss, security |
| P1 | Core feature broken |
| P2 | Feature request, non-critical bug |
| P3 | Nice-to-have, cosmetic |

| Size | Scope |
|------|-------|
| S | < 50 lines, 1 file |
| M | 50-200 lines, 2-5 files |
| L | 200-1000 lines, needs design |
| XL | 1000+, decompose first |

## gh CLI Quick Reference

```bash
# Issues
gh issue list --repo OWNER/REPO --state open --json number,title,labels,assignees --limit 30
gh issue view NUMBER --repo OWNER/REPO --json body,comments
gh issue comment NUMBER --repo OWNER/REPO --body "message"
gh issue close NUMBER --repo OWNER/REPO --reason completed
gh issue edit NUMBER --repo OWNER/REPO --add-label "bot:triaged"

# PRs
gh pr list --repo OWNER/REPO --state open --json number,title,headRefName,statusCheckRollup
gh pr checks NUMBER --repo OWNER/REPO
gh pr merge NUMBER --repo OWNER/REPO --squash
gh pr revert NUMBER --repo OWNER/REPO

# Code browsing
gh api repos/OWNER/REPO/contents/PATH --jq '.content' | base64 -d
```

## Board Schema

```json
{
  "backlog": [{"issue": 42, "title": "...", "type": "bug", "size": "M", "priority": "P1"}],
  "in_progress": [{"issue": 43, "assignee": "engineer", "branch": "fix/issue-43", "round": 0}],
  "in_review": [{"issue": 44, "pr": 50}],
  "done": [{"issue": 45, "closed_at": "2025-01-02"}]
}
```

## Scan Interval Mapping

| Setting | schedule_create every_secs |
|---------|--------------------------|
| 5min | 300 |
| 15min | 900 |
| 1hour | 3600 |
