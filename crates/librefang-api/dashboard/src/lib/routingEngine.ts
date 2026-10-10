import type { ManifestFormState } from "./agentManifest";

/**
 * The routing engine an agent runs on: one choice, and the fields that have to
 * move together to make it the engine the kernel resolves.
 *
 * The kernel has two routers and they are not a merge of settings — they are
 * ordered, and the first that applies takes the turn
 * (`model_selection_path` in
 * crates/librefang-kernel/src/kernel/agent_execution.rs resolves
 * Stable → Profile → Tier → the manifest's own model). Read as loose controls
 * they describe engines other than the ones their names suggest: a manifest
 * saying `mode = "flexible"` with the router override on runs the *tiers*,
 * because the override bypasses the profile router before any profile is
 * matched, and one with no `[routing]` table runs neither router. The engine
 * writes the fields it decides together and leaves the rest alone, so what
 * lands on disk is always a state the kernel resolves to the engine named
 * here — and never a capability the operator did not ask to change.
 *
 * | engine  | `model.mode` | `model.router_fixed`      | `[routing]` table      |
 * | ------- | ------------ | ------------------------- | ---------------------- |
 * | fixed   | `fixed`      | left alone                | removed                |
 * | effort  | `fixed`      | left alone                | written                |
 * | profile | `flexible`   | `false`                   | left exactly as it was |
 *
 * Why each cell, because the table is the whole design:
 *
 * - **effort** sets `mode = "fixed"` because `route_to_profile` returns `None`
 *   unless the manifest is `ModelMode::Flexible`; leaving it flexible would
 *   hand every turn to the profile router and the three tiers would decide
 *   nothing.
 * - **profile** does not touch the tier table, in either direction. The two
 *   routers are a chain rather than alternatives: `model_selection_path`
 *   resolves Profile before Tier, so on a flexible manifest a turn that
 *   matches a profile is routed by it and a turn that matches none falls
 *   through to the tiers. Writing `false` there would therefore not switch an
 *   engine off — it would delete the fallback, because `routing.enabled` *is*
 *   the table's presence in the file: `serializeManifestForm` writes the whole
 *   `[routing]` block or none of it, so the three models, the two thresholds
 *   and every key of the table the form has no widget for would go with it.
 *   Leaving it alone is the only cell that is honest in both directions: an
 *   agent that had tiers keeps them, and one that never had a `[routing]`
 *   table does not get one invented from this form's own defaults.
 * - **router_fixed is written by the profile engine alone, and never to
 *   `true`.** The pin is not a routing flag with a routing meaning: `true`
 *   opts the agent out of profiles for its own turns *and* refuses every
 *   profile to the agents it spawns. The spawn path consults it in
 *   `check_profile_against_parent`
 *   (crates/librefang-runtime/src/tool_runner/agent.rs) without looking at
 *   `mode` at all, and the override it consults is fetched from the kernel
 *   through `model_router_override_for`
 *   (crates/librefang-kernel-handle/src/catalog_query.rs). An engine writing
 *   `true` would revoke a capability the operator never touched, which is why
 *   neither non-profile engine writes it: they do not need it, because
 *   `mode = "fixed"` is already what keeps the profile router off. The
 *   profile engine writes `false` because a pinned agent cannot route by
 *   profile, so the choice would otherwise do nothing at all.
 *
 * The three values are the serialised spellings, not the Rust variant names:
 * `ModelMode` is `rename_all = "snake_case"`, so `Fixed` reaches the file as
 * `fixed`.
 */
export const ROUTING_ENGINES = ["fixed", "effort", "profile"] as const;

export type RoutingEngine = (typeof ROUTING_ENGINES)[number];

/**
 * English names for the engines, for surfaces that are not localised.
 *
 * The editor reads its own translated keys; this is for the markdown summary,
 * which is a document rather than a screen. The `Record` is what keeps the set
 * honest: an engine with no name here does not compile, so this list cannot
 * quietly fall behind `ROUTING_ENGINES`.
 */
export const ROUTING_ENGINE_LABELS: Record<RoutingEngine, string> = {
  fixed: "Fixed model",
  effort: "Effort (complexity)",
  profile: "Profile router",
};

/** The manifest fields the engine decides, under the names the file format uses. */
export interface RoutingEngineSetting {
  /** `[model] mode` — `ModelMode` in crates/librefang-types/src/agent.rs. */
  mode: ManifestFormState["model"]["mode"];
  /**
   * What the engine does to `[model] router_override.fixed` —
   * `AgentRouterOverride::fixed` in crates/librefang-types/src/model_profile.rs.
   *
   * `"unchanged"` leaves the pin exactly as the manifest had it; a boolean
   * writes it. No engine writes `true`: the pin is not a routing knob but a
   * capability, refusing profiles both to this agent's own turns and to the
   * agents it spawns, so switching engines must not set it. Only the profile
   * engine writes anything, and only `false` — a pinned agent cannot route by
   * profile at all, and the choice would be a dead control without that.
   */
  router_fixed: boolean | "unchanged";
  /**
   * What the engine does to the `[routing]` table, which is the tier router's
   * whole configuration.
   *
   * `true` and `false` are the value written into `form.routing.enabled` — the
   * flag the serializer reads to emit the block or drop it. `"unchanged"`
   * means the engine makes no statement about the tiers at all and the table
   * is carried over as it was; that is not the same as `false`, which deletes
   * it. The Rust field is `AgentManifest::routing: Option<ModelRoutingConfig>`,
   * and the table's presence is what arms the tier router for this agent.
   */
  routing_table: boolean | "unchanged";
}

