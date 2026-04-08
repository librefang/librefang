# Hermes Agent Comparison for Qwntik Multi-Tenancy

**Status:** Current analysis
**Date:** 2026-04-08
**Scope:** Compare Hermes Agent's architecture and access model against LibreFang's Qwntik multi-tenant target
**Sources:**

- Hermes Architecture: https://hermes-agent.nousresearch.com/docs/developer-guide/architecture/
- Hermes Security: https://hermes-agent.nousresearch.com/docs/user-guide/security/
- Hermes API Server: https://hermes-agent.nousresearch.com/docs/user-guide/features/api-server/
- Hermes Discord / gateway auth model: https://hermes-agent.nousresearch.com/docs/user-guide/messaging/discord/
- Hermes Team Telegram Assistant guide: https://hermes-agent.nousresearch.com/docs/guides/team-telegram-assistant/
- LibreFang target policy: `TENANT-INVARIANTS.md`, `ROUTE-POLICY-MATRIX.md`

---

## Executive Summary

Hermes Agent is a strong reference for:

- self-hosted gateway authorization
- human approval for dangerous actions
- per-user or per-channel conversation partitioning
- safe default deployment guidance for shared bots

Hermes Agent is **not** a strong reference for:

- strict per-request tenant identity
- account-owned resource models
- route-level `404` versus `403` tenancy semantics
- multi-tenant API control planes
- tenant-isolated storage and daemon-global infrastructure separation

Put simply:

- Hermes is a **multi-user agent runtime**
- LibreFang Qwntik target is a **multi-tenant agent platform**

That distinction matters. Hermes answers "who is allowed to talk to this bot?".
LibreFang must answer "which account owns this request, this resource, this
workflow run, this upload, this memory row, and this background task?".

---

## Hermes Architecture: What It Actually Is

Based on the Hermes architecture and feature docs, Hermes centers around:

- one main `AIAgent` execution loop
- local/session persistence in SQLite + JSON logs
- messaging gateways for Discord, Telegram, Signal, etc.
- local or containerized tool execution
- OpenAI-compatible API exposure with one bearer token

This is a coherent design for a powerful self-hosted agent, but it is not
designed as a tenant-aware control plane. Its default posture is closer to:

- one Hermes deployment
- one operator or team
- multiple authorized users
- conversations partitioned by session key or chat context

That is a different problem from Qwntik's target operating model.

---

## How Hermes Handles "Multi-Tenancy"

Hermes does not appear to implement tenant isolation in the Qwntik sense.
Its access model is primarily:

1. authorize users at the gateway edge
2. isolate conversations by user, DM, thread, or channel
3. require approval for dangerous commands
4. recommend Docker or other sandbox backends for safer execution

### What Hermes Has

- Allowlists for who may interact with the bot
- DM pairing for onboarding trusted users on shared bots
- Per-platform session separation
- Shared-room versus per-user conversation behavior depending on platform config
- One bearer token for API server access

### What Hermes Does Not Seem to Model

- `account_id` on every tenant-owned resource
- per-request tenant identity on all non-public routes
- explicit tenant-owned versus admin-only endpoint classes
- tenant-invisible cross-resource lookups
- tenant-scoped background jobs, uploads, memory rows, or artifact registries as a first-class architecture

Hermes' model is closer to "many users, one bot" than "many tenants, one
platform".

---

## Direct Comparison to LibreFang

### Identity Model

Hermes:

- user identity is primarily platform identity or API bearer-token identity
- authorization answers whether a user may interact with the agent

LibreFang Qwntik target:

- identity is an account namespace
- every non-public request must carry `X-Account-Id`
- account signature verification is part of the request contract
- account identity determines visibility and ownership across the platform

Conclusion:

Hermes is closer to authenticated chat access.
LibreFang is closer to account-scoped tenancy.

### Storage Model

Hermes:

- stores sessions and logs locally
- conversation isolation is session-oriented
- documentation emphasizes persistence and chat context, not account ownership

LibreFang:

- must treat agents, sessions, memory, uploads, workflows, media artifacts,
  schedules, and task registries as tenant-owned or explicitly global
- cross-tenant access must disappear behind `404`

