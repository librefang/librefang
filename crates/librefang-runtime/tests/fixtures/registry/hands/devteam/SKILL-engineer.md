---
name: devteam-engineer-skill
version: "1.0.0"
description: "Engineer reference knowledge — tech stack detection, build commands, git workflow, debugging patterns"
runtime: prompt_only
---

# Engineer Reference Knowledge

## Tech Stack Detection

| File | Stack | Build | Test | Lint |
|------|-------|-------|------|------|
| `Cargo.toml` | Rust | `cargo build` | `cargo test` | `cargo clippy -- -D warnings` |
| `package.json` + `tsconfig.json` | TypeScript | `npm run build` | `npm test` | `npm run lint` |
| `package.json` | JavaScript | `npm run build` | `npm test` | `npm run lint` |
| `go.mod` | Go | `go build ./...` | `go test ./...` | `golangci-lint run` |
| `pyproject.toml` | Python | — | `pytest` | `ruff check .` |
| `pom.xml` | Java | `mvn compile` | `mvn test` | `mvn checkstyle:check` |
| `Package.swift` | Swift | `swift build` | `swift test` | `swiftlint` |

## Git Workflow

```bash
# New task
git checkout main && git pull
git checkout -B feat/issue-42

# Commit (specific files only)
git add path/to/file.rs path/to/test.rs
git commit -m "fix: description (#42)"
git push -u origin feat/issue-42

# Create PR
gh pr create --title "fix: description (#42)" --body "Closes #42" --head feat/issue-42 --base main

# Rebase on conflict
git fetch origin && git rebase origin/main && git push --force-with-lease
```

## Debugging Methodology

1. **REPRODUCE** — get error message, stack trace, exact failure
2. **ISOLATE** — read source, git log/diff, narrow search space
3. **IDENTIFY** — trace data flow, check boundaries, find root cause (not symptoms)
4. **FIX** — minimal correct fix, don't refactor
5. **VERIFY** — write regression test, run full suite

## Common Bug Patterns

- Off-by-one errors, null/None handling
- Resource leaks (file handles, connections)
- Error handling paths (what happens on failure?)
- Race conditions, shared mutable state
- Type mismatches, silent truncation
