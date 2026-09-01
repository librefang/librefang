import { describe, expect, it } from "vitest";
import {
  emptyManifestExtras,
  emptyManifestForm,
  parseManifestToml,
  serializeManifestForm,
  validateManifestForm,
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
  it("flags missing name and model fields", () => {
    const errors = validateManifestForm(emptyManifestForm());
    expect(errors).toContain("name");
    expect(errors).toContain("model.provider");
    expect(errors).toContain("model.model");
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

    expect(first.form.fallback_models[0]._uid).toBe("parsed-1");
    expect(first.form.context_injection[0]._uid).toBe("parsed-2");
    expect(second.form.fallback_models[0]._uid).toBe("parsed-1");
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
    expect(result.form.fallback_models.map(({ _uid, ...rest }) => rest)).toEqual([
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
    expect(parsed.form.fallback_models[0].extras).toEqual({
      enable_memory: true,
      custom_param: "preserved",
    });
    const reserialized = serializeManifestForm(parsed.form, parsed.extras);
    const reparsed = parseManifestToml(reserialized);
    expect(reparsed.ok).toBe(true);
    if (!reparsed.ok) return;
    expect(reparsed.form.fallback_models[0].extras).toEqual({
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
      fallback_models: stripUids(f.fallback_models),
      context_injection: stripUids(f.context_injection),
    });
    expect(cleanForm(reparsed.form)).toEqual(cleanForm(parsed.form));
    expect(reparsed.extras).toEqual(parsed.extras);
  });
});
