import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
  FORM_TOP_LEVEL_KEYS,
  emptyManifestExtras,
  emptyManifestForm,
  parseManifestToml,
  preservedWorkspaceNamesFromExtras,
  serializeManifestForm,
  validateManifestForm,
  type ManifestFormState,
} from "./agentManifest";

describe("agentManifest serializer", () => {
  it("renders the minimum viable manifest", () => {
    const form = emptyManifestForm();
    form.name = "researcher";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";

    const toml = serializeManifestForm(form);

    expect(toml).toContain('name = "researcher"');
    expect(toml).toContain('module = "builtin:chat"');
    expect(toml).toContain("[model]");
    expect(toml).toContain('provider = "openai"');
    expect(toml).toContain('model = "gpt-4o"');
    expect(toml).not.toContain("[resources]");
    expect(toml).not.toContain("[capabilities]");
  });

  it("escapes special characters in strings", () => {
    const form = emptyManifestForm();
    form.name = "spy";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    form.description = 'has "quotes" and a \\backslash';
    form.model.system_prompt = "Line 1\nLine 2";

    const toml = serializeManifestForm(form);

    expect(toml).toContain('description = "has \\"quotes\\" and a \\\\backslash"');
    expect(toml).toContain('system_prompt = "Line 1\\nLine 2"');
  });

  it("round-trips TOML control characters in strings", () => {
    const form = emptyManifestForm();
    form.name = "control-characters";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    form.model.system_prompt = "prefix\r\t\0\b\v\f\u001f\u007fsuffix";

    const toml = serializeManifestForm(form);
    const parsed = parseManifestToml(toml);

    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.model.system_prompt).toBe(form.model.system_prompt);
  });

  // #8028: `system_prompt` is not tri-state like the sampling knobs — a
  // blank value means "this agent has no system prompt", not "no opinion".
  // Routing it through the generic skip-if-empty writer dropped the key on
  // an intentionally blank prompt, and the server's `#[serde(default)]`
  // then filled the missing key with the canned default text on the very
  // next save. The key must always be emitted, even empty, so a blank
  // prompt round-trips as blank rather than acquiring text the operator
  // never asked for.
  it("writes system_prompt through even when blank, rather than omitting the key", () => {
    const form = emptyManifestForm();
    form.name = "blank-prompt";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    form.model.system_prompt = "";

    const toml = serializeManifestForm(form);

    expect(toml).toContain('system_prompt = ""');
  });

  it("preserves Unicode scalars and replaces isolated UTF-16 surrogates", () => {
    const form = emptyManifestForm();
    form.name = "unicode-boundaries";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    form.model.system_prompt = "emoji 😀, high \ud800, low \udc00";

    const parsed = parseManifestToml(serializeManifestForm(form));

    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.model.system_prompt).toBe("emoji 😀, high �, low �");
  });

  it("omits empty numeric fields and emits valid ones", () => {
    const form = emptyManifestForm();
    form.name = "agent";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    form.model.temperature = "0.3";
    form.model.max_tokens = "8192";
    form.resources.max_cost_per_hour_usd = "1.5";
    form.resources.max_tool_calls_per_minute = "30";

    const toml = serializeManifestForm(form);

    expect(toml).toContain("temperature = 0.3");
    expect(toml).toContain("max_tokens = 8192");
    expect(toml).toContain("[resources]");
    expect(toml).toContain("max_cost_per_hour_usd = 1.5");
    expect(toml).toContain("max_tool_calls_per_minute = 30");
    expect(toml).not.toContain("max_llm_tokens_per_hour");
  });

  it("ignores garbage in numeric fields without throwing", () => {
    const form = emptyManifestForm();
    form.name = "agent";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    form.model.temperature = "not a number";
    form.model.max_tokens = "1.5";

    const toml = serializeManifestForm(form);
    expect(toml).not.toContain("temperature =");
    expect(toml).not.toContain("max_tokens =");
  });

  it("serializes out-of-range sampling values unclamped, for the validator to catch", () => {
    // Clamping used to rewrite `5` to `1` here — a number the operator never
    // chose, reaching the TOML silently. `PATCH /api/agents/{id}/model`
    // rejects the same out-of-range values with an explicit 400, so the
    // editor now reports the same conflict via `validateManifestForm`
    // instead (#8112).
    const form = emptyManifestForm();
    form.name = "agent";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    form.model.temperature = "9";
    form.model.top_p = "5";
    form.model.frequency_penalty = "9";
    form.model.presence_penalty = "-9";

    const toml = serializeManifestForm(form);
    expect(toml).toContain("temperature = 9");
    expect(toml).toContain("top_p = 5");
    expect(toml).toContain("frequency_penalty = 9");
    expect(toml).toContain("presence_penalty = -9");

    const errors = validateManifestForm(form);
    expect(errors).toContain("model.temperature");
    expect(errors).toContain("model.top_p");
    expect(errors).toContain("model.frequency_penalty");
    expect(errors).toContain("model.presence_penalty");
  });

  it("serializes a negative top_p unclamped rather than dropping it to inherit", () => {
    // `parseFloatish` rejects negatives outright (it backs the cost/quota
    // fields, which are never negative), so routing `top_p` through it made
    // "-0.5" parse to `null` and the field silently revert to "inherit"
    // instead of surfacing as the out-of-range value it is (#8112).
    const form = emptyManifestForm();
    form.name = "agent";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    form.model.top_p = "-0.5";

    const toml = serializeManifestForm(form);
    expect(toml).toContain("top_p = -0.5");
    expect(validateManifestForm(form)).toContain("model.top_p");
  });

  it("omits sampling fields when empty or garbage", () => {
    const form = emptyManifestForm();
    form.name = "agent";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    form.model.top_p = "";
    form.model.frequency_penalty = "not a number";
    form.model.presence_penalty = "";

    const toml = serializeManifestForm(form);
    expect(toml).not.toContain("top_p");
    expect(toml).not.toContain("frequency_penalty");
    expect(toml).not.toContain("presence_penalty");
  });

  it("round-trips a negative penalty through parse and serialize", () => {
    const form = emptyManifestForm();
    form.name = "agent";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    form.model.presence_penalty = "-0.5";
    const toml = serializeManifestForm(form);
    const parsed = parseManifestToml(toml);
    if (!parsed.ok) throw new Error(parsed.message);
    expect(parsed.form.model.presence_penalty).toBe("-0.5");
    const round = serializeManifestForm(parsed.form);
    expect(round).toContain("presence_penalty = -0.5");
  });

  it("emits arrays only when populated", () => {
    const form = emptyManifestForm();
    form.name = "agent";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    form.skills = ["coder", "search"];
    form.tags = ["beta"];
    form.capabilities.network = ["api.openai.com:443"];
    form.capabilities.agent_spawn = true;

    const toml = serializeManifestForm(form);

    expect(toml).toContain('skills = ["coder", "search"]');
    expect(toml).toContain('tags = ["beta"]');
    expect(toml).toContain("[capabilities]");
    expect(toml).toContain('network = ["api.openai.com:443"]');
    expect(toml).toContain("agent_spawn = true");
    expect(toml).not.toContain("ofp_discover");
  });

  it("omits enabled when default (true), emits when disabled", () => {
    const form = emptyManifestForm();
    form.name = "agent";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    expect(serializeManifestForm(form)).not.toContain("enabled");

    form.enabled = false;
    expect(serializeManifestForm(form)).toContain("enabled = false");
  });

  it("merges extras: top-level scalars + sub-tables", () => {
    const form = emptyManifestForm();
    form.name = "agent";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";

    const extras = emptyManifestExtras();
    extras.topLevel.priority = "High";
    extras.topLevel.thinking = { budget_tokens: 10000, stream_thinking: false };
    extras.model.api_key_env = "OPENAI_API_KEY";
    extras.capabilities.memory_read = ["user/*"];

    const toml = serializeManifestForm(form, extras);

    // Form fields stay first in their hand-tuned layout.
    expect(toml.indexOf('name = "agent"')).toBeLessThan(toml.indexOf("[model]"));
    // Extras inside [model] live alongside form-known model keys.
    expect(toml).toContain('api_key_env = "OPENAI_API_KEY"');
    expect(toml).toContain('memory_read = [ "user/*" ]');
    // Top-level extras render after the form-known sections.
    expect(toml).toContain('priority = "High"');
    expect(toml).toContain("[thinking]");
    expect(toml).toContain("budget_tokens = 10000");
  });
});

describe("agentManifest validator", () => {
  // An agent type authored without a pinned provider persists `provider = ""`
  // verbatim — `AgentTypeSpec::apply_to` and `into_new_manifest` both treat
  // `Some("")` as "the caller cleared it", and `ModelConfig::provider` is a
  // plain `String` with no skip-if-empty, so the blank reaches the agent's
  // `agent.toml` on disk and back into this form.
  // Requiring it here turned Save into a silent no-op for those agents:
  // `saveManifestEditor` returns before issuing the PATCH, with no toast and no
  // request — the only signal is a red border on a Model section that sits
  // below the fold of the configuration drawer.
  it("does a not block", () => {
    const parsed = parseManifestToml(
      ['name = "inherits-default"', 'module = "builtin:chat"', "", "[model]", 'provider = ""', 'model = ""'].join("\n"),
    );
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.model.provider).toBe("");
    expect(parsed.form.model.model).toBe("");
    expect(validateManifestForm(parsed.form)).toEqual([]);
  });

  it("flags a missing name", () => {
    const errors = validateManifestForm(emptyManifestForm());
    expect(errors).toContain("name");
  });

  // #8028: a blank provider/model is the documented way an agent inherits
  // the daemon's configured default (the form's own hint text next to
  // these fields says so), and `ModelConfig`'s empty string is written
  // through verbatim by both the flat editor's patch and its create path.
  // Every agent (type) ever saved without a pinned provider had these two
  // blank on disk, so requiring them here made Save silently no-op on all
  // of them.
  it("does not require provider/model — blank means inherit the daemon default", () => {
    const form = emptyManifestForm();
    form.name = "agent";
    const errors = validateManifestForm(form);
    expect(errors).not.toContain("model.provider");
    expect(errors).not.toContain("model.model");
  });

  it("returns no errors when minimum fields are filled", () => {
    const form = emptyManifestForm();
    form.name = "agent";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    expect(validateManifestForm(form)).toEqual([]);
  });

  it("requires a cron expression for periodic schedules", () => {
    const form = emptyManifestForm();
    form.name = "agent";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    form.schedule = { mode: "periodic", cron: "   " };

    expect(validateManifestForm(form)).toContain("schedule.cron");
  });

  it.each(["", "0", "-1", "1.5", "invalid", "9223372036854775808"])(
    "requires a positive TOML integer for continuous schedules: %j",
    (check_interval_secs) => {
      const form = emptyManifestForm();
      form.name = "agent";
      form.model.provider = "openai";
      form.model.model = "gpt-4o";
      form.schedule = { mode: "continuous", check_interval_secs };

      expect(validateManifestForm(form)).toContain("schedule.check_interval_secs");
    },
  );

  // Ranges mirror `PATCH /api/agents/{id}/model`
  // (crates/librefang-api/src/routes/agents/config.rs): temperature 0..2,
  // top_p 0..1, frequency_penalty and presence_penalty -2..2.
  const outOfRangeForm = (field: "temperature" | "top_p" | "frequency_penalty" | "presence_penalty", badValue: string) => {
    const form = emptyManifestForm();
    form.name = "agent";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    form.model[field] = badValue;
    return form;
  };

  it("flags an out-of-range model.temperature above the max", () => {
    expect(validateManifestForm(outOfRangeForm("temperature", "9"))).toContain("model.temperature");
  });

  it("flags an out-of-range model.temperature below the min", () => {
    expect(validateManifestForm(outOfRangeForm("temperature", "-1"))).toContain("model.temperature");
  });

  it("flags an out-of-range model.top_p above the max", () => {
    expect(validateManifestForm(outOfRangeForm("top_p", "5"))).toContain("model.top_p");
  });

  it("flags an out-of-range model.top_p below the min", () => {
    expect(validateManifestForm(outOfRangeForm("top_p", "-0.5"))).toContain("model.top_p");
  });

  it("flags an out-of-range model.frequency_penalty", () => {
    expect(validateManifestForm(outOfRangeForm("frequency_penalty", "9"))).toContain(
      "model.frequency_penalty",
    );
  });

  it("flags an out-of-range model.presence_penalty", () => {
    expect(validateManifestForm(outOfRangeForm("presence_penalty", "-9"))).toContain(
      "model.presence_penalty",
    );
  });

  it("accepts sampling values at the edge of their range, and empty as inherit", () => {
    const form = emptyManifestForm();
    form.name = "agent";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    form.model.temperature = "2";
    form.model.top_p = "0";
    form.model.frequency_penalty = "-2";
    form.model.presence_penalty = "2";

    expect(validateManifestForm(form)).toEqual([]);
  });

  it("accepts the largest TOML integer for a continuous schedule", () => {
    const form = emptyManifestForm();
    form.name = "agent";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    form.schedule = {
      mode: "continuous",
      check_interval_secs: "9223372036854775807",
    };

    expect(validateManifestForm(form)).not.toContain("schedule.check_interval_secs");
  });

  it.each(["", "{not-json"])(
    "requires valid JSON for json_schema response format: %j",
    (schema) => {
      const form = emptyManifestForm();
      form.name = "agent";
      form.model.provider = "openai";
      form.model.model = "gpt-4o";
      form.response_format = { mode: "json_schema", name: "response", schema, strict: false };

      expect(validateManifestForm(form)).toContain("response_format.schema");
    },
  );

  it.each([
    "[]",
    '"string"',
    "42",
    "null",
    '{"const":null}',
    '{"const":9007199254740993}',
    '{"maximum":1e400}',
    '{"minimum":1e-400}',
  ])(
    "rejects schemas that TOML cannot preserve: %s",
    (schema) => {
      const form = emptyManifestForm();
      form.name = "agent";
      form.model.provider = "openai";
      form.model.model = "gpt-4o";
      form.response_format = { mode: "json_schema", name: "response", schema, strict: false };

      expect(validateManifestForm(form)).toContain("response_format.schema");
    },
  );

  it.each([
    "true",
    "false",
    '{"type":"null"}',
    '{"const":"9007199254740993"}',
    '{"type":"object","properties":{}}',
  ])(
    "accepts and round-trips supported JSON Schema: %s",
    (schema) => {
      const form = emptyManifestForm();
      form.name = "agent";
      form.model.provider = "openai";
      form.model.model = "gpt-4o";
      form.response_format = { mode: "json_schema", name: "response", schema, strict: false };

      expect(validateManifestForm(form)).not.toContain("response_format.schema");
      const parsed = parseManifestToml(serializeManifestForm(form));
      expect(parsed.ok).toBe(true);
      if (!parsed.ok || parsed.form.response_format.mode !== "json_schema") return;
      expect(JSON.parse(parsed.form.response_format.schema)).toEqual(JSON.parse(schema));
    },
  );

  it("preserves the largest safe integer in a schema", () => {
    const form = emptyManifestForm();
    form.name = "agent";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    form.response_format = {
      mode: "json_schema",
      name: "response",
      schema: '{"maximum":9007199254740991}',
      strict: false,
    };

    expect(validateManifestForm(form)).not.toContain("response_format.schema");
    const toml = serializeManifestForm(form);
    expect(toml).toContain("maximum = 9007199254740991");
    const parsed = parseManifestToml(toml);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok || parsed.form.response_format.mode !== "json_schema") return;
    expect(parsed.form.response_format.schema).toContain("9007199254740991");
  });
});

