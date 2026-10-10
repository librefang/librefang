import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import {
  emptyManifestExtras,
  emptyManifestForm,
  MODEL_MODES,
  serializeManifestForm,
  type ManifestFormState,
} from "../agentManifest";
import {
  applyRoutingEngine,
  ROUTING_ENGINES,
  ROUTING_ENGINE_SETTINGS,
  ROUTING_TIER_DEFAULTS,
  routingEngineOf,
  routingTierModel,
} from "../routingEngine";

// The engine selector is a mapping from one operator choice onto three manifest
// fields, and every cell of that mapping is a claim about what the kernel does
// with those fields. The claims live in Rust source this test reads at test
// time, in the same spirit as `manifest-field-coverage.test.ts` reading
// `AgentManifest`: a hand-copied restatement of the rule goes stale the moment
// upstream renames a field or reorders a branch, and — worse than stale — it
// stays green while the form writes a state the kernel now resolves
// differently. Parsing the real code fails closed when any anchor moves.
//
// Four anchors, one per row of the table in `lib/routingEngine.ts`:
//
//   - `ModelMode`'s serialised spellings (agent.rs), which is what `[model]
//     mode = …` has to contain;
//   - `AgentRouterOverride::fixed` (model_profile.rs) and the `Bypassed` arm it
//     produces (model_router.rs), which is why the profile engine writes it
//     false and both other engines write it true;
//   - `AgentManifest::routing: Option<ModelRoutingConfig>` (agent.rs), whose
//     presence is what arms the tier router;
//   - the precedence in `model_selection_path` and the `Flexible` gate in
//     `route_to_profile` (agent_execution.rs), which is *why* each engine sets
//     the mode it does.

const CRATES = join(__dirname, "..", "..", "..", "..", "..");
const TYPES_SRC = join(CRATES, "librefang-types", "src");

const read = (...parts: string[]): string =>
  readFileSync(join(...parts), "utf8");

const AGENT_RS = read(TYPES_SRC, "agent.rs");
const MODEL_PROFILE_RS = read(TYPES_SRC, "model_profile.rs");
const KERNEL_SRC = join(CRATES, "librefang-kernel", "src");
const AGENT_EXECUTION_RS = read(KERNEL_SRC, "kernel", "agent_execution.rs");
const MODEL_ROUTER_RS = read(KERNEL_SRC, "model_router.rs");
const SPAWN_RS = read(CRATES, "librefang-runtime", "src", "tool_runner", "agent.rs");
const CATALOG_QUERY_RS = read(CRATES, "librefang-kernel-handle", "src", "catalog_query.rs");

/**
 * The serialised spellings of a Rust enum, honouring its `rename_all` — the
 * subset `manifest-field-coverage.test.ts` already applies to the enums the
 * form writes. Duplicated rather than shared because the two tests must fail
 * for their own reasons: this one is about the routing engines, and a helper
 * that drifts between them would be a third thing to keep right.
 */