Conclusion:

Hermes stores conversation state.
LibreFang must store owned platform state.

### API Model

Hermes:

- OpenAI-compatible API server with one bearer token
- docs explicitly warn that network exposure gives callers access to full toolset

LibreFang:

- API surface is large and heterogenous
- every route family must be classified as public, tenant-owned, admin-only, or split-surface

Conclusion:

Hermes has an authenticated local API.
LibreFang requires a policy-governed tenant API.

### Security Model

Hermes:

- approval prompts for dangerous commands
- container isolation guidance
- user allowlists and pairing
- MCP credential filtering
- context file scanning

LibreFang:

- already has a stronger server-side ownership problem to solve
- must secure not just command execution but also resource classification and namespace integrity

Conclusion:

Hermes is strong on operator safety.
LibreFang must be strong on operator safety **and** tenancy correctness.

---

## What Hermes Gets Right That LibreFang Should Learn From

Hermes is still valuable. Several patterns are worth copying or adapting.

### 1. Explicit Gateway Authorization UX

Hermes documents allowlists and DM pairing clearly and treats them as first-class
operational controls.

Useful LibreFang enhancement:

- formalize channel/user-to-account bootstrap flows for shared messaging adapters
- document how a platform user is mapped to a tenant account
- require explicit tenant binding before channel-originated actions mutate tenant-owned state

Why this matters:

LibreFang is stronger at backend tenancy, but channel-facing identity binding is
still a sharp edge. Hermes treats this edge operationally; LibreFang should treat
it architecturally.

### 2. Better "Shared Bot" Operational Guidance

Hermes docs consistently explain how to run shared bots more safely:

- container backends
- narrow allowlists
- service supervision
- log locations

Useful LibreFang enhancement:

- add Qwntik-specific "shared deployment" operational docs
- define safe defaults for channel execution backends
- document which route families and channel actions are tenant-affecting

### 3. Conversation Scoping as a Configurable Primitive

Hermes makes session partitioning on messaging platforms legible:

- DM session
- thread session
- per-user-in-room session
- optional shared-room session

Useful LibreFang enhancement:

- make channel conversation scope an explicit policy object
- separate conversation scope from account scope
- define when one conversation may act on one tenant versus when rebinding is required

This is important because conversation partitioning and tenant partitioning are
different concerns and should not be conflated.

### 4. Human Approval Model for Dangerous Mutations

Hermes has a clear operator-facing dangerous-command approval model.

Useful LibreFang enhancement:

- extend approvals beyond terminal commands to selected tenant-impacting operations
- examples:
  - cross-system channel rebinds
  - tenant-visible global config mutations
  - plugin or skill lifecycle actions on shared infrastructure
  - webhook or external integration actions with broad scope

### 5. Documentation That Matches the Operating Model

Hermes docs are opinionated about what Hermes is.
LibreFang's multi-tenant fork is only recently becoming that explicit.

Useful LibreFang enhancement:

- keep `TENANT-INVARIANTS.md` as the top-level truth
- remove remaining "legacy mode" framing from target-architecture docs
- avoid describing transitional fallback behavior as if it were a supported design

---

## Where Hermes Is Weaker for Qwntik's Needs

### 1. No First-Class Tenant Namespace

Hermes does not appear to model tenant ownership across all resources.
That makes it unsuitable as a direct blueprint for Qwntik.

LibreFang implication:

- do not import Hermes' "authorized user == correct scope" assumption
- always preserve explicit account-bound ownership checks

### 2. API Auth Is Coarse

A single bearer token for the Hermes API server is acceptable for local trusted
deployments, but it is not enough for a platform serving multiple accounts.

LibreFang implication:

- keep explicit per-request account identity
- keep request signing
- keep route-family policy classification

### 3. Shared Runtime Assumptions

Hermes assumes one runtime where all major capabilities are fundamentally shared
unless session logic says otherwise.

LibreFang implication:

- shared runtime objects must never imply shared tenant visibility
- in-memory registries and temp-file artifacts must be treated as tenant-owned or
  intentionally global

### 4. Session Isolation Is Not Resource Isolation

