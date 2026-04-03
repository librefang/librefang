# ADR-013: Mandatory Development Workflow

**Status**: Accepted
**Date**: 2026-03-20
**Authors**: Daniel Alberttis

## Version History

| Version | Date | Author | Changes |
|---------|------|--------|---------|
| 1.0 | 2026-03-20 | Daniel Alberttis | Initial — codifies 10-step workflow derived from audit of G5/G6 and META_SEG rounds |

---

## Context

After 18 rounds of memory store implementation, a pattern emerged: the quality of each round was directly correlated to whether a structured investigation (Sherlock baseline) preceded coding, and whether AQE agents ran before the coder agent. Rounds without these steps produced:

- False claims in session summaries (e.g., "pre-existing vendor clippy errors" that were actually introduced in Round 17/18)
- Incomplete health observability (corrupt-skip paths without counters)
- Spec deviations that persisted for multiple rounds because no gate enforced they be closed before moving on

Codex self-assessed the implementation output at 88/100 and identified the missing workflow governance as the primary gap.

The 10-step workflow in this ADR closes that gap by making each quality gate a hard prerequisite for the next step, not an optional audit after the fact.

---

## Decision

All code changes in openfang-ai follow the 10-step mandatory workflow defined below. The workflow is enforced via:

1. **CLAUDE.md hard rules** — always active, cannot be disabled
2. **`flow-coach-codex-dev` skill** — operational guide with gate definitions and agent assignments
3. **This ADR** — authoritative definitions and rationale for gate pass/fail criteria

### The 10-Step Workflow

| Step | Name | Gate Pass Condition | Skip Condition |
|------|------|---------------------|----------------|
| 1 | ADR | File exists, Status: Accepted | No cross-crate API/type changes AND no new design decisions |
| 2 | SPEC | File exists with acceptance criteria + `Claims requiring verification` section | Never |
| 3 | Sherlock Baseline | `Verified Baseline Facts` block in PLAN with real test output | Never |
| 4 | PLAN | All 6 required sections populated | Never |
| 5 | AQE Pre-code | `qe-requirements-validator`, `qe-devils-advocate`, `qe-impact-analyzer` all ran | Never (may run as single combined agent in "fast mode") |
| 6 | RED | All new tests exist and FAIL for the right reason | Never |
| 7 | GREEN | All RED tests pass, no regressions | Never |
| 8 | REFACTOR | `cargo clippy --workspace --all-targets -- -D warnings` exits zero | Never |
| 9 | Review | No BLOCKER findings from `qe-code-reviewer` | Security review may be skipped if change is non-security-sensitive |
| 10 | Final Sherlock | Every claim in summary has a passing test or Sherlock verdict | Never |

### Claim Policy

**No session summary, PR description, or commit message may contain an implementation claim unless it cites:**
- A passing test name from the PLAN exit criteria, **OR**
- A Sherlock verdict (✓ TRUE or ⚠ PARTIALLY TRUE)

Claims without evidence must be removed or downgraded to "intended" / "believed to be".

---

## Consequences

**Positive**:
- False implementation claims eliminated at Step 10
- Spec deviations caught at Step 3 (baseline) before becoming multi-round debt
- AQE agents surface edge cases before the coder writes anything
- Learnings saved to pi-brain after Step 10, so future similar tasks benefit from prior patterns

**Negative**:
- Each round requires more upfront work (Steps 1–5 before any code)
- Small fixes (1–5 line changes) still require at minimum Steps 2, 3, 4, 6–8, 10 — the "fast path" still has 7 required steps

**Mitigation for small fixes**: Steps 1 (ADR) and 5 (AQE) may be skipped for fixes that are strictly contained within a single function and have no API/type changes. All other steps are mandatory regardless of change size.

---

## Alternatives Considered

### A: Post-hoc Sherlock only

Sherlock runs after coding instead of before (Step 10 only). Rejected — the baseline comparison is impossible without a pre-coding snapshot. Without Step 3, Sherlock can only verify claims, not detect regressions from the starting state.

### B: AQE agents optional

Make AQE pre-code agents advisory rather than gating. Rejected — in the G5/G6 round, qe-devils-advocate would have surfaced the `update_embedding` content_map migration case that was initially missed. Optional agents get skipped under time pressure.

### C: Encode workflow in CLAUDE.md only

Single-file governance. Rejected — CLAUDE.md is for always-active short rules. The full 10-step operational detail belongs in a skill where it can be invoked with context-aware brain pattern injection.

---

## Gate Pass/Fail Definitions

### What counts as evidence for a claim

| Claim type | Required evidence |
|------------|-------------------|
| "Feature X is implemented" | Test name from PLAN exit criteria that specifically exercises X |
| "Bug Y is fixed" | Test name that reproduces Y (would have failed before fix) |
| "Performance improved" | Benchmark output before/after |
| "No regressions" | `cargo test --workspace` exit zero with count ≥ baseline |
| "Clippy clean" | `cargo clippy --workspace --all-targets -- -D warnings` exit zero |

### What does NOT count as evidence

- "I read the code and it looks correct"
- "The build passes"
- "Tests pass" without specifying which tests
- Commit messages or PR descriptions from the same session (circular)

---

## Canonical Document Locations

| Artifact | Location | Purpose |
|----------|----------|---------|
| ADRs | `docs/adr/ADR-NNN-<slug>.md` | Architectural decisions |
| SPECs | `docs/specs/SPEC-NNN-<slug>.md` | Acceptance criteria |
| PLANs | `docs/plans/PLAN-NNN-<slug>.md` | Implementation + gate tracking |
| Session audits | `docs/audits/<date>.md` | Post-round investigation records |
| Skill | `.claude/skills/flow-coach-codex-dev/SKILL.md` | Operational workflow guide |

---

## Review Schedule

This ADR should be revisited when:
- The AQE agent catalogue changes significantly
- A workflow gate consistently produces false negatives (misses real bugs)
- A workflow gate consistently produces false positives (blocks valid work)
- The project adds a second language or runtime environment