describe("agentManifest parser", () => {
  it("assigns deterministic parse-local list ids", () => {
    const source = `name = "a"

[model]
provider = "openai"
model = "gpt-4o"

[[fallback_models]]
provider = "qwen"
model = "qwen-3.6"

[[context_injection]]
name = "rules"
content = "Be concise"
`;

    const first = parseManifestToml(source);
    const second = parseManifestToml(source);
    expect(first.ok).toBe(true);
    expect(second.ok).toBe(true);
    if (!first.ok || !second.ok) return;

    expect(first.form.fallback_models?.[0]._uid).toBe("parsed-1");
    expect(first.form.context_injection[0]._uid).toBe("parsed-2");
    expect(second.form.fallback_models?.[0]._uid).toBe("parsed-1");
    expect(second.form.context_injection[0]._uid).toBe("parsed-2");
  });

  it("parses the minimum viable manifest", () => {
    const result = parseManifestToml(
      'name = "researcher"\nmodule = "builtin:chat"\n\n[model]\nprovider = "openai"\nmodel = "gpt-4o"\n',
    );
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.form.name).toBe("researcher");
    expect(result.form.model.provider).toBe("openai");
    expect(result.form.model.model).toBe("gpt-4o");
  });

  it("populates form fields from a richly-typed manifest", () => {
    const toml = `name = "agent"
description = "ops bot"
tags = ["beta"]
enabled = false

[model]
provider = "openai"
model = "gpt-4o"
temperature = 0.4
max_tokens = 2048

[resources]
max_cost_per_hour_usd = 1.5
max_tool_calls_per_minute = 30

[capabilities]
network = ["api.openai.com:443"]
agent_spawn = true
`;
    const result = parseManifestToml(toml);
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.form.description).toBe("ops bot");
    expect(result.form.tags).toEqual(["beta"]);
    expect(result.form.enabled).toBe(false);
    expect(result.form.model.temperature).toBe("0.4");
    expect(result.form.model.max_tokens).toBe("2048");
    expect(result.form.resources.max_cost_per_hour_usd).toBe("1.5");
    expect(result.form.capabilities.network).toEqual(["api.openai.com:443"]);
    expect(result.form.capabilities.agent_spawn).toBe(true);
  });

  it("hydrates advanced fields and only preserves truly-unknown extras", () => {
    const toml = `name = "agent"
priority = "High"
session_mode = "new"

[model]
provider = "openai"
model = "gpt-4o"
api_key_env = "OPENAI_API_KEY"
custom_provider_param = "preserved"

[thinking]
budget_tokens = 10000
stream_thinking = true

[autonomous]
max_iterations = 100
heartbeat_channel = "telegram"

[[fallback_models]]
provider = "anthropic"
model = "claude-3-5-sonnet"

[[context_injection]]
name = "policy"
content = "Always be polite."
position = "before_user"

[tools.web_search]
params = { region = "us" }
`;
    const result = parseManifestToml(toml);
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    // First-class fields are now in form state, not extras.
    expect(result.form.priority).toBe("High");
    expect(result.form.session_mode).toBe("new");
    expect(result.form.model.api_key_env).toBe("OPENAI_API_KEY");
    expect(result.form.thinking.enabled).toBe(true);
    expect(result.form.thinking.budget_tokens).toBe("10000");
    expect(result.form.thinking.stream_thinking).toBe(true);
    expect(result.form.autonomous.enabled).toBe(true);
    expect(result.form.autonomous.max_iterations).toBe("100");
    expect(result.form.autonomous.heartbeat_channel).toBe("telegram");
    expect((result.form.fallback_models ?? []).map(({ _uid, ...rest }) => rest)).toEqual([
      {
        provider: "anthropic",
        model: "claude-3-5-sonnet",
        api_key_env: "",
        base_url: "",
        extras: {},
      },
    ]);
    expect(result.form.context_injection.map(({ _uid, ...rest }) => rest)).toEqual([
      { name: "policy", content: "Always be polite.", position: "before_user", condition: "" },
    ]);
    // Genuinely unknown stuff (model.custom_provider_param, [tools.*])
    // still rides along in extras.
    expect(result.extras.model.custom_provider_param).toBe("preserved");
    expect(result.extras.topLevel.tools).toEqual({
      web_search: { params: { region: "us" } },
    });
  });

  it("preserves an unmapped 'channels' allowlist through extras on round-trip (#7742)", () => {
    // `channels` is a real AgentManifest field (agent.toml, PUT
    // /agents/{id}/channels) but the visual editor doesn't have a
    // first-class form widget for it — it must survive a
    // parse → serialize → re-parse cycle unchanged via extras, the same
    // guarantee every other unmapped field gets.
    const toml = `name = "agent"
channels = ["telegram", "discord"]

[model]
provider = "openai"
model = "gpt-4o"
`;
    const parsed = parseManifestToml(toml);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.extras.topLevel.channels).toEqual(["telegram", "discord"]);

    const reserialized = serializeManifestForm(parsed.form, parsed.extras);
    const reparsed = parseManifestToml(reserialized);
    expect(reparsed.ok).toBe(true);
    if (!reparsed.ok) return;
    expect(reparsed.extras.topLevel.channels).toEqual(["telegram", "discord"]);
  });

  it("returns a structured error on malformed TOML", () => {
    const result = parseManifestToml('name = "unterminated\n[oops');
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.message.length).toBeGreaterThan(0);
  });

  it("response_format json_schema with nested schema round-trips cleanly", () => {
    // Regression: an earlier serializer naively did
    //   stringify({schema: nested}).split("\n")[0]
    // which produced "[schema]" for non-trivial schemas and yielded invalid TOML.
    const toml = `name = "a"
response_format = { type = "json_schema", name = "user", schema = { type = "object", properties = { id = { type = "integer" } } } }

[model]
provider = "openai"
model = "gpt-4o"
`;
    const parsed = parseManifestToml(toml);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    const reserialized = serializeManifestForm(parsed.form, parsed.extras);
    const reparsed = parseManifestToml(reserialized);
    expect(reparsed.ok).toBe(true);
    if (!reparsed.ok) return;
    expect(reparsed.form.response_format).toEqual(parsed.form.response_format);
  });

  it("nested-table extras inside [model] don't break section scoping", () => {
    // Regression: stringify({key: nested}) can emit "[key]" headers; if
    // those get appended inside the [model] block, subsequent lines get
    // scoped to the wrong table. We must NOT emit content that re-anchors
    // scoping inside form-known sections.
    const toml = `name = "a"

[model]
provider = "openai"
model = "gpt-4o"

[model.exotic_subtable]
foo = "bar"

[resources]
max_cost_per_hour_usd = 1
`;
    const parsed = parseManifestToml(toml);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    const reserialized = serializeManifestForm(parsed.form, parsed.extras);
    const reparsed = parseManifestToml(reserialized);
    expect(reparsed.ok).toBe(true);
    if (!reparsed.ok) return;
    // The crucial assertion: max_cost_per_hour_usd must still belong to
    // [resources], not be silently re-scoped under [model.exotic_subtable].
    expect(reparsed.form.resources.max_cost_per_hour_usd).toBe("1");
    // And [model.exotic_subtable] should still be addressable as a model
    // sub-table after the round-trip, not silently re-scoped to top-level.
    expect(reparsed.extras.model.exotic_subtable).toEqual({ foo: "bar" });
  });

  it("normalizes exec_policy aliases the kernel accepts to canonical form", () => {
    // exec_policy_lenient on the kernel side accepts aliases for each
    // mode; the form's dropdown only has the 4 canonical names. Without
    // normalisation the alias spelling rounds-trips to an empty
    // shorthand (form treats it as "use global policy") and the user's
    // intent is silently lost.
    const cases: Array<[string, "deny" | "allowlist" | "full"]> = [
      ["none", "deny"],
      ["disabled", "deny"],
      ["restricted", "allowlist"],
      ["all", "full"],
      ["unrestricted", "full"],
    ];
    for (const [alias, canonical] of cases) {
      const parsed = parseManifestToml(
        `name = "a"\nexec_policy = "${alias}"\n[model]\nprovider = "openai"\nmodel = "gpt-4o"\n`,
      );
      expect(parsed.ok).toBe(true);
      if (!parsed.ok) return;
      expect(parsed.form.exec_policy_shorthand).toBe(canonical);
    }
  });

  it("keeps an exec_policy spelling the kernel accepts in a different case", () => {
    // The kernel lowercases before mapping (`exec_policy_lenient`,
    // `crates/librefang-types/src/serde_compat.rs:262`, wired in at
    // `agent.rs:1347`), so `"Deny"` and `"FULL"` are manifests the runtime
    // honours. Matching exactly here read them as a spelling the form did not
    // know, returned an empty shorthand, and dropped the key on the next save.
    // That loss widened the policy rather than narrowing it: an agent carrying
    // `shell_exec` and no `exec_policy` is promoted to `Full`
    // (`kernel/spawn.rs:236-250`, `kernel/boot.rs:2690-2705`).
    const cases: Array<[string, string]> = [
      ["Deny", "deny"],
      ["FULL", "full"],
      ["AllowList", "allowlist"],
      ["None", "deny"],
      ["UNRESTRICTED", "full"],
    ];
    for (const [spelling, canonical] of cases) {
      const parsed = parseManifestToml(
        `name = "a"\nexec_policy = "${spelling}"\n[model]\nprovider = "openai"\nmodel = "gpt-4o"\n`,
      );
      expect(parsed.ok).toBe(true);
      if (!parsed.ok) return;
      expect(parsed.form.exec_policy_shorthand).toBe(canonical);

      // And the key survives the save — the failure this guards is the key
      // disappearing, not the dropdown reading the wrong label.
      const out = serializeManifestForm(parsed.form, parsed.extras);
      expect(out).toContain("exec_policy");
    }
  });

  it("does not emit both response_format form-mode and preserved [response_format] extras", () => {
    // Same shape as the exec_policy P1: TOML carries an unmappable
    // response_format → preserved as extras → user picks json/json_schema
    // in form. Without the mutual-exclusion filter, both get emitted and
    // the result is a TOML key/table redefinition conflict.
    const toml = `name = "a"

[model]
provider = "openai"
model = "gpt-4o"

[response_format]
type = "future_format_we_dont_understand"
custom = "x"
`;
    const parsed = parseManifestToml(toml);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.response_format.mode).toBe("text"); // unmappable → defaults to text
    expect(parsed.extras.topLevel.response_format).toBeTruthy();

    // User explicitly picks json in the form.
    parsed.form.response_format = { mode: "json" };
    const reserialized = serializeManifestForm(parsed.form, parsed.extras);
    const reparsed = parseManifestToml(reserialized);
    expect(reparsed.ok).toBe(true);
    if (!reparsed.ok) return;
    expect(reparsed.form.response_format.mode).toBe("json");
    // Old preserved table must not have followed along.
    expect(reparsed.extras.topLevel.response_format).toBeUndefined();
  });

  // The keys inside a response_format table the form has no widget for must
  // survive a round-trip, mapped type or not. The mode picker is the source
  // of truth from the moment the operator touches it: edits within a mode
  // keep the unknown keys, picking a different mode replaces them — the same
  // semantics the unmappable-type test above pins.
  describe("keys the form does not render", () => {
    it("survive alongside a mapped type = json", () => {
      const parsed = parseManifestToml(
        `name = "x"\n\nresponse_format = { type = "json", custom_flag = true }\n`,
      );
      expect(parsed.ok).toBe(true);
      if (!parsed.ok) return;
      expect(parsed.form.response_format.mode).toBe("json");

      const toml = serializeManifestForm(parsed.form, parsed.extras);
      expect(toml).toContain('response_format = { type = "json", custom_flag = true }');

      const reparsed = parseManifestToml(toml);
      expect(reparsed.ok).toBe(true);
      if (!reparsed.ok) return;
      expect(reparsed.form.response_format.mode).toBe("json");
      const again = serializeManifestForm(reparsed.form, reparsed.extras);
      expect(again).toContain('custom_flag = true');
    });

    it("survive inside a mapped json_schema table", () => {
      const parsed = parseManifestToml(
        `name = "x"\n\nresponse_format = { type = "json_schema", name = "user", schema = { type = "object" }, strict = true, custom_flag = true }\n`,
      );
      expect(parsed.ok).toBe(true);
      if (!parsed.ok) return;
      if (parsed.form.response_format.mode !== "json_schema") {
        throw new Error("json_schema table did not map to the json_schema mode");
      }
      expect(parsed.form.response_format.name).toBe("user");

      const toml = serializeManifestForm(parsed.form, parsed.extras);
      expect(toml).toContain('type = "json_schema"');
      expect(toml).toContain('name = "user"');
      expect(toml).toContain("custom_flag = true");
      expect(toml).toContain("strict = true");
    });

    it("a BigInt-valued unknown key emits its digits, not an empty string", () => {
      const parsed = parseManifestToml(
        `name = "x"\n\nresponse_format = { type = "json", zz_big = 18446744073709551616 }\n`,
      );
      expect(parsed.ok).toBe(true);
      if (!parsed.ok) return;

      const toml = serializeManifestForm(parsed.form, parsed.extras);
      expect(toml).toContain("zz_big = 18446744073709551616");
      expect(toml).not.toContain('zz_big = ""');
    });

    it("a nested-table unknown key renders as an inline table", () => {
      const parsed = parseManifestToml(
        `name = "x"\n\nresponse_format = { type = "json", custom = { depth = 2 } }\n`,
      );
      expect(parsed.ok).toBe(true);
      if (!parsed.ok) return;

      const toml = serializeManifestForm(parsed.form, parsed.extras);
      // A `[custom]` header inside the value would be multi-line and would
      // re-anchor TOML scoping; only inline syntax is legal here.
      expect(toml).toMatch(/response_format = \{ type = "json", custom = \{ depth = 2 \} \}/);
    });

    it("a type = text table keeps its keys through the extras", () => {
      const parsed = parseManifestToml(
        `name = "x"\n\nresponse_format = { type = "text", custom_flag = true }\n`,
      );
      expect(parsed.ok).toBe(true);
      if (!parsed.ok) return;
      // `type = "text"` maps to the text mode, which renders nothing — so the
      // table has no form-emitted half to merge with and survives whole.
      const toml = serializeManifestForm(parsed.form, parsed.extras);
      expect(toml).toContain("custom_flag = true");
    });

    it("are replaced when the operator picks a different mode", () => {
      const parsed = parseManifestToml(
        `name = "x"\n\nresponse_format = { type = "json", custom_flag = true }\n`,
      );
      expect(parsed.ok).toBe(true);
      if (!parsed.ok) return;

      parsed.form.response_format = {
        mode: "json_schema",
        name: "user",
        schema: "{}",
        strict: false,
      };
      const toml = serializeManifestForm(parsed.form, parsed.extras);
      expect(toml).toContain('type = "json_schema"');
      expect(toml).not.toContain("custom_flag");
    });
  });

  it("parseResponseFormatField always yields a string schema", () => {
    // Codex-style regression: JSON.stringify(undefined, null, 2) returns
    // undefined, which would flow into a `<textarea value={…}>` and
    // trigger React's uncontrolled→controlled warning.
    const toml = `name = "a"
response_format = { type = "json_schema", name = "user" }

[model]
provider = "openai"
model = "gpt-4o"
`;
    const parsed = parseManifestToml(toml);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.response_format.mode).toBe("json_schema");
    if (parsed.form.response_format.mode !== "json_schema") return;
    expect(typeof parsed.form.response_format.schema).toBe("string");
  });

  it("does not emit both exec_policy shorthand and [exec_policy] table", () => {
    // Codex P1 regression: when TOML carries a full [exec_policy] table
    // and the user later picks a shorthand string in the form, the old
    // serializer wrote BOTH `exec_policy = "allowlist"` and the
    // preserved `[exec_policy]` table — TOML rejects this as a key/table
    // redefinition conflict.
    const toml = `name = "a"

[model]
provider = "openai"
model = "gpt-4o"

[exec_policy]
mode = "allowlist"
allowed_commands = ["ls"]
timeout_secs = 30
`;
    const parsed = parseManifestToml(toml);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.extras.topLevel.exec_policy).toBeTruthy();

    // User picks a shorthand in the form.
    parsed.form.exec_policy_shorthand = "deny";
    const reserialized = serializeManifestForm(parsed.form, parsed.extras);
    // Output must still be valid TOML (no duplicate exec_policy key).
    const reparsed = parseManifestToml(reserialized);
    expect(reparsed.ok).toBe(true);
    if (!reparsed.ok) return;
    expect(reparsed.form.exec_policy_shorthand).toBe("deny");
    // The full table must be gone — the shorthand wins.
    expect(reparsed.extras.topLevel.exec_policy).toBeUndefined();
  });

  it("preserves u64 resource limits above Number.MAX_SAFE_INTEGER", () => {
    const source = `name = "a"

[model]
provider = "openai"
model = "gpt-4o"

[resources]
max_llm_tokens_per_hour = 9007199254740993
max_memory_bytes = 9007199254740994
max_cpu_time_ms = 9223372036854775806
max_network_bytes_per_hour = 9223372036854775807
`;
    const parsed = parseManifestToml(source);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;

    const serialized = serializeManifestForm(parsed.form, parsed.extras);
    expect(serialized).toContain("max_llm_tokens_per_hour = 9007199254740993");
    expect(serialized).toContain("max_memory_bytes = 9007199254740994");
    expect(serialized).toContain("max_cpu_time_ms = 9223372036854775806");
    expect(serialized).toContain("max_network_bytes_per_hour = 9223372036854775807");

    const reparsed = parseManifestToml(serialized);
    expect(reparsed.ok).toBe(true);
    if (!reparsed.ok) return;
    expect(reparsed.form.resources).toMatchObject(parsed.form.resources);
  });

  it("preserves large continuous and autonomous interval values", () => {
    const source = `name = "a"
schedule = { continuous = { check_interval_secs = 9007199254740993 } }

[model]
provider = "openai"
model = "gpt-4o"

[autonomous]
heartbeat_interval_secs = 9223372036854775807
heartbeat_keep_recent = 9007199254740994
`;
    const parsed = parseManifestToml(source);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;

    const serialized = serializeManifestForm(parsed.form, parsed.extras);
    expect(serialized).toContain("check_interval_secs = 9007199254740993");
    expect(serialized).toContain("heartbeat_interval_secs = 9223372036854775807");
    expect(serialized).toContain("heartbeat_keep_recent = 9007199254740994");
  });

  it("fails closed when a JSON schema contains an unsafe BigInt", () => {
    const parsed = parseManifestToml(`name = "a"
response_format = { type = "json_schema", name = "score", schema = { maximum = 9007199254740993 } }

[model]
provider = "openai"
model = "gpt-4o"
`);

    expect(parsed.ok).toBe(false);
    if (parsed.ok) return;
    expect(parsed.message).toBe("json_schema_unsafe_integer");
  });

  it("rejects negative and out-of-range integers in number fields", () => {
    // Codex P2 regression: parseInteger used to accept any JS number,
    // including negatives (which u32/u64 deserializers reject) and
    // values outside the target unsigned Rust type.
    const form = emptyManifestForm();
    form.name = "a";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    form.model.max_tokens = "-100";
    form.resources.max_llm_tokens_per_hour = "9223372036854775808"; // TOML i64::MAX + 1

    const toml = serializeManifestForm(form);
    expect(toml).not.toContain("max_tokens =");
    expect(toml).not.toContain("max_llm_tokens_per_hour =");
  });

  it("preserves per-fallback-model extra_params on round-trip", () => {
    // Codex P2 regression: FallbackModel has #[serde(flatten)] extra_params,
    // which the parser used to drop. Provider-specific fields like
    // `enable_memory` (Qwen) survive a round-trip now.
    const toml = `name = "a"

[model]
provider = "openai"
model = "gpt-4o"

[[fallback_models]]
provider = "qwen"
model = "qwen-3.6"
enable_memory = true
custom_param = "preserved"
`;
    const parsed = parseManifestToml(toml);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.fallback_models?.[0].extras).toEqual({
      enable_memory: true,
      custom_param: "preserved",
    });
    const reserialized = serializeManifestForm(parsed.form, parsed.extras);
    const reparsed = parseManifestToml(reserialized);
    expect(reparsed.ok).toBe(true);
    if (!reparsed.ok) return;
    expect(reparsed.form.fallback_models?.[0].extras).toEqual({
      enable_memory: true,
      custom_param: "preserved",
    });
  });

  it("schedule round-trips through every variant", () => {
    const periodic = parseManifestToml(
      'name = "a"\nschedule = { periodic = { cron = "0 9 * * *" } }\n[model]\nprovider = "openai"\nmodel = "gpt-4o"\n',
    );
    expect(periodic.ok).toBe(true);
    if (!periodic.ok) return;
    expect(periodic.form.schedule).toEqual({ mode: "periodic", cron: "0 9 * * *" });

    const continuous = parseManifestToml(
      'name = "a"\nschedule = { continuous = { check_interval_secs = 600 } }\n[model]\nprovider = "openai"\nmodel = "gpt-4o"\n',
    );
    expect(continuous.ok).toBe(true);
    if (!continuous.ok) return;
    expect(continuous.form.schedule).toEqual({ mode: "continuous", check_interval_secs: "600" });
  });

  it("response_format json_schema preserves the schema body", () => {
    const toml = `name = "a"
response_format = { type = "json_schema", name = "user", schema = { type = "object", properties = { id = { type = "integer" } } }, strict = true }

[model]
provider = "openai"
model = "gpt-4o"
`;
    const result = parseManifestToml(toml);
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.form.response_format.mode).toBe("json_schema");
    if (result.form.response_format.mode !== "json_schema") return;
    expect(result.form.response_format.name).toBe("user");
    expect(result.form.response_format.strict).toBe(true);
    const parsedSchema = JSON.parse(result.form.response_format.schema);
    expect(parsedSchema.type).toBe("object");
    expect(parsedSchema.properties.id.type).toBe("integer");
  });

  // #7946 added `reasoning_mode` to the `[thinking]` table, and the form has no
  // widget for it. Before the extras slot below, opening any agent in the visual
  // editor and pressing save re-emitted `[thinking]` from `budget_tokens` and
  // `stream_thinking` alone, silently deleting the operator's reasoning mode
  // from agent.toml — the same class of loss `extras.capabilities` already guards.
  it("round-trips an unknown [thinking] key such as reasoning_mode", () => {
    const original = `name = "agent"

[thinking]
budget_tokens = 5000
stream_thinking = true
reasoning_mode = "none"
`;
    const result = parseManifestToml(original);
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.form.thinking.budget_tokens).toBe("5000");
    expect(result.extras.thinking).toEqual({ reasoning_mode: "none" });

    const out = serializeManifestForm(result.form, result.extras);
    expect(out).toContain('reasoning_mode = "none"');
    // And it must land inside [thinking], not leak into a later section: an
    // extra scalar emitted after the next `[header]` would belong to that
    // section instead, which is a different (and silent) kind of corruption.
    const after = out.slice(out.indexOf("[thinking]") + "[thinking]".length);
    const nextHeader = after.search(/\n\[/);
    const thinkingBlock = nextHeader === -1 ? after : after.slice(0, nextHeader);
    expect(thinkingBlock).toContain('reasoning_mode = "none"');

    // Stable across a second pass.
    const second = parseManifestToml(out);
    expect(second.ok).toBe(true);
    if (!second.ok) return;
    expect(second.extras.thinking).toEqual({ reasoning_mode: "none" });
  });

  // The same loss, in a section that never got its slot.
  //
  // `[autonomous]` is in `FORM_TOP_LEVEL_KEYS`, so its table never reaches
  // `extras.topLevel`, and `ManifestExtras` has no member for it — so every key
  // the form has no widget for is consumed on parse and never re-emitted on
  // save. `block_stall_degrade_after` is one today, and it is the loop-guard
  // threshold, not a display preference: an agent that had it set comes back
  // from an unrelated edit with the guard gone.
  it("round-trips an unknown [autonomous] key such as block_stall_degrade_after", () => {
    const original = `name = "agent"

[autonomous]
max_iterations = 50
block_stall_degrade_after = 2
`;
    const result = parseManifestToml(original);
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.form.autonomous.max_iterations).toBe("50");

    const out = serializeManifestForm(result.form, result.extras);
    expect(out).toContain("block_stall_degrade_after");

    // Inside [autonomous], not leaked into whichever section follows: a scalar
    // emitted after the next `[header]` belongs to that section instead.
    const after = out.slice(out.indexOf("[autonomous]") + "[autonomous]".length);
    const nextHeader = after.search(/\n\[/);
    const block = nextHeader === -1 ? after : after.slice(0, nextHeader);
    expect(block).toContain("block_stall_degrade_after");

    // Stable across a second pass.
    const second = parseManifestToml(out);
    expect(second.ok).toBe(true);
    if (!second.ok) return;
    expect(second.extras.autonomous).toEqual({ block_stall_degrade_after: 2 });
  });

  // The other half of the same slot, and the one that is pure prophylaxis today:
  // `ModelRoutingConfig` has exactly the five fields the form already knows, so
  // no `[routing]` key can be dropped — yet. The next one added there would be,
  // which is how `block_stall_degrade_after` and `reasoning_mode` went missing.
  // `escalate_model` is not a field of that struct; it stands in for that next
  // field, and what this test pins is the slot, not the name.
  it("round-trips an unknown [routing] key the form has no field for", () => {
    const original = `name = "agent"

[routing]
simple_model = "a"
medium_model = "b"
complex_model = "c"
simple_threshold = 1000
complex_threshold = 8000
escalate_model = "d"
`;
    const result = parseManifestToml(original);
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.form.routing.simple_model).toBe("a");

    const out = serializeManifestForm(result.form, result.extras);
    expect(out).toContain("escalate_model");

    // Inside [routing], not leaked into whichever section follows.
    const after = out.slice(out.indexOf("[routing]") + "[routing]".length);
    const nextHeader = after.search(/\n\[/);
    const block = nextHeader === -1 ? after : after.slice(0, nextHeader);
    expect(block).toContain("escalate_model");

    // Stable across a second pass.
    const second = parseManifestToml(out);
    expect(second.ok).toBe(true);
    if (!second.ok) return;
    expect(second.extras.routing).toEqual({ escalate_model: "d" });
  });

  // Unticking "enabled" is the user deleting the whole table, so the preserved
  // keys go with it rather than stranding a [thinking] block nothing owns.
  it("drops preserved [thinking] extras when the section is disabled", () => {
    const result = parseManifestToml(`name = "agent"

[thinking]
reasoning_mode = "max"
`);
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    result.form.thinking.enabled = false;
    const out = serializeManifestForm(result.form, result.extras);
    expect(out).not.toContain("[thinking]");
    expect(out).not.toContain("reasoning_mode");
  });

  it("round-trips a declared fallback_models = [] without re-enabling global fallbacks", () => {
    // #7749 review: `fallback_models = []` is the disable-all statement; an
    // omitted key inherits the global fallback_providers. A form that
    // collapses the two re-routes a pinned agent's spend on an unrelated save.
    const original = `name = "agent"
description = "test"

fallback_models = []
`;
    const parsed = parseManifestToml(original);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.fallback_models).toEqual([]);
    const round = serializeManifestForm(parsed.form, parsed.extras);
    expect(round).toContain("fallback_models = []");
    const reparsed = parseManifestToml(round);
    expect(reparsed.ok).toBe(true);
    if (!reparsed.ok) return;
    expect(reparsed.form.fallback_models).toEqual([]);
  });

  it("emits a declared fallback_models = [] at top level, not inside the last table (#7749 review)", () => {
    // The fixture above is `name` + `description` only, so every section body
    // came out empty, no `[header]` was ever emitted, and the bare key landed
    // at top level by accident. Every manifest the daemon actually serves
    // carries a `[model]` table (`toml::to_string_pretty` on a real
    // `AgentManifest`), which put `fallback_models = []` inside `[model]`.
    // `AgentManifest`/`ModelConfig` declare no `deny_unknown_fields`, so the
    // kernel dropped it silently and the agent went back to inheriting the
    // deployment-wide `fallback_providers` chain with no error surfaced.
    const original = `name = "agent"
description = "test"

fallback_models = []

[model]
provider = "anthropic"
model = "claude-sonnet-4"
`;
    const parsed = parseManifestToml(original);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.fallback_models).toEqual([]);

    const round = serializeManifestForm(parsed.form, parsed.extras);
    // The key must precede the first table header, which is what makes it
    // top-level. Asserting only `toContain("fallback_models = []")` passes on
    // the broken output too — that is exactly how this shipped.
    const keyAt = round.indexOf("fallback_models = []");
    const firstHeaderAt = round.indexOf("[model]");
    expect(keyAt).toBeGreaterThanOrEqual(0);
    expect(firstHeaderAt).toBeGreaterThanOrEqual(0);
    expect(keyAt).toBeLessThan(firstHeaderAt);

    // And the round trip has to survive a re-parse as a top-level key rather
    // than surfacing as a `model` extra.
    const reparsed = parseManifestToml(round);
    expect(reparsed.ok).toBe(true);
    if (!reparsed.ok) return;
    expect(reparsed.form.fallback_models).toEqual([]);
    expect(reparsed.extras.model).not.toHaveProperty("fallback_models");
    expect(reparsed.form.model.provider).toBe("anthropic");
  });

  it("keeps an absent fallback_models absent after a round trip (inherit stays inherit)", () => {
    const original = `name = "agent"
description = "test"
`;
    const parsed = parseManifestToml(original);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.fallback_models).toBeNull();
    const round = serializeManifestForm(parsed.form, parsed.extras);
    expect(round).not.toContain("fallback_models");
    const reparsed = parseManifestToml(round);
    expect(reparsed.ok).toBe(true);
    if (!reparsed.ok) return;
    expect(reparsed.form.fallback_models).toBeNull();
  });

  it("round-trips a declared memory_read = [] without flipping it to unrestricted", () => {
    // #7749 review: `memory_read = []` is the deny-all declaration (#7605) —
    // absent means unrestricted. A form state that cannot carry the
    // distinction silently re-enables an agent's memory on an unrelated save.
    const original = `name = "agent"
description = "test"

[capabilities]
memory_read = []
`;
    const parsed = parseManifestToml(original);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.capabilities.memory_read).toEqual([]);
    const round = serializeManifestForm(parsed.form, parsed.extras);
    expect(round).toContain("memory_read = []");
    const reparsed = parseManifestToml(round);
    expect(reparsed.ok).toBe(true);
    if (!reparsed.ok) return;
    expect(reparsed.form.capabilities.memory_read).toEqual([]);
  });

  it("keeps an absent memory_read absent after a round trip (unrestricted stays unrestricted)", () => {
    const original = `name = "agent"
description = "test"

[capabilities]
network = ["api.openai.com:443"]
`;
    const parsed = parseManifestToml(original);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.capabilities.memory_read).toBeNull();
    const round = serializeManifestForm(parsed.form, parsed.extras);
    expect(round).not.toContain("memory_read");
    const reparsed = parseManifestToml(round);
    expect(reparsed.ok).toBe(true);
    if (!reparsed.ok) return;
    expect(reparsed.form.capabilities.memory_read).toBeNull();
  });

  it("round-trips: serialize(parse(toml)) preserves form + extras", () => {
    const original = `name = "agent"
description = "test"
priority = "High"
session_mode = "new"
web_search_augmentation = "always"
schedule = { periodic = { cron = "0 9 * * *" } }
exec_policy = "allowlist"

[model]
provider = "openai"
model = "gpt-4o"
temperature = 0.5
api_key_env = "OPENAI_API_KEY"
custom_provider_param = "preserved"

[resources]
max_cost_per_hour_usd = 2

[capabilities]
network = ["api.openai.com:443"]
memory_read = ["user/*"]

[thinking]
budget_tokens = 5000
stream_thinking = true

[autonomous]
max_iterations = 100
heartbeat_channel = "telegram"

[routing]
simple_model = "claude-haiku"
medium_model = "claude-sonnet"
complex_model = "claude-opus"
simple_threshold = 100
complex_threshold = 500

[[fallback_models]]
provider = "anthropic"
model = "claude-3-5-sonnet"

[[context_injection]]
name = "policy"
content = "Be polite."
position = "before_user"

[tools.web_search]
params = { region = "us" }
`;
    const parsed = parseManifestToml(original);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;

    const reserialized = serializeManifestForm(parsed.form, parsed.extras);
    const reparsed = parseManifestToml(reserialized);
    expect(reparsed.ok).toBe(true);
    if (!reparsed.ok) return;

    // The form state and extras should match exactly after a full round-trip.
    // _uid is an ephemeral React key rather than manifest data, so strip it.
    const stripUids = <
      T extends Record<string, unknown> & { _uid?: string },
    >(items: T[]): Omit<T, "_uid">[] =>
      items.map(({ _uid, ...rest }) => rest) as Omit<T, "_uid">[];
    const cleanForm = (f: typeof parsed.form) => ({
      ...f,
      fallback_models: f.fallback_models === null ? null : stripUids(f.fallback_models),
      context_injection: stripUids(f.context_injection),
    });
    expect(cleanForm(reparsed.form)).toEqual(cleanForm(parsed.form));
    expect(reparsed.extras).toEqual(parsed.extras);
  });
  it("round-trips a manifest with triggers, compaction, an MCP allowlist and unknown keys without losing or moving anything", () => {
    const original = `name = "parity"
session_mode = "new"
mcp_servers = ["github"]
tool_allowlist = ["file_read"]
future_field = "unknown to this daemon"

[workspaces]
notes = { path = "notes", mode = "rw" }

[compaction]
threshold_messages = 7

[[triggers]]
pattern = "git.push"
prompt_template = "on push"
`;
    const parsed = parseManifestToml(original);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    const out = serializeManifestForm(parsed.form, parsed.extras);

    // Nothing was lost — the sections the deleted editor note promised to preserve
    // survive the parse -> serialize -> parse cycle.
    const reparsed = parseManifestToml(out);
    expect(reparsed.ok).toBe(true);
    if (!reparsed.ok) return;
    expect(reparsed.form.session_mode).toBe("new");
    expect(reparsed.form.mcp_servers).toEqual(["github"]);
    expect(reparsed.form.tool_allowlist).toEqual(["file_read"]);
    expect(reparsed.extras.topLevel["future_field"]).toBe("unknown to this daemon");
    // `[compaction]` became a first-class form table in this branch, so the
    // value round-trips through `form.compaction` instead of surviving as an
    // unknown top-level key — the same change of address `[workspaces]` went
    // through below, in #8013. Where it survives changed; that it survives has
    // not, and the assertion is stronger for it: it now checks the value
    // reaches the field the daemon reads, not just that the table was kept.
    expect(reparsed.form.compaction.threshold_messages).toBe("7");
    // `[workspaces]` is a first-class form field since #8013, so a path-based row round-trips through `form.workspaces` instead of surviving as an unknown top-level key.
    // Where it survives changed; that it survives has not.
    expect(reparsed.form.workspaces).toHaveLength(1);
    const { _uid: _ignoredWorkspaceUid, ...workspace } = reparsed.form.workspaces[0];
    expect(workspace).toEqual({ name: "notes", path: "notes", mode: "rw" });
    expect(reparsed.extras.topLevel["triggers"]).toEqual([
      { pattern: "git.push", prompt_template: "on push" },
    ]);

    // …and nothing was moved: every scalar/array still sits before the first table
    // header, so no later key can be absorbed into a preceding section (the #8013
    // hazard — a table emitted before the remaining top-level scalars would swallow
    // tags, skills, mcp_servers, schedule and the rest).
    const firstTableHeader = out.search(/^\[/m);
    expect(firstTableHeader).toBeGreaterThan(-1);
    for (const key of ["session_mode", "mcp_servers", "tool_allowlist", "future_field"]) {
      const at = out.indexOf(`${key} =`);
      expect(at).toBeGreaterThanOrEqual(0);
      expect(at).toBeLessThan(firstTableHeader);
    }
  });

});