export const ROUTING_ENGINE_SETTINGS: Record<RoutingEngine, RoutingEngineSetting> = {
  fixed: { mode: "fixed", router_fixed: "unchanged", routing_table: false },
  effort: { mode: "fixed", router_fixed: "unchanged", routing_table: true },
  profile: { mode: "flexible", router_fixed: false, routing_table: "unchanged" },
};

/**
 * The engine a form state describes, read through the same rules the kernel
 * applies — not through the last control the operator touched.
 *
 * A manifest that reaches the editor from anywhere else (the API, a hand-edit,
 * a template) has no engine field to read, so it is classified by what the
 * kernel would do with it: `router_fixed` is checked with `mode` rather than
 * after it, because the override bypasses the profile router even in
 * `flexible` mode — a manifest carrying both is not profile routing, it is the
 * tier engine with the profile router disarmed, or no routing at all.
 *
 * The pin means a second thing — it refuses profiles to the agents this one
 * spawns — and that is not a routing decision, so this classifier does not
 * read it as one. A pinned agent keeps its engine; it just cannot spawn onto
 * a profile.
 */
export function routingEngineOf(form: ManifestFormState): RoutingEngine {
  if (form.model.mode === "flexible" && !form.model.router_fixed) return "profile";
  return form.routing.enabled ? "effort" : "fixed";
}

/**
 * The form state `engine` describes. Every field outside the ones it decides
 * is carried over untouched.
 *
 * The tier models, the thresholds and the profile settings all survive a
 * switch: they belong to the engine being switched away from, and clearing
 * them would turn a look at the other engine into data loss. Under the profile
 * engine that is not a courtesy but the design — the tier table is the
 * fallback the kernel runs when no profile matches, so it is left exactly as
 * it was (see `routing_table`) rather than rewritten, and the same reason is
 * why the serializer preserves fixed-mode router overrides instead of reading
 * a missing key as a decision.
 *
 * `router_fixed` follows the same rule for the same kind of reason: it is a
 * capability, not a routing preference (see `RoutingEngineSetting`), so both
 * engines that do not need it leave it exactly as they found it.
 */
export function applyRoutingEngine(
  form: ManifestFormState,
  engine: RoutingEngine,
): ManifestFormState {
  const setting = ROUTING_ENGINE_SETTINGS[engine];
  return {
    ...form,
    model: {
      ...form.model,
      mode: setting.mode,
      ...(setting.router_fixed === "unchanged"
        ? {}
        : { router_fixed: setting.router_fixed }),
    },
    routing:
      setting.routing_table === "unchanged"
        ? form.routing
        : { ...form.routing, enabled: setting.routing_table },
  };
}

/** The tier slots that name a model, as `[routing]` spells them. */
export type RoutingTierModelSlot = "simple_model" | "medium_model" | "complex_model";

/**
 * `impl Default for ModelRoutingConfig` — what the daemon substitutes for
 * every `[routing]` key the manifest leaves out
 * (crates/librefang-types/src/agent.rs).
 *
 * These are not decoration: the struct is `#[serde(default)]`, so a
 * `[routing]` table written with no keys at all still arms the tier router,
 * on these three models and these two thresholds. An editor that showed three
 * blank pickers and said nothing would be describing an agent that does not
 * exist — the operator would believe the tiers were unconfigured while every
 * turn was being routed onto `claude-haiku-4-5-20251001`.
 *
 * `routing-engine.test.ts` reads the Rust `Default` at test time and fails
 * when these drift, so a bumped snapshot model upstream is a test failure
 * rather than a stale name on screen.
 */
export const ROUTING_TIER_DEFAULTS = {
  simple_model: "claude-haiku-4-5-20251001",
  medium_model: "claude-sonnet-4-20250514",
  complex_model: "claude-sonnet-4-20250514",
  simple_threshold: 100,
  complex_threshold: 500,
} as const;

/**
 * The model a tier slot will actually route to: what the manifest says, or the
 * daemon's own default for that slot when it says nothing.
 *
 * The empty string is the absent key — `serializeManifestForm` omits it — and
 * it is *not* "no model": serde fills it. Callers that want to show an
 * operator what the slot resolves to have to show this, not the raw field.
 */
export function routingTierModel(
  form: ManifestFormState,
  slot: RoutingTierModelSlot,
): string {
  return form.routing[slot].trim() || ROUTING_TIER_DEFAULTS[slot];
}
