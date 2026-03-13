# Governance

## Goals

LibreFang — "libre" as in freedom — exists to maintain the codebase as a truly open community project. We accept outside contributions through normal GitHub workflows, preserve contributor credit, and actively help contributors get their work merged.

## Core Principle: Merge-First

**If a contribution positively helps the project, we merge it.** This is the default, not the exception.

- PRs that meet quality standards are merged as-is with full contributor attribution.
- PRs that need improvement receive active, constructive code review with specific suggestions — we help contributors ship their work, not block it.
- We do not silently close PRs or let them go stale without explanation.

## Decision-Making

- Day-to-day technical decisions are made in pull requests and issues — not behind closed doors.
- Maintainers are expected to explain design rejections with concrete technical reasons.
- Breaking governance changes should be proposed in a pull request against this file.

## Contribution Policy

- Accepted changes land through GitHub merge or squash merge.
- If a maintainer needs to adapt or rewrite a contributor's patch, the maintainer preserves attribution with commit metadata such as `Co-authored-by` and credits the contributor in release notes.
- **Closing a pull request and re-implementing it privately without attribution is explicitly prohibited.**
- Large design changes should start as an issue so contributors can align before doing heavy implementation work.
- All types of contributions are valued equally: code, documentation, tests, translations, packaging, issue triage, and community support.

## Review Expectations

- New pull requests receive an initial maintainer response within 7 days.
- If a pull request is blocked on architecture or scope, maintainers say so explicitly and suggest a path forward.
- Stale pull requests may be closed after explanation, but contributors are always told the reason and how to revive the work.

## Maintainers

- Repository maintainers are responsible for review quality, release management, and enforcing this governance document.
- Maintainers should avoid becoming a single-person bottleneck. When possible, at least two people should be able to review and release the project.
- Maintainer expectations and the current roster are tracked in [`MAINTAINERS.md`](MAINTAINERS.md).

## Security

Security reports use the private process in [`SECURITY.md`](SECURITY.md), not public issues.