describe("agentManifest — inference parameters (#7781)", () => {
  it("round-trips every preference knob and both endpoint limits", () => {
    const form = emptyManifestForm();
    form.name = "academic-writer";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    form.model.temperature = "0.1";
    form.model.top_p = "0.85";
    form.model.frequency_penalty = "0.4";
    form.model.presence_penalty = "-0.3";
    form.model.max_tokens = "8192";
    form.model.context_window = "200000";
    form.model.max_output_tokens = "16384";

    const toml = serializeManifestForm(form);
    expect(toml).toContain("temperature = 0.1");
    expect(toml).toContain("top_p = 0.85");
    expect(toml).toContain("frequency_penalty = 0.4");
    expect(toml).toContain("presence_penalty = -0.3");
    expect(toml).toContain("max_tokens = 8192");
    expect(toml).toContain("context_window = 200000");
    expect(toml).toContain("max_output_tokens = 16384");

    const parsed = parseManifestToml(toml);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.model.temperature).toBe("0.1");
    expect(parsed.form.model.top_p).toBe("0.85");
    expect(parsed.form.model.frequency_penalty).toBe("0.4");
    expect(parsed.form.model.presence_penalty).toBe("-0.3");
    expect(parsed.form.model.max_tokens).toBe("8192");
    expect(parsed.form.model.context_window).toBe("200000");
    expect(parsed.form.model.max_output_tokens).toBe("16384");
  });

  /**
   * The inherit state has to survive the round trip as an *absent key*.
   * Writing `top_p = 0` instead would pin a number the operator never chose and
   * make the per-model override unreachable for that field — the exact failure
   * the tri-state was introduced to remove.
   */
  it("omits a knob left on inherit rather than writing a zero", () => {
    const form = emptyManifestForm();
    form.name = "inheriting";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    form.model.temperature = "0.1";

    const toml = serializeManifestForm(form);
    expect(toml).toContain("temperature = 0.1");
    expect(toml).not.toContain("top_p");
    expect(toml).not.toContain("frequency_penalty");
    expect(toml).not.toContain("presence_penalty");
    expect(toml).not.toContain("max_tokens");
    expect(toml).not.toContain("context_window");
    expect(toml).not.toContain("max_output_tokens");

    const parsed = parseManifestToml(toml);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.model.top_p).toBe("");
    expect(parsed.form.model.max_tokens).toBe("");
    expect(parsed.form.model.context_window).toBe("");
  });

  /**
   * The migration guarantee for the 25 already-deployed agents: a manifest that
   * carries a number keeps it as an explicit value. Nothing starts inheriting
   * behind the operator's back on upgrade.
   */
  it("keeps an existing explicit value explicit", () => {
    const parsed = parseManifestToml(
      [
        'name = "deployed"',
        'module = "builtin:chat"',
        "",
        "[model]",
        'provider = "openai"',
        'model = "gpt-4o"',
        "temperature = 0.7",
        "max_tokens = 4096",
      ].join("\n"),
    );
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.model.temperature).toBe("0.7");
    expect(parsed.form.model.max_tokens).toBe("4096");

    // …and comes back out unchanged.
    const toml = serializeManifestForm(parsed.form);
    expect(toml).toContain("temperature = 0.7");
    expect(toml).toContain("max_tokens = 4096");
  });
});

