# ADR-020: Hybrid Bundled-Plus-Registry Hands Distribution

**Status**: Proposed
**Date**: 2026-04-02
**Authors**: Daniel Alberttis

## Version History

| Version | Date | Author | Changes |
|---------|------|--------|---------|
| 0.1 | 2026-04-02 | Daniel Alberttis | Initial draft. Proposes evolving Hands from bundled-only delivery toward a hybrid model with bundled defaults plus registry-backed distribution, initially described as using the local `librefang` fork as the concrete reference implementation to emulate selectively. |
| 0.2 | 2026-04-02 | Daniel Alberttis | Clarified that LibreFang is evidence for filesystem-backed Hand loading, not a complete hybrid/source-aware implementation. Reframed the ADR as an OpenFang design that intentionally goes beyond LibreFang's current implementation. |

---

## Context

OpenFang currently has two different distribution stories for two adjacent capability systems:

- **Skills** already have an ecosystem-oriented model: installable artifacts, marketplace flow, and registry-style discovery.
- **Hands** are still primarily a bundled capability system shipped with the product.

In the current `openfang-ai` codebase, the Hands registry loads bundled definitions via `crate::bundled` in [crates/openfang-hands/src/registry.rs](/Users/danielalberttis/Desktop/Projects/openfang-ai/crates/openfang-hands/src/registry.rs). That means the default model is:

1. Hands are compiled into the product
2. Hand updates ship with OpenFang releases
3. the product owns the canonical set of available Hands

This is good for:

- first-run experience
- stability
- offline/default installs
- CI confidence
- trusted baseline behavior

But it creates several limitations:

- every Hand improvement requires an OpenFang release
- community-contributed Hands are awkward to distribute
- organization-specific/private Hands do not fit naturally
- experimentation is slowed by compile-time bundling
- Hands lag behind Skills as an extensibility surface

### Reference implementation in the local LibreFang fork

The local LibreFang checkout at `/Users/danielalberttis/Desktop/Projects/librefang` provides a useful concrete reference for a different direction.

Relevant evidence:

- [crates/librefang-hands/src/registry.rs](/Users/danielalberttis/Desktop/Projects/librefang/crates/librefang-hands/src/registry.rs) scans external `HAND.toml` definitions from a filesystem registry location under the user home.
- [docs/src/app/architecture/page.mdx](/Users/danielalberttis/Desktop/Projects/librefang/docs/src/app/architecture/page.mdx) documents a registry-backed model under `~/.librefang/registry/hands/`.
- [README.md](/Users/danielalberttis/Desktop/Projects/librefang/README.md) and docs repeatedly present Hands as a broader ecosystem surface, not merely a compile-time bundle.

LibreFang should not be copied blindly. The checked-in code currently demonstrates filesystem-backed Hand discovery, but it does not yet demonstrate the fuller model proposed here:

- no source metadata on loaded Hands
- no explicit bundled/registry/custom precedence model
- no complete install/update/remove lifecycle centered on one durable registry path

Some repository docs also appear ahead of the local checkout reality. So LibreFang is best treated as proof that external Hand loading is viable, not as a full reference implementation of the hybrid model described in this ADR.

### Why a pure registry model is not enough

Moving Hands to registry-only distribution would degrade important OpenFang properties:

- a fresh install would feel empty until registry sync/install
- demos and onboarding would depend on networked distribution
- the product would lose its trusted built-in baseline
- releases would become less self-contained

### Why bundled-only is no longer sufficient

Keeping Hands exclusively bundled means OpenFang cannot treat Hands as a first-class ecosystem surface comparable to Skills.

The right design goal is therefore not replacement, but **layering**.

---

## Decision

OpenFang will evolve Hands toward a **hybrid bundled-plus-registry distribution model**.

The system will continue shipping a curated built-in set of Hands, while also supporting external Hand definitions loaded from a registry-backed filesystem location.

This is an OpenFang design decision inspired in part by LibreFang's filesystem-backed loading direction, but intentionally more comprehensive than LibreFang's current implementation.

The intended result is:

- **bundled Hands** remain the trusted, zero-setup baseline
- **registry Hands** become installable/updatable capability packages
- **custom/user Hands** remain possible as local overrides

### 1. Three source tiers for Hands

OpenFang will support three Hand sources:

| Source | Purpose | Trust posture |
|--------|---------|---------------|
| `bundled` | curated built-in Hands shipped with OpenFang | highest-trust baseline |
| `registry` | installed or synced Hands from an official/private/community registry | medium trust, policy-controlled |
| `custom` | local developer or organization-defined Hands | explicit local trust |

Each loaded Hand must carry source metadata so the UI, CLI, audit log, and policy engine can distinguish them.

### 2. Resolution precedence

Hand resolution precedence will be:

1. `custom`
2. `registry`
3. `bundled`

That means:

- a custom Hand may override a registry Hand with the same `id`
- a registry Hand may override a bundled Hand with the same `id`
- the bundled Hand is used only when no higher-precedence source exists

This preserves a strong built-in default while still allowing targeted overrides and updates.

### 3. Bundled Hands remain mandatory

OpenFang will continue to ship a curated built-in Hand set in the product release.

These built-ins serve four roles:

1. zero-config onboarding
2. product demos and screenshots
3. trusted baseline quality
4. offline/default availability

Registry support is additive. It does not replace the bundled baseline.

### 4. Registry-backed loading model

OpenFang will add a filesystem-backed registry loading layer for Hands, analogous in spirit to how Skills already support installable distribution.

The initial registry location should be under the OpenFang home directory, for example:

```text
~/.openfang/registry/hands/<hand-id>/
  HAND.toml
  SKILL.md
  assets/...
  prompts/...
```