Hermes is strong at session boundaries.
Qwntik needs system-wide resource boundaries.

LibreFang implication:

- avoid treating per-session separation as sufficient protection
- continue auditing uploads, media tasks, workflow runs, inbox state, hand
  instances, and channel bindings as resource-ownership problems

---

## Concrete Enhancements for LibreFang

The following changes would make LibreFang more complete for Qwntik's
multi-tenant purpose.

### A. Tenant Binding at Channel Ingress

Add a formal tenant-binding layer for incoming channel traffic:

- platform identity -> verified tenant account mapping
- per-channel or per-thread tenant affinity rules
- explicit rejection when channel origin cannot be mapped to a tenant

Why:

- this closes the gap between backend tenancy and gateway identity
- this is where Hermes has operational clarity, even if not true tenancy

### B. Route-Family Policy Annotations in Code

Move the route policy matrix closer to the handlers:

- module-level policy constant or comment block
- tests that assert the expected behavior for missing account, wrong account, and non-admin

Why:

- reduces drift between docs and code
- makes "temporary admin clamp" visible and reviewable

### C. Resource Ownership Audit for All Opaque IDs

Create a standing checklist for any feature that introduces:

- UUID task IDs
- upload IDs
- webhook IDs
- pairing IDs
- session IDs
- run IDs
- browser session IDs

For each, require:

- owner field or explicit global classification
- wrong-tenant `404`
- integration test

Why:

- this is the class of issue most likely to recur

### D. Tenant-Aware Background Work Contract

Formalize how background tasks carry tenant context:

- workflow runs
- media tasks
- cron/schedules
- event/webhook jobs
- hand background loops

Each background execution should record:

- initiating `account_id`
- visibility rules for lookup
- whether outputs are tenant-owned or global

### E. Tenant-Safe Shared Catalog Pattern

For split-surface families such as providers, skills, and hands, codify a
consistent pattern:

- global definitions/catalogs are read-only and intentionally global
- tenant overlays are explicitly tenant-owned
- lifecycle mutations of daemon-global state are admin-only

This avoids bespoke policy per module.

### F. Messaging Scope Versus Tenant Scope Model

Add a dedicated design note separating:

- message/session scope
- agent ownership scope
- tenant scope
- channel membership scope

Why:

- Hermes shows how useful message-scope modeling can be
- LibreFang needs that clarity without weakening tenant guarantees

### G. Tenant Bootstrap and Onboarding Docs

Add an operational guide for:

- how Qwntik provisions admin accounts
- how tenant accounts are created
- how channel identities are linked to accounts
- how account signatures are issued and rotated

This is one of the biggest practical differences between a secure design and a
secure deployment.

---

## Recommended Documentation Follow-Ups

### 1. Add This Comparison to the Multi-Tenant Doc Set

This document should remain in `docs/multi-tenant/` rather than the public
marketing comparison page because it informs architecture, not product marketing.

### 2. Update Public Comparison Messaging Carefully

The public docs comparison page currently focuses on performance and feature
breadth. If tenancy is mentioned there, it should be accurate and restrained.

Suggested public-level language:

- Hermes Agent is optimized for self-hosted multi-user agent access
- LibreFang Qwntik target is optimized for tenant-scoped platform isolation

### 3. Add a "Reference Systems" Section to the Multi-Tenant Plan

Include short notes for:

- Hermes Agent: gateway auth and session partitioning reference
- LangGraph / CrewAI / AutoGen: orchestration patterns only, weak tenancy reference
- true SaaS multi-tenant systems: better reference for resource ownership and policy enforcement

---

## Bottom Line

Hermes Agent is a useful operational and UX reference for shared-agent
deployments, but it should not be treated as the tenancy model for Qwntik.

The right synthesis is:

- learn from Hermes on gateway authorization, pairing, session scoping, and safe
  self-hosted operations
- keep LibreFang's stricter account/resource isolation model
- explicitly build the missing bridge between channel ingress identity and
  backend tenant ownership

If LibreFang does that well, it will occupy a stronger position than Hermes for
real multi-tenant agent infrastructure: not just "many users can talk to one
agent", but "many accounts can safely operate on one platform without scope
confusion or resource leakage".