describe("agentManifest workspaces", () => {
  // #8013: `[workspaces]` is a table header — if the serializer emitted it
  // inside the top-level scalar block, every bare key after it (tags, skills,
  // mcp_servers, schedule, …) would be scoped INTO the table and silently
  // deleted from the manifest.
  it("emits [workspaces] after the top-level scalars so tags survive a round-trip", () => {
    const form = emptyManifestForm();
    form.name = "agent";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    form.tags = ["ops"];
    form.workspaces.push({ _uid: "w1", name: "shared", path: "shared", mode: "rw" });

    const toml = serializeManifestForm(form);

    expect(toml).toContain("[workspaces]");
    expect(toml).toContain('tags = ["ops"]');
    expect(toml.indexOf("tags = ")).toBeLessThan(toml.indexOf("[workspaces]"));

    const parsed = parseManifestToml(toml);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.tags).toEqual(["ops"]);
    expect(parsed.form.workspaces).toHaveLength(1);
    const { _uid: _ignored, ...ws } = parsed.form.workspaces[0];
    expect(ws).toEqual({ name: "shared", path: "shared", mode: "rw" });
  });

  it("emits no [workspaces] header for an empty or blank-row list", () => {
    const form = emptyManifestForm();
    form.name = "agent";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    expect(serializeManifestForm(form)).not.toContain("[workspaces]");

    form.workspaces.push({ _uid: "blank", name: "  ", path: "  ", mode: "rw" });
    expect(serializeManifestForm(form)).not.toContain("[workspaces]");
  });

  it("preserves a mount-based declaration verbatim instead of dropping it", () => {
    const parsed = parseManifestToml(`name = "agent"
[workspaces]
vault = { mount = "/data/vault" }
shared = { path = "shared" }
`);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;

    // Only the path-based row becomes editable; the mount survives in extras.
    expect(parsed.form.workspaces.map((ws) => ws.name)).toEqual(["shared"]);
    const reserialized = serializeManifestForm(parsed.form, parsed.extras);
    expect(reserialized).toContain("[workspaces.vault]");
    expect(reserialized).toContain('mount = "/data/vault"');
    // ...and the whole thing parses again.
    const reparsed = parseManifestToml(reserialized);
    expect(reparsed.ok).toBe(true);
  });

  it("flags duplicate folder names, including against a preserved declaration", () => {
    const form = emptyManifestForm();
    form.name = "agent";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    form.workspaces.push(
      { _uid: "w1", name: "shared", path: "shared", mode: "rw" },
      { _uid: "w2", name: "shared", path: "other", mode: "r" },
    );
    expect(validateManifestForm(form)).toContain("workspaces.w2.name");
    expect(validateManifestForm(form, ["vault"])).not.toContain("workspaces.w1.name");

    form.workspaces[1].name = "vault";
    expect(validateManifestForm(form, ["vault"])).toContain("workspaces.w2.name");
  });

  it.each(["/etc/passwd", "\\\\host\\share", "C:\\data", "../escape", "a/../b"])(
    "rejects a workspace path that escapes workspaces_dir: %j",
    (wsPath) => {
      const form = emptyManifestForm();
      form.name = "agent";
      form.model.provider = "openai";
      form.model.model = "gpt-4o";
      form.workspaces.push({ _uid: "w1", name: "shared", path: wsPath, mode: "rw" });
      expect(validateManifestForm(form)).toContain("workspaces.w1.path");
    },
  );

  it("accepts a plain relative workspace path", () => {
    const form = emptyManifestForm();
    form.name = "agent";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    form.workspaces.push({ _uid: "w1", name: "shared", path: "shared/library", mode: "rw" });
    expect(validateManifestForm(form)).toEqual([]);
  });

  it.each(["r", "read", "read-only", "readonly"])(
    // #8013: the kernel's `WorkspaceMode` accepts all four spellings, and
    // "readonly" is the one it actually writes (persist_full_manifest_at,
    // and the template endpoints). Missing any of them means a read-only
    // shared folder silently becomes read-write the moment this form
    // re-saves it.
    "parses %j as read-only, matching the kernel's WorkspaceMode aliases",
    (modeSpelling) => {
      const parsed = parseManifestToml(`name = "agent"
[workspaces]
shared = { path = "shared", mode = "${modeSpelling}" }
`);
      expect(parsed.ok).toBe(true);
      if (!parsed.ok) return;
      expect(parsed.form.workspaces[0].mode).toBe("r");
    },
  );

  it("parses an unrecognized mode spelling as read-write rather than silently upgrading", () => {
    const parsed = parseManifestToml(`name = "agent"
[workspaces]
shared = { path = "shared", mode = "bogus" }
`);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.workspaces[0].mode).toBe("rw");
  });

  it("flags a half-filled row (name without a path) instead of dropping it silently", () => {
    const form = emptyManifestForm();
    form.name = "agent";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    form.workspaces.push({ _uid: "w1", name: "shared", path: "", mode: "rw" });
    const errors = validateManifestForm(form);
    expect(errors).toContain("workspaces.w1.path");
    expect(errors).not.toContain("workspaces.w1.name");
  });

  it("flags a half-filled row (path without a name) instead of dropping it silently", () => {
    const form = emptyManifestForm();
    form.name = "agent";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    form.workspaces.push({ _uid: "w1", name: "", path: "shared", mode: "rw" });
    const errors = validateManifestForm(form);
    expect(errors).toContain("workspaces.w1.name");
    expect(errors).not.toContain("workspaces.w1.path");
  });

  it("does not flag a wholly blank row", () => {
    const form = emptyManifestForm();
    form.name = "agent";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    form.workspaces.push({ _uid: "w1", name: "  ", path: "  ", mode: "rw" });
    expect(validateManifestForm(form)).toEqual([]);
  });

  it.each(["shared/library", "shared library", "@shared"])(
    // #8013: expand_workspace_alias matches only the segment before the
    // first '/' against the declared name, so a name with '/' can never be
    // addressed via '@name/...'; whitespace and '@' are excluded for the
    // same reason — they let the alias resolve to a name the agent never typed.
    "rejects a workspace name outside the alias-safe character set: %j",
    (name) => {
      const form = emptyManifestForm();
      form.name = "agent";
      form.model.provider = "openai";
      form.model.model = "gpt-4o";
      form.workspaces.push({ _uid: "w1", name, path: "shared", mode: "rw" });
      expect(validateManifestForm(form)).toContain("workspaces.w1.name");
    },
  );

  it("preserves the remainder of a preserved workspace name containing a dot", () => {
    // #8013: `dottedKey.split(".", 2)` truncates rather than preserving the
    // remainder, so "workspaces.notes.v2" re-emitted as "[workspaces.notes]"
    // and lost "v2" — silently colliding with a sibling entry literally
    // named "notes".
    const parsed = parseManifestToml(`name = "agent"
[workspaces]
"notes.v2" = { mount = "/data/notes" }
notes = { mount = "/data/other" }
`);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;

    const reserialized = serializeManifestForm(parsed.form, parsed.extras);
    expect(reserialized).toContain('[workspaces."notes.v2"]');
    expect(reserialized).toContain('mount = "/data/notes"');
    expect(reserialized).toContain("[workspaces.notes]");
    expect(reserialized).toContain('mount = "/data/other"');

    const reparsed = parseManifestToml(reserialized);
    expect(reparsed.ok).toBe(true);
  });

  it("preservedWorkspaceNamesFromExtras extracts the preserved-name collision list AgentsPage wires into validateManifestForm", () => {
    // #8013: this parameter was never supplied at the only production call
    // site, so the collision check below was dead outside its own test.
    const parsed = parseManifestToml(`name = "agent"
[workspaces]
vault = { mount = "/data/vault" }
`);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;

    expect(preservedWorkspaceNamesFromExtras(parsed.extras)).toEqual(["vault"]);
    expect(preservedWorkspaceNamesFromExtras(emptyManifestExtras())).toEqual([]);
  });
});