This location is intentionally user-data scoped rather than repository scoped.

The Hands registry will scan this location at startup and on explicit refresh/reload.

### 5. Source metadata becomes part of the Hand model

Each loaded Hand definition should carry metadata similar to:

```rust
pub enum HandSourceKind {
    Bundled,
    Registry,
    Custom,
}

pub struct HandSourceMeta {
    pub source: HandSourceKind,
    pub publisher: Option<String>,
    pub version: Option<String>,
    pub path: Option<PathBuf>,
    pub signature_status: Option<SignatureStatus>,
    pub overrides_hand_id: Option<String>,
}
```

This is necessary for:

- UI badges like `Built-in`, `Registry`, `Custom`
- update notifications
- trust and compatibility decisions
- support/debugging
- auditability

### 6. Hands should become installable like Skills

The Hands system should gain install/update/remove flows similar to the Skills experience, but without forcing the same exact implementation or marketplace semantics.

Initial minimum capability:

- install Hand from local path
- list installed registry Hands
- uninstall registry Hand
- show source and version

Later capability:

- sync from official registry
- update available state
- publisher metadata
- signature verification
- community/private registries

### 7. Compatibility checks are required

Registry Hands must not be treated as opaque arbitrary content.

At minimum, a registry Hand should be able to declare:

- Hand version
- minimum OpenFang version
- optional maximum compatible OpenFang version
- required capabilities/dependencies

OpenFang should reject or warn on incompatible Hands rather than failing deep in execution.

### 8. Trust and policy controls are required

Unlike bundled Hands, registry Hands introduce a supply-chain and policy surface.

Therefore, OpenFang must be able to distinguish:

- official built-ins
- official registry Hands
- third-party/community Hands
- local custom Hands

At maturity, this should support:

- publisher identity
- signature/checksum verification
- allowlists/blocklists
- admin policy on allowed Hand sources

The first implementation does not need the full trust stack, but the model must leave room for it.

---

## Implementation Direction

### Phase 1 — Hybrid loading without marketplace sync

Implement the lowest-risk version first:

1. keep current bundled Hand loading
2. add registry directory scanning under `~/.openfang/registry/hands/`
3. add source metadata
4. implement precedence resolution
5. add CLI/API install-from-path support

This phase alone unlocks:

- local/private Hand distribution
- faster Hand iteration without product release
- a clean override model

### Phase 2 — Registry management surface

Add:

- list/install/remove/update APIs
- dashboard source badges
- dashboard update available state
- explicit refresh/rescan actions

### Phase 3 — Official registry sync

Add:

- official OpenFang Hand registry source
- version discovery
- update checks
- compatibility enforcement

### Phase 4 — Trust hardening

Add:

- signatures/checksums
- publisher verification
- organization policy controls
- audit visibility into install/update events

---

## Why This Decision

### Benefits

#### 1. Hands gain the same ecosystem trajectory as Skills

This closes an architectural inconsistency in OpenFang:

- Skills are extensible packages
- Hands are currently product-bundled only

Hybrid distribution makes Hands feel like a true platform surface.

#### 2. Faster capability iteration

Hand behavior and packaging can evolve independently from OpenFang core releases.

That lowers release coupling and speeds up experimentation.

#### 3. Better fit for organizations

Companies will be able to ship:

- private Hands
- domain-specific Hands
- patched Hands
- environment-specific overrides

without maintaining a fork of OpenFang itself.

#### 4. Built-in baseline remains intact

The user still gets a trustworthy out-of-the-box experience.

This is the main reason hybrid is better than registry-only.

### Costs

#### 1. More loader complexity

The Hands registry must now:

- scan multiple sources
- resolve precedence
- track metadata
- handle compatibility and source state

#### 2. More UI complexity

The dashboard and CLI need to expose:

- source
- version
- trust state
- update state
- install/remove flows

#### 3. Supply-chain concerns

Registry content introduces security and governance requirements that bundled content largely avoids.

That is manageable, but it is real.

---

## Alternatives Considered

### Alternative A — Keep Hands bundled-only

Rejected because it keeps Hands behind the release cadence of OpenFang core and makes the Hand ecosystem permanently weaker than the Skill ecosystem.

### Alternative B — Move Hands fully to registry-only

Rejected because it weakens first-run experience, offline usefulness, and trusted baseline product behavior.

### Alternative C — Treat Hands as just a special kind of Skill

Rejected for now because Hands have product semantics beyond ordinary skill packaging:

- lifecycle management
- activation state
- dashboard metrics
- autonomous role/package semantics

Hands may share packaging patterns with Skills, but they should remain a distinct product concept.

---

## Reference Model To Emulate

The local LibreFang fork is the best concrete OpenFang-adjacent reference currently available for filesystem-loaded Hands behavior.

The specific lessons to emulate are:

1. load Hands from a filesystem-backed registry location
2. parse external `HAND.toml` definitions dynamically
3. treat Hands as installable/distributable artifacts, not only bundled code

The specific things **not** to copy blindly are:

1. doc claims or counts not verified in the repo snapshot
2. any packaging/distribution behavior that assumes a different governance or release model
3. implementation details that conflict with OpenFang's current Hands contracts or UX

LibreFang is therefore a **directional reference**, not a full implementation to mirror mechanically.

---

## Consequences

If adopted, this ADR will lead OpenFang toward:

- a curated built-in Hand baseline
- an installable/updateable Hand ecosystem
- clearer source and trust metadata
- better alignment between Skills and Hands as platform surfaces

It will also establish a clean long-term boundary:

- OpenFang core ships the platform and trusted defaults
- registries ship capability packages

That is the strategic direction this ADR endorses.
