<!-- PR title must follow conventional commit format: type[scope]: description -->
<!-- Examples: feat: add X, fix(kernel): resolve Y, docs: update Z -->
<!-- Allowed types: feat, fix, docs, style, refactor, perf, test, build, ci, chore, revert -->

## Type

<!-- Check one: -->

- [ ] Agent template (TOML)
- [ ] Skill (Python/JS/Prompt)
- [ ] Channel adapter
- [ ] LLM provider
- [ ] Built-in tool
- [ ] Bug fix
- [ ] Feature (Rust)
- [ ] Documentation / Translation
- [ ] Refactor / Performance
- [ ] CI / Tooling
- [ ] Other

## Summary

<!-- What does this PR do? Link related issues with "Fixes #123". -->

## Changes

<!-- Brief list of what changed. -->

## Attribution

- [ ] This PR preserves author attribution for any adapted prior work (`Co-authored-by`, commit preservation, or explicit credit in the PR body)

## Testing

- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] `cargo test --workspace` passes
- [ ] Live integration tested (if applicable)

## Security

- [ ] No new unsafe code
- [ ] No secrets or API keys in diff
- [ ] User input validated at boundaries