describe("agentManifest capability routing", () => {
  it("omits an empty field instead of pinning an empty provider", () => {
    const form = emptyManifestForm();
    form.name = "profesor";
    form.capabilities.image_understanding = "";

    const toml = serializeManifestForm(form, emptyManifestExtras());
    // Omission is what the kernel reads as "inherit the global block"; an
    // `image_understanding = ""` would pin an empty provider instead.
    expect(toml).not.toContain("image_understanding");
  });

  it("writes a filled field into [capabilities]", () => {
    const form = emptyManifestForm();
    form.name = "profesor";
    form.capabilities.image_understanding = "openai/gpt-4o";
    form.capabilities.speech_to_text = "groq";

    const toml = serializeManifestForm(form, emptyManifestExtras());
    expect(toml).toContain("[capabilities]");
    expect(toml).toContain('image_understanding = "openai/gpt-4o"');
    expect(toml).toContain('speech_to_text = "groq"');
  });

  it("round-trips the string shorthand through parse and serialize", () => {
    const parsed = parseManifestToml(
      ['name = "profesor"', "", "[capabilities]", 'image_understanding = "openai/gpt-4o"'].join("\n"),
    );
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;

    expect(parsed.form.capabilities.image_understanding).toBe("openai/gpt-4o");
    const toml = serializeManifestForm(parsed.form, parsed.extras);
    expect(toml).toContain('image_understanding = "openai/gpt-4o"');
    // Exactly once — the key must not survive in `extras` as well, which
    // would emit two spellings of the same setting.
    expect(toml.match(/image_understanding/g)).toHaveLength(1);
  });

  it("normalises the { provider, model } table form to the shorthand", () => {
    const parsed = parseManifestToml(
      [
        'name = "profesor"',
        "",
        "[capabilities]",
        'speech_to_text = { provider = "groq", model = "whisper-large-v3" }',
      ].join("\n"),
    );
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.capabilities.speech_to_text).toBe("groq/whisper-large-v3");
  });

  it("keeps a model-only override inheriting the provider", () => {
    const parsed = parseManifestToml(
      ['name = "profesor"', "", "[capabilities]", 'image_understanding = { model = "gpt-4o-mini" }'].join("\n"),
    );
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    // "/model" is how an inherited provider survives a round-trip through a
    // single text field; the kernel parses it back to provider=None.
    expect(parsed.form.capabilities.image_understanding).toBe("/gpt-4o-mini");
  });

  it("loads the kernel's aliases into the canonical field", () => {
    const parsed = parseManifestToml(
      ['name = "profesor"', "", "[capabilities]", 'vision = "gemini/gemini-2.5-flash"', 'transcription = "openai"'].join(
        "\n",
      ),
    );
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.capabilities.image_understanding).toBe("gemini/gemini-2.5-flash");
    expect(parsed.form.capabilities.speech_to_text).toBe("openai");
    // The alias must not also linger in extras, or the re-emitted block would
    // carry both `vision` and `image_understanding`.
    expect(parsed.extras.capabilities).not.toHaveProperty("vision");
    expect(parsed.extras.capabilities).not.toHaveProperty("transcription");
  });

  it("leaves the existing tool and memory grants untouched", () => {
    const parsed = parseManifestToml(
      [
        'name = "profesor"',
        "",
        "[capabilities]",
        'tools = ["memory_recall"]',
        'memory_read = ["*"]',
        'image_understanding = "openai/gpt-4o"',
      ].join("\n"),
    );
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.capabilities.tools).toEqual(["memory_recall"]);
    expect(parsed.form.capabilities.memory_read).toEqual(["*"]);
    expect(parsed.form.capabilities.image_understanding).toBe("openai/gpt-4o");
  });
});

// `assignee_wake` is an `Option<bool>` on the Rust side: absent means "inherit
// the kernel's [task_board].assignee_wake", and the agent writes a value only
// when an operator overrides it. That makes it a tri-state, and a tri-state
// over a boolean is where this file has been wrong before — the same shape as
// `fallback_models = []` (an explicit empty list is a statement, not an
// absence) and `system_prompt` (a blank prompt is a statement too).
describe("assignee_wake tri-state", () => {
  it("omits the key when the agent has no opinion", () => {
    const form = emptyManifestForm();
    form.assignee_wake = "";

    // Absent, not `false`: writing `false` would pin the agent against a later
    // change to the deployment-wide default.
    expect(serializeManifestForm(form)).not.toContain("assignee_wake");
  });

  it("emits an explicit false, which is a real override", () => {
    const form = emptyManifestForm();
    form.assignee_wake = "false";

    expect(serializeManifestForm(form)).toContain("assignee_wake = false");
  });

  it("round-trips both explicit values", () => {
    for (const value of ["true", "false"] as const) {
      const form = emptyManifestForm();
      form.assignee_wake = value;

      const parsed = parseManifestToml(serializeManifestForm(form));
      expect(parsed.ok).toBe(true);
      if (!parsed.ok) return;
      expect(parsed.form.assignee_wake).toBe(value);
    }
  });
});

// Booleans whose serde default is not `false`.
//
// `show_progress` is declared `#[serde(default = "default_true")]` in
// `AgentManifest`, so omitting the key means the agent *shows* progress. A
// form that defaulted it to `false` and emitted the key would turn the
// progress indicator off for every agent opened and saved without anyone
// asking — a silent behaviour change on a value the operator never touched.
// The two defaults have to agree in both directions: what the form writes when
// the operator says nothing, and what it reads back when the file says nothing.
describe("booleans whose default is true", () => {
  it("agrees with serde on the default for an absent key", () => {
    const parsed = parseManifestToml('name = "x"\n');
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    // The Rust side fills this with `default_true`.
    expect(parsed.form.show_progress).toBe(true);
    // And this one with `#[serde(default)]`, i.e. false.
    expect(parsed.form.cache_context).toBe(false);
    expect(parsed.form.mcp_disabled).toBe(false);
  });

  it("omits show_progress while it holds the default", () => {
    const form = emptyManifestForm();
    expect(form.show_progress).toBe(true);
    expect(serializeManifestForm(form)).not.toContain("show_progress");
  });

  it("emits show_progress only when it is turned off", () => {
    const form = emptyManifestForm();
    form.show_progress = false;
    expect(serializeManifestForm(form)).toContain("show_progress = false");
  });

  it("omits the false-by-default flags while they are false", () => {
    const form = emptyManifestForm();
    const toml = serializeManifestForm(form);
    expect(toml).not.toContain("cache_context");
    expect(toml).not.toContain("mcp_disabled");
  });

  it("round-trips every state of all three", () => {
    for (const show of [true, false]) {
      for (const cache of [true, false]) {
        for (const mcp of [true, false]) {
          const form = emptyManifestForm();
          form.show_progress = show;
          form.cache_context = cache;
          form.mcp_disabled = mcp;

          const parsed = parseManifestToml(serializeManifestForm(form));
          expect(parsed.ok).toBe(true);
          if (!parsed.ok) return;
          expect(parsed.form.show_progress).toBe(show);
          expect(parsed.form.cache_context).toBe(cache);
          expect(parsed.form.mcp_disabled).toBe(mcp);
        }
      }
    }
  });
});

// `Option<usize>` / `Option<u32>` / `Option<Enum>` fields: absent is a state,
// and it is not zero and not the first variant. A count written as `0` is a
// limit of nothing; an enum written as its Rust variant name is a value the
// daemon cannot deserialise, and because these fields all carry
// `#[serde(default)]` that failure is a silent revert rather than an error.
describe("optional manifest overrides", () => {
  it("omits the counts when they are not set", () => {
    const toml = serializeManifestForm(emptyManifestForm());
    expect(toml).not.toContain("max_history_messages");
    expect(toml).not.toContain("max_concurrent_invocations");
  });

  it("emits a count as a bare number, not a string", () => {
    const form = emptyManifestForm();
    form.max_history_messages = "80";
    form.max_concurrent_invocations = "3";

    const toml = serializeManifestForm(form);
    // TOML-typed: `max_history_messages = "80"` would be a string and the
    // daemon would refuse the manifest.
    expect(toml).toContain("max_history_messages = 80");
    expect(toml).toContain("max_concurrent_invocations = 3");
  });

  it("round-trips the counts", () => {
    const form = emptyManifestForm();
    form.max_history_messages = "80";
    form.max_concurrent_invocations = "3";

    const parsed = parseManifestToml(serializeManifestForm(form));
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.max_history_messages).toBe("80");
    expect(parsed.form.max_concurrent_invocations).toBe("3");
  });

  it("omits every optional enum when unset, and round-trips every value", () => {
    // Plain string keys, not `keyof ManifestFormState`: a keyof union includes
    // symbol, which cannot be interpolated into the TOML assertions below.
    const cases: Array<[string, readonly string[]]> = [
      ["tool_exec_backend", ["local", "docker", "ssh", "daytona"]],
      ["profile", ["minimal", "coding", "research", "messaging", "automation", "full", "custom"]],
    ];

    for (const [field, values] of cases) {
      expect(serializeManifestForm(emptyManifestForm())).not.toContain(`${field} =`);

      for (const value of values) {
        const form = emptyManifestForm();
        (form as unknown as Record<string, string>)[field] = value;

        const toml = serializeManifestForm(form);
        expect(toml).toContain(`${field} = "${value}"`);

        const parsed = parseManifestToml(toml);
        expect(parsed.ok).toBe(true);
        if (!parsed.ok) return;
        expect((parsed.form as unknown as Record<string, string>)[field]).toBe(value);
      }
    }
  });

  // `reconcile_orphans` is not an `Option`: it is an `OrphanPolicy` whose serde
  // default is `Keep`. So the form has to agree on what "unset" means, and
  // `keep` is the state that must not be written — an agent that has never
  // been given a policy should not acquire one just by being opened and saved.
  it("leaves reconcile_orphans out while it holds the default", () => {
    const form = emptyManifestForm();
    expect(form.reconcile_orphans).toBe("keep");
    expect(serializeManifestForm(form)).not.toContain("reconcile_orphans");
  });

  it("writes reconcile_orphans for the two non-default policies", () => {
    for (const value of ["warn", "delete"] as const) {
      const form = emptyManifestForm();
      form.reconcile_orphans = value;

      const toml = serializeManifestForm(form);
      expect(toml).toContain(`reconcile_orphans = "${value}"`);

      const parsed = parseManifestToml(toml);
      expect(parsed.ok).toBe(true);
      if (!parsed.ok) return;
      expect(parsed.form.reconcile_orphans).toBe(value);
    }
  });
});