function rustEnumSpellings(source: string, name: string): string[] {
  const m = new RegExp(`pub enum ${name}\\b[^{]*\\{([^}]*)\\}`).exec(source);
  if (!m) throw new Error(`enum ${name} not found`);
  const attrs = source.slice(Math.max(0, m.index - 400), m.index);
  const rename = /rename_all\s*=\s*"([^"]+)"/.exec(attrs);
  const rule = rename ? rename[1] : "none";
  return [...m[1].matchAll(/^\s*(?:#\[[^\]]*\]\s*)?([A-Z]\w*)/gm)]
    .map((v) => v[1])
    .map((variant) => {
      if (rule === "lowercase") return variant.toLowerCase();
      if (rule === "snake_case") {
        return variant
          .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
          .replace(/([A-Z])([A-Z][a-z])/g, "$1_$2")
          .toLowerCase();
      }
      return variant;
    });
}

/**
 * `impl Default for ModelRoutingConfig`'s field values, as Rust spells them.
 *
 * Read rather than restated for the reason the rest of this file gives: the
 * numbers and model names are the daemon's, and a bumped snapshot upstream
 * must fail here rather than leave the editor describing a model the kernel
 * stopped using.
 */
function modelRoutingDefaults(source: string): Record<string, string | number> {
  const block =
    /impl Default for ModelRoutingConfig \{[\s\S]*?fn default\(\) -> Self \{\s*Self \{([\s\S]*?)\n\s*\}/.exec(
      source,
    );
  if (!block) throw new Error("impl Default for ModelRoutingConfig not found");

  const out: Record<string, string | number> = {};
  for (const line of block[1].split("\n")) {
    const text = /^\s*(\w+):\s*"([^"]*)"\.to_string\(\),/.exec(line);
    if (text) {
      out[text[1]] = text[2];
      continue;
    }
    const number = /^\s*(\w+):\s*(\d+),/.exec(line);
    if (number) out[number[1]] = Number(number[2]);
  }
  return out;
}

/**
 * The body of `fn <name>`, by walking its braces.
 *
 * Walking rather than matching `\n}` because `route_to_profile` is a method
 * inside an `impl` block: its closing brace is indented, so a regex that looks
 * for a brace in column zero runs past the end of the function and keeps
 * reading the rest of the impl — which would let an assertion about the gate
 * be satisfied by some other function's text.
 */
function rustFunctionBody(source: string, name: string): string {
  const start = new RegExp(`fn ${name}\\b`).exec(source);
  if (!start) throw new Error(`fn ${name} not found`);
  const open = source.indexOf("{", start.index);
  let depth = 0;
  for (let i = open; i < source.length; i++) {
    if (source[i] === "{") depth++;
    else if (source[i] === "}") {
      depth--;
      if (depth === 0) return source.slice(start.index, i + 1);
    }
  }
  throw new Error(`fn ${name} has no closing brace`);
}

describe("the routing engine mapping is anchored to the kernel's own rules", () => {
  it("spells `model.mode` the way `ModelMode` serialises, and covers every variant", () => {
    const spellings = rustEnumSpellings(AGENT_RS, "ModelMode");
    // A third variant would mean a routing state this table cannot name, so the
    // engine list has to be re-derived rather than silently under-described.
    expect(spellings, "ModelMode grew or lost a variant").toHaveLength(2);
    const [fixed, flexible] = spellings;

    expect(ROUTING_ENGINE_SETTINGS.fixed.mode).toBe(fixed);
    expect(ROUTING_ENGINE_SETTINGS.effort.mode).toBe(fixed);
    expect(ROUTING_ENGINE_SETTINGS.profile.mode).toBe(flexible);
    // The form's own copy of the spellings, which the serializer writes
    // verbatim — the two must not drift apart.
    expect([...MODEL_MODES]).toEqual(spellings);
  });

  it("only ever clears `router_fixed`, which is a capability as well as a routing flag", () => {
    // The field the form writes as `router_override = { fixed = true }`.
    expect(MODEL_PROFILE_RS).toMatch(
      /pub struct AgentRouterOverride \{[\s\S]*?pub fixed: bool,/,
    );
    // The routing half: an agent whose override is fixed is Bypassed *before*
    // any profile is matched, which is why the profile engine has to clear it —
    // otherwise choosing that engine would do nothing.
    expect(MODEL_ROUTER_RS).toMatch(
      /if !config\.enabled \|\| agent_override\.is_some_and\(\|o\| o\.fixed\)/,
    );

    // The half that makes writing `true` wrong for the other engines: the
    // spawn path refuses a profile for an agent pinned with this flag, and it
    // never looks at `mode`. Two files carry that: the gate that consults the
    // pin (`check_profile_against_parent` in tool_runner/agent.rs, reached
    // through `gate_spawn_profile`) and the accessor the gate gets the
    // override from (`model_router_override_for` in
    // kernel-handle/src/catalog_query.rs, whose default impl returns `None`
    // and whose kernel impl reads the manifest). An engine that set the pin
    // would revoke a capability the operator never touched.
    const spawnGate = rustFunctionBody(SPAWN_RS, "check_profile_against_parent");
    expect(
      spawnGate,
      "The spawn gate no longer reads the parent's `fixed` pin. If it stopped, " +
        "the reason the non-profile engines must not write `true` is gone and " +
        "this table has to be re-derived.",
    ).toMatch(/override_\.fixed/);
    expect(spawnGate).not.toMatch(/\.mode\b/);
    expect(
      CATALOG_QUERY_RS,
      "The kernel-handle accessor the spawn gate fetches the override through " +
        "is gone, so nothing carries the pin to the spawn path any more.",
    ).toMatch(/fn model_router_override_for\(/);
    expect(SPAWN_RS).toMatch(/fn gate_spawn_profile\([\s\S]*?check_profile_against_parent\(/);

    expect(ROUTING_ENGINE_SETTINGS.fixed.router_fixed).toBe("unchanged");
    expect(ROUTING_ENGINE_SETTINGS.effort.router_fixed).toBe("unchanged");
    expect(ROUTING_ENGINE_SETTINGS.profile.router_fixed).toBe(false);
    // No row may ever write the pin on.
    for (const engine of ROUTING_ENGINES) {
      expect(ROUTING_ENGINE_SETTINGS[engine].router_fixed).not.toBe(true);
    }
  });

  it("arms the tier router by the presence of `AgentManifest::routing`", () => {
    expect(AGENT_RS).toMatch(/pub routing: Option<ModelRoutingConfig>,/);

    // The table's presence is the tier router's whole configuration, so the
    // engine that must not speak for the tiers says "unchanged" rather than
    // `false`: writing `false` would delete the block — and with it the
    // fallback the kernel runs when no profile matches.
    expect(ROUTING_ENGINE_SETTINGS.fixed.routing_table).toBe(false);
    expect(ROUTING_ENGINE_SETTINGS.effort.routing_table).toBe(true);
    expect(ROUTING_ENGINE_SETTINGS.profile.routing_table).toBe("unchanged");
  });

  it("gates profile routing on the flexible mode the profile engine writes", () => {
    const body = rustFunctionBody(AGENT_EXECUTION_RS, "route_to_profile");
    // The reader stops at this function's own closing brace: the next item in
    // the impl must not be part of what these assertions read.
    expect(body).not.toContain("fn execute_llm_agent");
    expect(
      body,
      "route_to_profile no longer declines a non-flexible manifest, so the " +
        "profile engine's mode is no longer what decides between the two routers.",
    ).toMatch(/manifest\.model\.mode != ModelMode::Flexible/);
    expect(body).toMatch(/!cfg\.model_router\.enabled/);
  });

  it("resolves profile before tiers, in that order", () => {
    const body = rustFunctionBody(AGENT_EXECUTION_RS, "model_selection_path");
    const profile = body.indexOf("ModelSelectionPath::Profile");
    const tier = body.indexOf("ModelSelectionPath::Tier");
    expect(profile).toBeGreaterThan(-1);
    expect(tier).toBeGreaterThan(-1);
    expect(
      profile < tier,
      "The profile router is no longer resolved before the tier router. Every " +
        "cell of the engine table assumes it wins when it applies: the effort " +
        "engine relies on being ineligible for it, not on coming first, and " +
        "the profile engine relies on the tiers running when no profile " +
        "matches — which is why it leaves the table alone instead of removing it.",
    ).toBe(true);

    // The call site passes the two candidates positionally, so their order is
    // part of the contract too.
    expect(AGENT_EXECUTION_RS).toMatch(
      /model_selection_path\(\s*is_stable,\s*routed_profile\.is_some\(\),\s*tier_routing_config\.is_some\(\),?\s*\)/,
    );
  });
});

// The guards above are only worth what their readers are worth: a reader that
// returned a happy-path constant would keep every anchor green no matter what
// upstream did. These two prove the readers act on the text handed to them.
describe("the Rust readers read the source they are given", () => {
  it("applies the enum's own rename rule to whatever variants are there", () => {
    const synthetic = `
#[derive(Debug)]
#[serde(rename_all = "lowercase")]
pub enum ModelMode {
    /// A doc comment naming ModelSelectionPath::Profile, which is not a variant.
    #[default]
    Fixed,
    Flexible,
}
`;
    expect(rustEnumSpellings(synthetic, "ModelMode")).toEqual(["fixed", "flexible"]);
    expect(rustEnumSpellings(synthetic.replace("Flexible", "Adaptive"), "ModelMode")).toEqual([
      "fixed",
      "adaptive",
    ]);
  });

  it("stops a function body at that function's closing brace", () => {
    const synthetic = `
pub(crate) fn model_selection_path() -> X {
    if a {
        ModelSelectionPath::Profile
    }
}

fn execute_llm_agent() {
    ModelSelectionPath::Tier
}
`;
    const body = rustFunctionBody(synthetic, "model_selection_path");
    expect(body).toContain("ModelSelectionPath::Profile");
    expect(body).not.toContain("ModelSelectionPath::Tier");
  });
});

describe("routingEngineOf classifies a manifest the way the kernel resolves it", () => {
  const state = (
    mode: ManifestFormState["model"]["mode"],
    router_fixed: boolean,
    routing_enabled: boolean,
  ): ManifestFormState => {
    const form = emptyManifestForm();
    form.model = { ...form.model, mode, router_fixed };
    form.routing = { ...form.routing, enabled: routing_enabled };
    return form;
  };

  // Every combination of the three fields, and the engine the kernel resolves
  // it to. `flexible` with the override on is not profile routing — the
  // `Bypassed` arm above stops it before a profile is even matched — so it
  // classifies by what is left.
  const CASES: Array<[ManifestFormState["model"]["mode"], boolean, boolean, string]> = [
    ["fixed", false, false, "fixed"],
    ["fixed", false, true, "effort"],
    ["fixed", true, false, "fixed"],
    ["fixed", true, true, "effort"],
    ["flexible", false, false, "profile"],
    ["flexible", false, true, "profile"],
    ["flexible", true, false, "fixed"],
    ["flexible", true, true, "effort"],
  ];

  it.each(CASES)(
    "mode=%s router_fixed=%s routing.enabled=%s is the %s engine",
    (mode, router_fixed, routing_enabled, expected) => {
      expect(routingEngineOf(state(mode, router_fixed, routing_enabled))).toBe(expected);
    },
  );

  it("covers every combination of the three fields", () => {
    const combinations = new Set(
      CASES.map(([mode, fixed, routing]) => `${mode}|${fixed}|${routing}`),
    );
    expect(combinations.size).toBe(CASES.length);
    expect(CASES).toHaveLength(2 * 2 * 2);
  });

  it("never names an engine outside ROUTING_ENGINES", () => {
    for (const [mode, router_fixed, routing_enabled] of CASES) {
      expect(ROUTING_ENGINES).toContain(
        routingEngineOf(state(mode, router_fixed, routing_enabled)),
      );
    }
  });
});

describe("applyRoutingEngine writes the documented triple", () => {
  it.each([
    ["fixed", "fixed", "unchanged", false],
    ["effort", "fixed", "unchanged", true],
    ["profile", "flexible", false, "unchanged"],
  ] as const)(
    "%s sets mode=%s, leaves router_fixed %s and the tier table %s",
    (engine, mode, router_fixed, routing_table) => {
      const from = emptyManifestForm();
      const next = applyRoutingEngine(from, engine);
      expect(next.model.mode).toBe(mode);
      if (router_fixed === "unchanged") {
        expect(next.model.router_fixed).toBe(from.model.router_fixed);
      } else {
        expect(next.model.router_fixed).toBe(router_fixed);
      }
      if (routing_table === "unchanged") {
        // Identity, not just equality: "the engine makes no statement about
        // the tiers" has to mean the very object is carried over, or a later
        // edit to the copy could still diverge from what is on disk.
        expect(next.routing).toBe(from.routing);
      } else {
        expect(next.routing.enabled).toBe(routing_table);
      }
    },
  );

  it("does not revoke a pin the operator set elsewhere when the engine changes", () => {
    // `fixed = true` is not a routing preference: it also refuses every profile
    // to the agents this one spawns, and the spawn gate reads it without
    // consulting `mode` (see the guard above). Choosing an engine must not
    // touch it, which is the whole reason no row writes `true`.
    const pinned = emptyManifestForm();
    pinned.model.router_fixed = true;

    for (const engine of ["fixed", "effort"] as const) {
      const next = applyRoutingEngine(pinned, engine);
      expect(next.model.router_fixed, `${engine} cleared the pin`).toBe(true);
      expect(serializeManifestForm(next)).toContain("router_override = { fixed = true }");
    }

    // The one engine that has to speak about the pin, and only to clear it: a
    // pinned agent cannot route by profile at all, so the choice would be
    // dead without it.
    const asProfile = applyRoutingEngine(pinned, "profile");
    expect(asProfile.model.router_fixed).toBe(false);
    expect(serializeManifestForm(asProfile)).not.toContain("router_override");
  });

  it("carries a `[routing]` table the form did not write straight through", () => {
    // An agent that arrived with tiers: the profile engine must not be read as
    // an instruction about them, in either direction.
    const from = emptyManifestForm();
    from.routing = { ...from.routing, enabled: true, simple_model: "haiku" };
    const next = applyRoutingEngine(from, "profile");
    expect(next.routing).toBe(from.routing);
    expect(next.routing.enabled).toBe(true);
    expect(routingEngineOf(next)).toBe("profile");
  });

  it.each([...ROUTING_ENGINES])("round-trips %s through the classifier", (engine) => {
    expect(routingEngineOf(applyRoutingEngine(emptyManifestForm(), engine))).toBe(engine);
  });

  it("leaves the other engine's settings alone, so switching back loses nothing", () => {
    const form = emptyManifestForm();
    form.routing = {
      ...form.routing,
      simple_model: "haiku",
      complex_model: "opus",
      complex_threshold: "900",
    };
    form.model = {
      ...form.model,
      router_allowed_profiles: ["coding"],
      router_cost_budget: "cheap",
      router_default_profile: "research",
    };

    const asProfile = applyRoutingEngine(form, "profile");
    expect(asProfile.routing.simple_model).toBe("haiku");
    expect(asProfile.model.router_allowed_profiles).toEqual(["coding"]);

    const backToEffort = applyRoutingEngine(asProfile, "effort");
    expect(backToEffort.routing.complex_model).toBe("opus");
    expect(backToEffort.routing.complex_threshold).toBe("900");
    expect(backToEffort.model.router_cost_budget).toBe("cheap");
    expect(backToEffort.model.router_default_profile).toBe("research");
  });

  it("does not touch the fields it does not own", () => {
    const form = emptyManifestForm();
    form.name = "router-test";
    form.reconcile_orphans = "keep";
    form.model.provider = "openai";

    const next = applyRoutingEngine(form, "profile");
    expect(next.name).toBe("router-test");
    expect(next.reconcile_orphans).toBe("keep");
    expect(next.model.provider).toBe("openai");
  });
});

// The tier table is not a switch the engine flips; it is the block the
// serializer writes when `routing.enabled` is true, and the fallback the kernel
// runs when the profile router matches nothing. Choosing "profile" therefore
// has to leave it alone on disk — including the keys this form has no widget
// for, which reach it only as `preserved` extras and would go with the block.
// Asserted against the serialized TOML rather than the form state because the
// file is what the daemon reads, and because a state-level assertion would not
// have caught the `extras` half.
describe("changing to the profile engine never drops the tier table", () => {
  /** A form as the effort engine leaves it, with tiers nobody would call defaults. */
  function withTiers(): ManifestFormState {
    const form = emptyManifestForm();
    form.routing = {
      enabled: true,
      simple_model: "gpt-4o-mini",
      medium_model: "gpt-4o",
      complex_model: "claude-opus-5",
      simple_threshold: "120",
      complex_threshold: "900",
    };
    return form;
  }

  it("keeps `[routing]` and every tier in the TOML", () => {
    const toml = serializeManifestForm(applyRoutingEngine(withTiers(), "profile"));

    expect(toml).toContain("[routing]");
    expect(toml).toContain('simple_model = "gpt-4o-mini"');
    expect(toml).toContain('medium_model = "gpt-4o"');
    expect(toml).toContain('complex_model = "claude-opus-5"');
    expect(toml).toContain("simple_threshold = 120");
    expect(toml).toContain("complex_threshold = 900");
  });

  it("keeps the `[routing]` keys the form has no widget for", () => {
    const extras = emptyManifestExtras();
    extras.routing = { block_stall_degrade_after: 7 };

    const toml = serializeManifestForm(applyRoutingEngine(withTiers(), "profile"), extras);

    expect(toml).toContain("block_stall_degrade_after = 7");
  });

  it("does not invent a `[routing]` table for an agent that never had one", () => {
    // The other direction: a manifest with no tiers must not come back with a
    // block built from this form's own defaults, which would arm a router the
    // operator never asked for.
    const toml = serializeManifestForm(applyRoutingEngine(emptyManifestForm(), "profile"));

    expect(toml).not.toContain("[routing]");
  });

  it("arms the router on the daemon's models when the table is saved blank", () => {
    // The state the editor has to describe rather than hide: the effort engine
    // writes `[routing]` for a form whose slots are all empty, and the daemon
    // reads the table's presence as "route", filling every missing key from
    // `ModelRoutingConfig::default()`.
    const toml = serializeManifestForm(applyRoutingEngine(emptyManifestForm(), "effort"));

    expect(toml).toContain("[routing]");
    expect(toml).not.toContain("simple_model");
    expect(routingTierModel(emptyManifestForm(), "simple_model")).toBe(
      ROUTING_TIER_DEFAULTS.simple_model,
    );
  });

  it("still removes the table when an engine without tiers is chosen outright", () => {
    // "Fixed" is an explicit statement about routing, and `false` is how it is
    // made; only "profile" declines to make one.
    const toml = serializeManifestForm(applyRoutingEngine(withTiers(), "fixed"));

    expect(toml).not.toContain("[routing]");
  });
});

// The editor names the daemon's fallbacks in two places — the effort engine's
// blank-slot line and the profile engine's unmatched-turn line — so the names
// have to be the daemon's own, not this form's idea of them. A fresh agent's
// tier slots are all empty and the `[routing]` table arms the router anyway,
// so a stale name here would describe a model the kernel stopped using.
describe("the tier defaults the editor names are the daemon's", () => {
  it("mirrors `ModelRoutingConfig::default()` field for field", () => {
    const defaults = modelRoutingDefaults(AGENT_RS);

    expect(Object.keys(defaults).sort()).toEqual(Object.keys(ROUTING_TIER_DEFAULTS).sort());
    for (const [key, value] of Object.entries(defaults)) {
      expect(
        ROUTING_TIER_DEFAULTS[key as keyof typeof ROUTING_TIER_DEFAULTS],
        `${key} has drifted from ModelRoutingConfig::default(), which is what the ` +
          `daemon routes on when the manifest leaves the key out.`,
      ).toBe(value);
    }
  });

  it("the defaults reader reads the source it is given", () => {
    const synthetic = `
impl Default for ModelRoutingConfig {
    fn default() -> Self {
        Self {
            simple_model: "one".to_string(),
            simple_threshold: 7,
        }
    }
}
`;
    expect(modelRoutingDefaults(synthetic)).toEqual({
      simple_model: "one",
      simple_threshold: 7,
    });
    expect(modelRoutingDefaults(synthetic.replace('"one"', '"two"'))).toEqual({
      simple_model: "two",
      simple_threshold: 7,
    });
  });

  it("resolves a slot to the default only when the manifest is blank", () => {
    const form = emptyManifestForm();
    expect(routingTierModel(form, "simple_model")).toBe(ROUTING_TIER_DEFAULTS.simple_model);

    form.routing.simple_model = "gpt-4o-mini";
    expect(routingTierModel(form, "simple_model")).toBe("gpt-4o-mini");

    // Whitespace is the absent key as far as the serializer is concerned: it
    // trims before deciding whether to write the line.
    form.routing.simple_model = "   ";
    expect(routingTierModel(form, "simple_model")).toBe(ROUTING_TIER_DEFAULTS.simple_model);
  });
});