// `[proactive_memory]` is a per-agent override table: every field is
// `Option<T>` on the Rust side with `skip_serializing_if`, so "inherit" and
// "explicitly off" are different keys on disk.
describe("proactive_memory overrides", () => {
  it("writes no table at all when every field inherits", () => {
    // A `[proactive_memory]` header that overrides nothing reads as a
    // configured section and is not one.
    expect(serializeManifestForm(emptyManifestForm())).not.toContain("[proactive_memory]");
  });

  it("writes the table as soon as one field is set", () => {
    const form = emptyManifestForm();
    form.proactive_memory.auto_retrieve = "false";

    const toml = serializeManifestForm(form);
    expect(toml).toContain("[proactive_memory]");
    // An explicit `false` is an override and must be written; omitting it here
    // is the bug that turns "this agent does not retrieve" into "this agent
    // does whatever the deployment says".
    expect(toml).toContain("auto_retrieve = false");
  });

  it("round-trips every field", () => {
    const form = emptyManifestForm();
    form.proactive_memory = {
      enabled: "true",
      auto_memorize: "false",
      auto_retrieve: "true",
      extraction_model: "ollama/llama3",
      session_scoped_recall: "false",
      min_similarity: "0.7",
      allow_self_consolidation: "true",
    };

    const parsed = parseManifestToml(serializeManifestForm(form));
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.proactive_memory).toEqual(form.proactive_memory);
  });

  // The reason this table has its own slot in `ManifestExtras`.
  //
  // The form knows seven keys. Adding the table to `FORM_TOP_LEVEL_KEYS` means
  // its contents stop reaching `topLevel`, so without a slot of its own every
  // key the form has no widget for is consumed on parse and never re-emitted —
  // which is how `[thinking] reasoning_mode` and `[autonomous]
  // block_stall_degrade_after` were being deleted before those slots existed.
  // Opening an agent in the editor and saving must not remove a key upstream
  // added.
  it("preserves keys inside the table that the form does not render", () => {
    // The fixture carries no key the form renders, and that is load-bearing.
    // With one present the body is never empty, the guard that drops a
    // body-less table never runs, and the test passes over the very bug it is
    // named for — measured, it did, until the fixture was cut to unknown keys.
    const source = [
      'name = "x"',
      "",
      "[proactive_memory]",
      "consolidation_interval_turns = 42",
      "",
      "[proactive_memory.tuning]",
      'mode = "eager"',
    ].join("\n");

    const parsed = parseManifestToml(source);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;

    const round = serializeManifestForm(parsed.form, parsed.extras);
    expect(round).toContain("[proactive_memory]");
    expect(round).toContain("consolidation_interval_turns = 42");
    expect(round).toContain("[proactive_memory.tuning]");
    expect(round).toContain('mode = "eager"');
  });

  it("keeps the table when every key the form knows serializes to nothing", () => {
    // A table can be present, non-empty, and still produce an empty body: the
    // form owns `extraction_model`, but an empty value writes no line. The same
    // guard then dropped the whole table, preserved keys included.
    const source = [
      'name = "x"',
      "",
      "[proactive_memory]",
      'extraction_model = ""',
      "consolidation_interval_turns = 42",
    ].join("\n");

    const parsed = parseManifestToml(source);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;

    const round = serializeManifestForm(parsed.form, parsed.extras);
    expect(round).toContain("[proactive_memory]");
    expect(round).toContain("consolidation_interval_turns = 42");
  });
});

// `auto_dream_min_hours` and `auto_dream_min_sessions` are top-level scalars,
// and the manifest format has no way to say "the table is over" — a bare key
// after the first `[section]` header belongs to that section. Emitting them
// with the tables would scope them into whichever one came last, where the
// daemon would never look; that is the bug `fallback_models = []` hit before,
// in the other direction.
describe("top-level scalars stay above the first table header", () => {
  it("emits the auto-dream thresholds before any section", () => {
    const form = emptyManifestForm();
    form.auto_dream_min_hours = "12";
    form.auto_dream_min_sessions = "25";
    // Force at least one table so there is a header to be scoped into.
    form.thinking.enabled = true;

    const toml = serializeManifestForm(form);
    const firstHeader = toml.indexOf("\n[");

    expect(firstHeader).toBeGreaterThan(-1);
    for (const key of ["auto_dream_min_hours = 12", "auto_dream_min_sessions = 25"]) {
      const at = toml.indexOf(key);
      expect(at, `${key} missing`).toBeGreaterThan(-1);
      expect(
        at,
        `${key} was emitted after the first table header, so it would be read ` +
          `as a key of that table and never reach the manifest field it names.`,
      ).toBeLessThan(firstHeader);
    }
  });

  it("round-trips both thresholds", () => {
    const form = emptyManifestForm();
    form.auto_dream_min_hours = "12.5";
    form.auto_dream_min_sessions = "25";

    const parsed = parseManifestToml(serializeManifestForm(form));
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.auto_dream_min_hours).toBe("12.5");
    expect(parsed.form.auto_dream_min_sessions).toBe("25");
  });
});

describe("rl_export and async_tasks", () => {
  it("omits rl_export while it inherits, and writes the table when set", () => {
    expect(serializeManifestForm(emptyManifestForm())).not.toContain("rl_export");

    const form = emptyManifestForm();
    form.rl_export = "false";
    const toml = serializeManifestForm(form);

    expect(toml).toContain("[rl_export]");
    // An explicit `false` is an override of the kernel switch and must be
    // written; that is the whole difference the tri-state exists to keep.
    expect(toml).toContain("enabled = false");
  });

  it("round-trips rl_export in all three states", () => {
    for (const value of ["", "true", "false"] as const) {
      const form = emptyManifestForm();
      form.rl_export = value;
      const parsed = parseManifestToml(serializeManifestForm(form));
      expect(parsed.ok).toBe(true);
      if (!parsed.ok) return;
      expect(parsed.form.rl_export).toBe(value);
    }
  });

  it("omits async_tasks when nothing is overridden", () => {
    const toml = serializeManifestForm(emptyManifestForm());
    expect(toml).not.toContain("[async_tasks]");
    expect(toml).not.toContain("notify_on_timeout");
  });

  // `AsyncTasksConfig` has a manual `Default` impl with `notify_on_timeout:
  // true` (crates/librefang-types/src/agent.rs), and its doc says why: a
  // timeout is user-visible by default so the agent can react. So `true` is the
  // state that must not be written — emitting it pins the agent against a later
  // change to that default *and* records a decision the operator never made.
  // Only `false` is one.
  it("writes notify_on_timeout only when false, and round-trips the pair", () => {
    const loud = emptyManifestForm();
    loud.async_tasks.default_timeout_secs = "300";
    expect(serializeManifestForm(loud)).not.toContain("notify_on_timeout");

    const quiet = emptyManifestForm();
    quiet.async_tasks = { default_timeout_secs: "300", notify_on_timeout: false };
    const toml = serializeManifestForm(quiet);
    expect(toml).toContain("[async_tasks]");
    expect(toml).toContain("notify_on_timeout = false");

    const parsed = parseManifestToml(toml);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.async_tasks).toEqual(quiet.async_tasks);
  });

  it("reads an omitted notify_on_timeout as the compiled default, not as off", () => {
    // The daemon notifies when the key is absent. The editor read that as "off",
    // so the operator turned the switch on and the form wrote `true` — the
    // value the daemon was already using. Neither the display nor the write
    // changed anything, and `false`, the one value that does change behaviour,
    // could not be expressed at all.
    const parsed = parseManifestToml("[async_tasks]\ndefault_timeout_secs = 300\n");
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.async_tasks.notify_on_timeout).toBe(true);
  });

  it("keeps an explicit false across a round trip instead of deleting the key", () => {
    const parsed = parseManifestToml("[async_tasks]\nnotify_on_timeout = false\n");
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;

    const round = serializeManifestForm(parsed.form, parsed.extras);
    expect(round).toContain("[async_tasks]");
    expect(round).toContain("notify_on_timeout = false");
  });

  it("preserves keys inside [async_tasks] the form does not render", () => {
    // Unknown keys only — see the note on the [proactive_memory] case above.
    const source = ['name = "x"', "", "[async_tasks]", "max_concurrent = 8"].join("\n");

    const parsed = parseManifestToml(source);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    const round = serializeManifestForm(parsed.form, parsed.extras);
    expect(round).toContain("[async_tasks]");
    expect(round).toContain("max_concurrent = 8");
  });

  it("preserves keys inside [rl_export] the form does not render", () => {
    // `rl_export` is in `FORM_TOP_LEVEL_KEYS`, so its table never reaches
    // `topLevel`; without a slot of its own in `ManifestExtras` every key but
    // `enabled` was consumed on parse and never re-emitted.
    const source = ['name = "x"', "", "[rl_export]", "sample_rate = 0.5"].join("\n");

    const parsed = parseManifestToml(source);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    const round = serializeManifestForm(parsed.form, parsed.extras);
    expect(round).toContain("[rl_export]");
    expect(round).toContain("sample_rate = 0.5");
  });
});

// These two counters were interpolated into the TOML as raw text while the
// other nine numeric fields went through a parser, so `-5`, `1.5` and `1e3`
// were emitted verbatim. `PATCH /api/agents/{id}` parses the document before it
// writes, so the result was a 400 — noisy rather than corrupting, but the
// editor feeds that server and must not produce a document it will reject.
describe("the per-agent counters are whole numbers or nothing", () => {
  it("omits a counter that is not a whole number", () => {
    for (const bad of ["-5", "1.5", "1e3", "1,000", "99999999999999999999"]) {
      const form = emptyManifestForm();
      form.max_history_messages = bad;
      form.max_concurrent_invocations = bad;
      const toml = serializeManifestForm(form);
      expect(toml, `${bad} reached the TOML`).not.toContain("max_history_messages");
      expect(toml, `${bad} reached the TOML`).not.toContain("max_concurrent_invocations");
    }
  });

  it("writes both counters when they are whole numbers", () => {
    const form = emptyManifestForm();
    form.max_history_messages = "40";
    form.max_concurrent_invocations = "4";
    const toml = serializeManifestForm(form);
    expect(toml).toContain("max_history_messages = 40");
    expect(toml).toContain("max_concurrent_invocations = 4");
  });

  it("reports both counters before the save, not after the server rejects it", () => {
    for (const bad of ["-5", "1.5", "1e3", "99999999999999999999"]) {
      const form = emptyManifestForm();
      form.name = "x";
      form.max_history_messages = bad;
      expect(validateManifestForm(form), bad).toContain("max_history_messages");
    }

    const form = emptyManifestForm();
    form.name = "x";
    form.max_concurrent_invocations = "-1";
    expect(validateManifestForm(form)).toContain("max_concurrent_invocations");
  });

  it("treats a blank counter as an override the agent does not make", () => {
    const form = emptyManifestForm();
    form.name = "x";
    form.max_history_messages = "";
    form.max_concurrent_invocations = "   ";
    expect(validateManifestForm(form)).toEqual([]);
  });

  // `max_concurrent_invocations` deserializes into `Option<u32>`
  // (crates/librefang-types/src/agent.rs:1480), so one power of two above the
  // whole-number check the form used to apply, `4294967296`, passed the
  // validator, reached the TOML, and came back as a 400 the report had
  // already claimed closed once. The ceiling is per field, not global:
  // `max_history_messages` is `Option<usize>` (agent.rs:1452) and takes the
  // same value without complaint.
  it("reports a concurrency cap above u32::MAX, the field's real ceiling", () => {
    const form = emptyManifestForm();
    form.name = "x";
    form.max_concurrent_invocations = "4294967296";
    expect(validateManifestForm(form)).toContain("max_concurrent_invocations");
  });

  it("accepts u32::MAX itself for the concurrency cap", () => {
    const form = emptyManifestForm();
    form.name = "x";
    form.max_concurrent_invocations = "4294967295";
    expect(validateManifestForm(form)).not.toContain("max_concurrent_invocations");
  });

  it("does not impose the u32 ceiling on the usize counter beside it", () => {
    const form = emptyManifestForm();
    form.name = "x";
    form.max_history_messages = "4294967296";
    expect(validateManifestForm(form)).not.toContain("max_history_messages");
  });
});

// A fifth member of the same class, found by measuring rather than by reading
// the list: `[schedule]`'s variant tables are where the form's schedule editor
// keeps its state, and they are the one place the `stripKnown` + extras
// treatment was never applied.
//
// The `[schedule]` *root* is a closed enum (`ScheduleMode` in
// crates/librefang-types/src/agent.rs), so an unknown key there is rejected by
// the daemon and there is nothing to preserve. Inside a variant the daemon
// rejects an unknown key too — measured against the parse the PATCH runs
// (`toml::from_str::<AgentManifest>`), for the root, the inline form, the
// variant content and a nested table alike — so what this slot preserves is
// never a document the daemon accepts today. Its reason to exist is forward
// compatibility: when a future manifest gains a schedule field, an old
// editor's round-trip must hand it back instead of deleting it, and the
// editor is also a faithful TOML round-trip tool for drafts the daemon has
// not accepted yet. That is why every test here works on a fixture the
// current daemon would reject with a 400: the guard is the editor not
// deleting content it does not render, not the daemon losing config it was
// reading.
describe("the schedule variants keep the keys the form does not render", () => {
  for (const [variant, known] of [
    ["periodic", 'cron = "0 9 * * *"'],
    ["continuous", "check_interval_secs = 600"],
    ["proactive", 'conditions = ["nightly"]'],
  ] as const) {
    it(`[schedule.${variant}] keeps a key a future manifest may carry`, () => {
      const source = `name = "x"\n\n[schedule.${variant}]\n${known}\nfuture_knob = 7\n`;

      const parsed = parseManifestToml(source);
      expect(parsed.ok).toBe(true);
      if (!parsed.ok) return;

      const round = serializeManifestForm(parsed.form, parsed.extras);
      expect(round).toContain("future_knob = 7");
      // And the file a save produces still preserves it for the editor after
      // this one — a save from the old editor must not be the step that
      // finally deletes the field.
      const again = parseManifestToml(round);
      expect(again.ok).toBe(true);
      if (!again.ok) return;
      expect(again.extras.schedule?.[variant]).toEqual({ future_knob: 7 });
    });

    it(`[schedule.${variant}] keeps the form's own key alongside it`, () => {
      const source = `name = "x"\n\n[schedule.${variant}]\n${known}\nfuture_knob = 7\n`;

      const parsed = parseManifestToml(source);
      expect(parsed.ok).toBe(true);
      if (!parsed.ok) return;

      const again = parseManifestToml(serializeManifestForm(parsed.form, parsed.extras));
      expect(again.ok).toBe(true);
      if (!again.ok) return;
      expect(again.form.schedule.mode).toBe(variant);
    });
  }

  // A future manifest field is not necessarily a scalar. The preserved slot
  // stores whatever value shape the variant table carried, and the emit half
  // must return every shape it stores — a table-valued key, or an array of
  // tables, dies the same death a scalar would have died before the slot
  // existed. `schedule` renders as ONE inline table (a `[schedule.x]` header
  // after the bare `schedule = …` key would re-anchor TOML scoping), so the
  // only legal home for these values is inline inside it.
  it("[schedule.periodic] keeps an unknown TABLE-valued key", () => {
    const source = [
      'name = "x"',
      "",
      "[schedule.periodic]",
      'cron = "0 9 * * *"',
      "future_table = { depth = 2 }",
    ].join("\n");

    const parsed = parseManifestToml(source);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.extras.schedule?.periodic).toEqual({ future_table: { depth: 2 } });

    const round = serializeManifestForm(parsed.form, parsed.extras);
    expect(round).toContain("future_table = { depth = 2 }");
  });

  // smol-toml parses integers past JavaScript's safe range as BigInt, and
  // the preserved stash carries it whole — the inline emitter must emit the
  // digits, not collapse the value to an empty string.
  it("[schedule.periodic] round-trips a BigInt-valued unknown key", () => {
    const parsed = parseManifestToml(
      `name = "x"\n\n[schedule.periodic]\ncron = "0 9 * * *"\nzz_big = 18446744073709551616\n`,
    );
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;

    const toml = serializeManifestForm(parsed.form, parsed.extras);
    expect(toml).toContain("zz_big = 18446744073709551616");
  });

  it("[schedule.periodic] keeps an unknown array-of-tables key", () => {
    const source = [
      'name = "x"',
      "",
      "[schedule.periodic]",
      'cron = "0 9 * * *"',
      "",
      "[[schedule.periodic.future_rows]]",
      "weight = 1",
    ].join("\n");

    const parsed = parseManifestToml(source);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;

    const round = serializeManifestForm(parsed.form, parsed.extras);
    expect(round).toContain("future_rows = [{ weight = 1 }]");
  });
});

// The class this phase spent its time on: a table the form claims as its own
// stops reaching `topLevel`, so every key it has no widget for is consumed on
// parse and never re-emitted.
// `[thinking] reasoning_mode`, `[autonomous] block_stall_degrade_after`,
// `[rl_export]` and the `[schedule.<variant>]` tables were all this, and each
// was found separately — one at a time, each by a measurement taken for
// something else.
//
// Fixing instances one at a time is how a class survives, so this asks the
// question of every table instead of the one that last broke.
describe("every table the form owns keeps the keys it does not render", () => {
  // Table-shaped by construction. A scalar has nowhere to hide a second key,
  // and a row-shaped `[[array]]` carries its extras inside the row object.
  // The `[schedule]` root is a closed enum, so its unknown key has to sit one
  // level down — which is exactly why its slot is one level deep too.
  const TABLES: ReadonlyArray<readonly [string, string]> = [
    ["model", "[model]\nzz_unknown = 7"],
    ["resources", "[resources]\nzz_unknown = 7"],
    ["capabilities", "[capabilities]\nzz_unknown = 7"],
    ["thinking", "[thinking]\nzz_unknown = 7"],
    ["autonomous", "[autonomous]\nzz_unknown = 7"],
    ["routing", "[routing]\nzz_unknown = 7"],
    ["proactive_memory", "[proactive_memory]\nzz_unknown = 7"],
    ["async_tasks", "[async_tasks]\nzz_unknown = 7"],
    ["rl_export", "[rl_export]\nzz_unknown = 7"],
    ["exec_policy", "[exec_policy]\nzz_unknown = 7"],
    // `type = "json"` is deliberate: the fixture must carry a type the form
    // maps, because an unknown-key table with NO type falls into the branch
    // that preserves the whole table — the same fixture defect the sweep
    // itself documents on the compaction test. Without a mapped type this
    // entry passes over the one response_format loss it exists to catch.
    ["response_format", '[response_format]\ntype = "json"\nzz_unknown = 7'],
    ["schedule", '[schedule.periodic]\ncron = "0 9 * * *"\nzz_unknown = 7'],
    ["compaction", "[compaction]\nzz_unknown = 7"],
    ["skill_workshop", "[skill_workshop]\nzz_unknown = 7"],
    ["channel_overrides", "[channel_overrides]\nzz_unknown = 7"],
  ];

  for (const [table, body] of TABLES) {
    it(`[${table}]`, () => {
      const parsed = parseManifestToml(`name = "x"\n\n${body}\n`);
      expect(parsed.ok, `[${table}] did not parse`).toBe(true);
      if (!parsed.ok) return;

      const round = serializeManifestForm(parsed.form, parsed.extras);
      expect(
        round,
        `[${table}] dropped a key the form has no widget for, so opening an ` +
          `agent in this editor and saving removes it from the manifest.`,
      ).toContain("zz_unknown = 7");
    });
  }

  // The [model] fixture above is top-level only, and the form appropriates
  // one level deeper than it: router_override is an inline table inside
  // [model] whose unknown members the form re-emits or drops whole. A guard
  // that never looks one level down swept this instance by nothing.
  it("[model.router_override] keeps an unknown key nested inside", () => {
    const parsed = parseManifestToml(
      `name = "x"\n\n[model]\nrouter_override = { zz_unknown = 7 }\n`,
    );
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;

    const round = serializeManifestForm(parsed.form, parsed.extras);
    expect(round).toContain("zz_unknown = 7");
  });

  // The form appropriates two levels beyond a first-key probe: row
  // collections (arrays of objects the form re-renders from four known
  // fields) and inline tables nested inside them. ContextInjection and
  // WorkspaceDecl carry no deny_unknown_fields, so an unknown key inside a
  // row is a legal manifest member the daemon keeps on disk — the same
  // silent deletion the section sweep exists to catch, one row down.
  const ROWS: ReadonlyArray<readonly [string, string]> = [
    [
      "fallback_models",
      "[[fallback_models]]\nprovider = \"p\"\nmodel = \"m\"\nzz_unknown = 7",
    ],
    [
      "context_injection",
      '[[context_injection]]\nname = "n"\ncontent = "c"\ncondition = "always"\nzz_unknown = 7',
    ],
    ["workspaces", '[workspaces]\nmine = { path = "sub", zz_unknown = 7 }'],
  ];

  for (const [row, body] of ROWS) {
    it(`[${row}] keeps the keys a row does not render`, () => {
      const parsed = parseManifestToml(`name = "x"\n\n${body}\n`);
      expect(parsed.ok, `[${row}] did not parse`).toBe(true);
      if (!parsed.ok) return;

      const round = serializeManifestForm(parsed.form, parsed.extras);
      expect(
        round,
        `[${row}] dropped a key the form has no widget for, so opening an ` +
          `agent in this editor and saving removes it from the manifest.`,
      ).toContain("zz_unknown = 7");
    });
  }

  // The sweep's first-level probe cannot see rows or nesting: an empty
  // manifest's arrays are empty, indistinguishable from the scalar lists.
  // The interface declares every row collection as `Array<{`, so the guard
  // reads the declaration — the same source-reading style the routability
  // guard uses — and fails when a row collection is added to the form
  // without a sweep entry.
  it("sweeps every row collection the interface declares", () => {
    const libSource = readFileSync(join(__dirname, "agentManifest.ts"), "utf8");
    const declared = [...libSource.matchAll(/(\w+): Array<\{/g)].map((m) => m[1]);
    expect(declared.length).toBeGreaterThan(0);

    const swept = ROWS.map(([row]) => row);
    const unswept = declared.filter((k) => !swept.includes(k));
    expect(
      unswept,
      `these row collections are swept by nothing: ${unswept.join(", ")}`,
    ).toEqual([]);
  });

  it("sweeps every table the form claims, not a hand-kept list", () => {
    // A table added to the form without an entry above would be swept by
    // nothing — which is the state `[rl_export]` was in when it was found.
    const objectValued = Object.entries(emptyManifestForm())
      .filter(([, v]) => v !== null && typeof v === "object" && !Array.isArray(v))
      .map(([k]) => k);
    const swept = TABLES.map(([t]) => t);
    const unswept = objectValued.filter(
      (k) => FORM_TOP_LEVEL_KEYS.has(k) && !swept.includes(k),
    );

    expect(
      unswept,
      `these form-owned tables are swept by nothing: ${unswept.join(", ")}`,
    ).toEqual([]);
  });

  it("never names a table the form does not actually own", () => {
    // A rename that misses this list would leave the sweep green and wrong.
    const stale = TABLES.map(([t]) => t).filter((t) => !FORM_TOP_LEVEL_KEYS.has(t));
    expect(
      stale,
      `the sweep lists tables the form does not claim: ${stale.join(", ")}`,
    ).toEqual([]);
  });
});

// `[compaction]` is the same shape as `[proactive_memory]`: nine `Option<T>`
// overrides, so "inherit" and "explicitly set" are different keys on disk and
// an untouched table must not be written at all.
describe("compaction overrides", () => {
  it("writes no table when every field inherits", () => {
    expect(serializeManifestForm(emptyManifestForm())).not.toContain("[compaction]");
  });

  it("writes the table as soon as one field is set", () => {
    const form = emptyManifestForm();
    form.compaction.threshold_messages = "40";

    const toml = serializeManifestForm(form);
    expect(toml).toContain("[compaction]");
    expect(toml).toContain("threshold_messages = 40");
  });

  it("round-trips every field", () => {
    const form = emptyManifestForm();
    form.compaction = {
      threshold_messages: "40",
      keep_recent: "10",
      max_summary_tokens: "4096",
      token_threshold_ratio: "0.7",
      max_chunk_chars: "8000",
      max_retries: "2",
      aggregate_developer_loops: "false",
      max_loop_steps_before_aggregate: "5",
      strip_reasoning_after_turns: "2",
    };

    const parsed = parseManifestToml(serializeManifestForm(form));
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.compaction).toEqual(form.compaction);
  });

  // An explicit `false` here is a real override — "do not aggregate these
  // loops" — and the whole reason this one field is a tri-state select rather
  // than a toggle. A toggle renders "inherit" and "false" identically, so
  // touching it would write a decision nobody made.
  it("keeps an explicit false distinct from inherit", () => {
    const inherited = emptyManifestForm();
    const explicit = emptyManifestForm();
    explicit.compaction.aggregate_developer_loops = "false";

    expect(serializeManifestForm(inherited)).not.toContain("aggregate_developer_loops");
    expect(serializeManifestForm(explicit)).toContain("aggregate_developer_loops = false");

    const parsed = parseManifestToml(serializeManifestForm(explicit));
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.compaction.aggregate_developer_loops).toBe("false");
  });

  it("preserves keys inside [compaction] the form does not render", () => {
    // One key the form does not render, and no key it does — with a rendered
    // key present the body is never empty, the guard that drops a body-less
    // table never runs, and this test passes over the bug it is named for.
    const source = [
      'name = "x"',
      "",
      "[compaction]",
      "summariser_model = \"cheap/model\"",
    ].join("\n");

    const parsed = parseManifestToml(source);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    const round = serializeManifestForm(parsed.form, parsed.extras);
    expect(round).toContain("[compaction]");
    expect(round).toContain('summariser_model = "cheap/model"');
  });
});

// `[skill_workshop]` is not all-`Option` like the other tables: its fields are
// plain `bool` / enum / `u32` with a `Default` on the struct, so "absent" and
// "the default" are the same state. That means the form's defaults have to
// match the Rust ones exactly, in the direction that writes nothing when they
// agree.
describe("skill_workshop overrides", () => {
  const DEFAULTS = {
    enabled: false,
    // Not `false`. `impl Default for SkillWorkshopConfig` sets this to `true`:
    // the workshop is off, but its capture pass is on, so that switching the
    // master switch on gives a workshop that does something. A form that
    // defaulted it to `false` would write `auto_capture = false` for every
    // agent opened and saved, turning capture off for anyone who had never
    // expressed an opinion.
    auto_capture: true,
    approval_policy: "pending" as const,
    review_mode: "heuristic" as const,
    max_pending: "",
    max_pending_age_days: "",
    evolution_mode: "free" as const,
  };

  it("starts at the Rust defaults", () => {
    expect(emptyManifestForm().skill_workshop).toEqual(DEFAULTS);
  });

  it("writes no table while every field holds its default", () => {
    expect(serializeManifestForm(emptyManifestForm())).not.toContain("[skill_workshop]");
  });

  it("writes auto_capture only when it is turned off, never when it is on", () => {
    const on = emptyManifestForm();
    on.skill_workshop.auto_capture = true;
    expect(serializeManifestForm(on)).not.toContain("auto_capture");

    const off = emptyManifestForm();
    off.skill_workshop.auto_capture = false;
    const toml = serializeManifestForm(off);
    expect(toml).toContain("[skill_workshop]");
    expect(toml).toContain("auto_capture = false");
  });

  it("writes enabled only when it is turned on", () => {
    const on = emptyManifestForm();
    on.skill_workshop.enabled = true;
    expect(serializeManifestForm(on)).toContain("enabled = true");

    const off = emptyManifestForm();
    off.skill_workshop.enabled = false;
    expect(serializeManifestForm(off)).not.toContain("enabled =");
  });

  it("writes an enum only when it leaves its default", () => {
    const form = emptyManifestForm();
    form.skill_workshop.approval_policy = "auto";
    form.skill_workshop.review_mode = "none";
    form.skill_workshop.evolution_mode = "controlled";

    const toml = serializeManifestForm(form);
    expect(toml).toContain('approval_policy = "auto"');
    expect(toml).toContain('review_mode = "none"');
    expect(toml).toContain('evolution_mode = "controlled"');
  });

  it("round-trips every field away from its default", () => {
    const form = emptyManifestForm();
    form.skill_workshop = {
      enabled: true,
      auto_capture: false,
      approval_policy: "auto",
      review_mode: "threshold_llm",
      max_pending: "50",
      max_pending_age_days: "14",
      evolution_mode: "controlled",
    };

    const parsed = parseManifestToml(serializeManifestForm(form));
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.skill_workshop).toEqual(form.skill_workshop);
  });

  it("reads an absent auto_capture as on, matching the Rust default", () => {
    const parsed = parseManifestToml('[skill_workshop]\nenabled = true\n');
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.skill_workshop.enabled).toBe(true);
    expect(parsed.form.skill_workshop.auto_capture).toBe(true);
  });
});

// `[channel_overrides]` is the largest table here: 29 fields, of which eight
// have defaults that come from named functions rather than the type's zero.
// That is the whole risk — a form that writes one of those defaults back turns
// "inherit" into "override with the value it happened to have".
describe("channel_overrides", () => {
  it("writes no table when every field holds its default", () => {
    expect(serializeManifestForm(emptyManifestForm())).not.toContain("[channel_overrides]");
  });

  it("writes none of the custom defaults when they are untouched", () => {
    // These are `#[serde(default = "…")]` on the Rust side, so an absent key
    // means the function's value. Emitting them would pin the agent to
    // whatever those functions return today.
    const toml = serializeManifestForm(emptyManifestForm());
    for (const key of [
      "message_debounce_max_ms",
      "message_debounce_max_buffer",
      "auto_route_ttl_minutes",
      "auto_route_confidence_threshold",
      "auto_route_sticky_bonus",
      "auto_route_divergence_count",
      "conversation_ownership_ttl_seconds",
    ]) {
      expect(toml, `${key} was written while untouched`).not.toContain(key);
    }
  });

  it("has a form default matching default_thread_ownership_enabled", () => {
    // The fourth default in this work that reads backwards: the Rust default
    // is `true`, so `false` is the value worth writing and `true` is the state
    // that must produce no key at all.
    const form = emptyManifestForm();
    expect(form.channel_overrides.thread_ownership_enabled).toBe(true);
    expect(serializeManifestForm(form)).not.toContain("thread_ownership_enabled");

    form.channel_overrides.thread_ownership_enabled = false;
    const toml = serializeManifestForm(form);
    expect(toml).toContain("[channel_overrides]");
    expect(toml).toContain("thread_ownership_enabled = false");
  });

  it("round-trips every field away from its default", () => {
    const form = emptyManifestForm();
    form.channel_overrides = {
      model: "openai/gpt-4o",
      system_prompt: "be brief",
      dm_policy: "allowed_only",
      group_policy: "mention_only",
      group_trigger_patterns: ["^!"],
      reply_precheck: true,
      reply_precheck_model: "cheap/model",
      rate_limit_per_minute: "30",
      rate_limit_per_user: "5",
      threading: true,
      output_format: "telegram_html",
      usage_footer: "tokens",
      typing_mode: "thinking",
      message_debounce_ms: "500",
      message_debounce_max_ms: "5000",
      message_debounce_max_buffer: "32",
      clear_done_reaction: true,
      disable_commands: true,
      allowed_commands: ["/help"],
      blocked_commands: ["/rm"],
      auto_route: "sticky_ttl",
      auto_route_ttl_minutes: "60",
      auto_route_confidence_threshold: "7",
      auto_route_sticky_bonus: "4",
      auto_route_divergence_count: "2",
      prefix_agent_name: "bracket",
      thread_ownership_enabled: false,
      conversation_ownership_ttl_seconds: "1800",
      conversation_ownership_include_dms: true,
    };

    const parsed = parseManifestToml(serializeManifestForm(form));
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.channel_overrides).toEqual(form.channel_overrides);
  });

  it("preserves keys inside the table the form does not render", () => {
    const source = [
      'name = "x"',
      "",
      "[channel_overrides]",
      "threading = true",
      "future_toggle = true",
    ].join("\n");

    const parsed = parseManifestToml(source);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    const round = serializeManifestForm(parsed.form, parsed.extras);
    expect(round).toContain("future_toggle = true");
    expect(round).toContain("threading = true");
  });
});

// The five manifest fields the profile router edits — `[model] mode` plus
// `router_override` — and `[resources] burst_ratio` round-trip through the
// form like every other field. These are the fields
// `GET/PUT /api/agents/{id}/model_routing` reads and writes, so a form that
// drops one would let the routing panel and this editor disagree about the
// same manifest.
describe("model router fields round-trip through the form", () => {
  const BASE = ['name = "x"', "", "[model]", 'provider = "openai"', 'model = "gpt-4o"'].join(
    "\n",
  );

  it("mode = flexible round-trips", () => {
    const parsed = parseManifestToml(`${BASE}\nmode = "flexible"\n`);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.model.mode).toBe("flexible");

    const toml = serializeManifestForm(parsed.form, parsed.extras);
    expect(toml).toContain('mode = "flexible"');

    const reparsed = parseManifestToml(toml);
    expect(reparsed.ok).toBe(true);
    if (!reparsed.ok) return;
    expect(reparsed.form.model.mode).toBe("flexible");
  });

  it("mode = fixed is the default that is not written", () => {
    const form = emptyManifestForm();
    form.name = "x";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";

    const toml = serializeManifestForm(form);
    // `fixed` is `ModelMode`'s `#[default]` variant; writing it would record a
    // decision nobody made and pin the agent if that default ever changes.
    expect(toml).not.toMatch(/^mode = /m);
  });

  it("router_override round-trips all four members", () => {
    const parsed = parseManifestToml(
      `${BASE}\nrouter_override = { fixed = true, allowed_profiles = ["coding", "research"], cost_budget = "cheap", default_profile = "research" }\n`,
    );
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.model.router_fixed).toBe(true);
    expect(parsed.form.model.router_allowed_profiles).toEqual(["coding", "research"]);
    expect(parsed.form.model.router_cost_budget).toBe("cheap");
    expect(parsed.form.model.router_default_profile).toBe("research");

    const toml = serializeManifestForm(parsed.form, parsed.extras);
    expect(toml).toContain(
      'router_override = { fixed = true, allowed_profiles = ["coding", "research"], cost_budget = "cheap", default_profile = "research" }',
    );

    const reparsed = parseManifestToml(toml);
    expect(reparsed.ok).toBe(true);
    if (!reparsed.ok) return;
    expect(reparsed.form.model.router_fixed).toBe(true);
    expect(reparsed.form.model.router_allowed_profiles).toEqual(["coding", "research"]);
    expect(reparsed.form.model.router_cost_budget).toBe("cheap");
    expect(reparsed.form.model.router_default_profile).toBe("research");
  });

  it("a router_override with one member set writes only that member", () => {
    const parsed = parseManifestToml(
      `${BASE}\nrouter_override = { cost_budget = "expensive" }\n`,
    );
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;

    const toml = serializeManifestForm(parsed.form, parsed.extras);
    expect(toml).toContain('router_override = { cost_budget = "expensive" }');
    // The members with "no opinion" defaults must not re-appear: `fixed = false`
    // is what the absent key already means, and an empty allowlist means "any
    // profile is allowed" — writing them would turn silence into a decision.
    expect(toml).not.toContain("fixed =");
    expect(toml).not.toContain("allowed_profiles");
    expect(toml).not.toContain("default_profile");
  });

  it("an unset router_override is not written at all", () => {
    const form = emptyManifestForm();
    form.name = "x";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";

    const toml = serializeManifestForm(form);
    // The Rust field is `Option<AgentRouterOverride>`; an empty table would
    // configure nothing and still look configured.
    expect(toml).not.toContain("router_override");
  });

  it("an empty allowed_profiles keeps meaning every profile", () => {
    const parsed = parseManifestToml(
      `${BASE}\nrouter_override = { allowed_profiles = [] }\n`,
    );
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.model.router_allowed_profiles).toEqual([]);

    // The empty list carries no information the daemon does not already have,
    // so the empty override collapses to nothing on save.
    const toml = serializeManifestForm(parsed.form, parsed.extras);
    expect(toml).not.toContain("router_override");
  });

  // AgentRouterOverride carries no deny_unknown_fields
  // (crates/librefang-types/src/model_profile.rs), so a key the form has no
  // widget for is a legal manifest member the daemon keeps on disk. The
  // form re-emits only the four members it renders — the same silent
  // deletion response_format's preserved stash fixed, one level down.
  it("keeps an unknown key nested inside the override", () => {
    const parsed = parseManifestToml(
      `${BASE}\nrouter_override = { cost_budget = "cheap", zz_unknown = 7 }\n`,
    );
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;

    const toml = serializeManifestForm(parsed.form, parsed.extras);
    expect(toml).toContain(
      'router_override = { cost_budget = "cheap", zz_unknown = 7 }',
    );
  });

  // And the stash is the whole table when nothing else is set: an unknown
  // key alone is still content the file carried.
  it("emits the override for a preserved-only table", () => {
    const parsed = parseManifestToml(`${BASE}\nrouter_override = { zz_unknown = 7 }\n`);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.model.router_fixed).toBe(false);

    const toml = serializeManifestForm(parsed.form, parsed.extras);
    expect(toml).toContain("router_override = { zz_unknown = 7 }");
  });

  it("an empty override stays unwritten", () => {
    const form = emptyManifestForm();
    form.name = "x";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";

    const toml = serializeManifestForm(form);
    expect(toml).not.toContain("router_override");
  });

  // The two surfaces this field used to have disagreed about fixed mode: the
  // routing panel cleared allowed_profiles and cost_budget on a fixed write —
  // mirroring `set_agent_model_routing`, which rebuilds the override wholesale
  // for flexible and `None` for fixed (routes/agents/config.rs), a
  // body-shape normalisation of that route, not a file-format constraint.
  // The form writes the manifest file, where fixed alongside constraints is
  // legal and the daemon keeps it on disk. And the constraints are NOT inert
  // beside a fixed flag: a spawned child inherits the parent's
  // router_override verbatim and ungated (librefang-runtime's
  // tool_runner/agent.rs serializes parent_override into the child's model
  // block whatever the mode), so a flexible child inheriting fixed = true
  // bypasses the router with it. Preserving is the only correct choice, and
  // it matters beyond the file's own shape.
  it("keeps the constraints beside a fixed override, which children inherit ungated", () => {
    const parsed = parseManifestToml(
      `${BASE}\nrouter_override = { fixed = true, allowed_profiles = ["coding"], cost_budget = "cheap" }\n`,
    );
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.model.router_fixed).toBe(true);

    const toml = serializeManifestForm(parsed.form, parsed.extras);
    expect(toml).toContain("fixed = true");
    expect(toml).toContain('allowed_profiles = ["coding"]');
    expect(toml).toContain('cost_budget = "cheap"');
  });

  it("unknown mode and cost_budget spellings fall back to the defaults", () => {
    const parsed = parseManifestToml(
      `${BASE}\nmode = "yolo"\nrouter_override = { cost_budget = "bargain" }\n`,
    );
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.model.mode).toBe("fixed");
    expect(parsed.form.model.router_cost_budget).toBe("");
  });
});

// `[resources] burst_ratio` is `Option<f32>` clamped to 0.01..=1.0 at
// enforcement time, not at write time — so the form neither clamps a value
// nor refuses one, it just carries what the operator wrote.
describe("resources burst_ratio round-trips through the form", () => {
  const BASE = ['name = "x"', "", "[resources]", "max_llm_tokens_per_hour = 100000"].join(
    "\n",
  );

  it("round-trips a ladder rung", () => {
    const parsed = parseManifestToml(`${BASE}\nburst_ratio = 0.5\n`);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.resources.burst_ratio).toBe("0.5");

    const toml = serializeManifestForm(parsed.form, parsed.extras);
    expect(toml).toContain("burst_ratio = 0.5");

    const reparsed = parseManifestToml(toml);
    expect(reparsed.ok).toBe(true);
    if (!reparsed.ok) return;
    expect(reparsed.form.resources.burst_ratio).toBe("0.5");
  });

  it("round-trips a custom fraction the ladder does not carry", () => {
    // Below the smallest rung (0.05) but above the enforcement floor (0.01):
    // the operator's number is written back byte-identical, not snapped to a
    // preset they did not choose.
    const parsed = parseManifestToml(`${BASE}\nburst_ratio = 0.03\n`);
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.form.resources.burst_ratio).toBe("0.03");

    const toml = serializeManifestForm(parsed.form, parsed.extras);
    expect(toml).toContain("burst_ratio = 0.03");
  });

  it("writes no key when unset", () => {
    const form = emptyManifestForm();
    form.name = "x";

    const toml = serializeManifestForm(form);
    // "" is the absent key, which means the compiled default of 0.2 applies.
    expect(toml).not.toContain("burst_ratio");
  });
});

// The u32 ceiling the concurrency counter got is the shape of a family, not a
// one-off: seven more fields the form carries deserialize into `Option<u32>`
// (crates/librefang-types/src/agent.rs — heartbeat_timeout_secs :118,
// max_tokens :949, auto_dream_min_sessions :1423, compaction's max_retries
// :1649, max_loop_steps_before_aggregate :1655, strip_reasoning_after_turns
// :1658, skill_workshop's max_pending_age_days :2040), and the two token
// counts beside max_tokens are `Option<u64>`. None of them validated anything:
// a value past a field's real ceiling reached the TOML and came back as a 400
// with no field named — or, for the shapes parseInteger refuses, silently
// vanished from the file. The ceiling belongs to the field's type; where
// MODEL_PARAM_RANGES carries one for a parameter (#8332), the table is the
// source of truth.
describe("every integer count validates against its Rust type", () => {
  const U32_FIELDS: ReadonlyArray<{
    path: string;
    set: (form: ManifestFormState, v: string) => void;
  }> = [
    { path: "model.max_tokens", set: (f, v) => { f.model.max_tokens = v; } },
    {
      path: "autonomous.heartbeat_timeout_secs",
      set: (f, v) => { f.autonomous.heartbeat_timeout_secs = v; },
    },
    { path: "auto_dream_min_sessions", set: (f, v) => { f.auto_dream_min_sessions = v; } },
    { path: "compaction.max_retries", set: (f, v) => { f.compaction.max_retries = v; } },
    {
      path: "compaction.max_loop_steps_before_aggregate",
      set: (f, v) => { f.compaction.max_loop_steps_before_aggregate = v; },
    },
    {
      path: "compaction.strip_reasoning_after_turns",
      set: (f, v) => { f.compaction.strip_reasoning_after_turns = v; },
    },
    {
      path: "skill_workshop.max_pending_age_days",
      set: (f, v) => { f.skill_workshop.max_pending_age_days = v; },
    },
  ];

  for (const { path, set } of U32_FIELDS) {
    it(`${path} rejects a value past u32::MAX`, () => {
      const form = emptyManifestForm();
      form.name = "x";
      set(form, "4294967296");
      expect(validateManifestForm(form)).toContain(path);
    });

    it(`${path} accepts u32::MAX itself`, () => {
      const form = emptyManifestForm();
      form.name = "x";
      set(form, "4294967295");
      expect(validateManifestForm(form)).not.toContain(path);
    });
  }

  // `context_window` and `max_output_tokens` are `Option<u64>` — no typo
  // reaches their ceiling through TOML, whose integers stop at the signed
  // 64-bit bound. Their defect is the silent drop instead: a negative or a
  // non-integer passed the validator and parseInteger made the key vanish.
  for (const [param, key] of [
    ["context_window", "model.context_window"],
    ["max_output_tokens", "model.max_output_tokens"],
  ] as const) {
    it(`${key} reports a negative instead of dropping it`, () => {
      const form = emptyManifestForm();
      form.name = "x";
      form.model[param] = "-5";
      expect(validateManifestForm(form)).toContain(key);
    });

    it(`${key} reports a non-integer instead of dropping it`, () => {
      const form = emptyManifestForm();
      form.name = "x";
      form.model[param] = "1.5";
      expect(validateManifestForm(form)).toContain(key);
    });

    // The ceiling is not global: the u64 field takes the value that caps its
    // u32 sibling without complaint.
    it(`${key} stays valid at u32::MAX + 1`, () => {
      const form = emptyManifestForm();
      form.name = "x";
      form.model[param] = "4294967296";
      expect(validateManifestForm(form)).not.toContain(key);
    });

    it(`${key} round-trips past JavaScript's safe integer range`, () => {
      const parsed = parseManifestToml(
        `name = "a"\n\n[model]\nprovider = "openai"\nmodel = "gpt-4o"\ncontext_window = 9223372036854775806\n`,
      );
      expect(parsed.ok).toBe(true);
      if (!parsed.ok) return;

      const round = serializeManifestForm(parsed.form, parsed.extras);
      expect(round).toContain("context_window = 9223372036854775806");
    });
  }
});
