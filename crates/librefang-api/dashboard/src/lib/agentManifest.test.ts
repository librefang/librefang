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
});

describe("agentManifest parser", () => {
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
    expect(result.form.fallback_models).toEqual([
      {
        provider: "anthropic",
        model: "claude-3-5-sonnet",
        api_key_env: "",
        base_url: "",
        extras: {},
      },
    ]);
    expect(result.form.context_injection).toEqual([
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

  it("rejects negative and out-of-range integers in number fields", () => {
    // Codex P2 regression: parseInteger used to accept any JS number,
    // including negatives (which u32/u64 deserializers reject) and
    // values above MAX_SAFE_INTEGER (which lose precision before
    // serialization).
    const form = emptyManifestForm();
    form.name = "a";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    form.model.max_tokens = "-100";
    form.resources.max_llm_tokens_per_hour = "9999999999999999999"; // > MAX_SAFE_INTEGER

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
    expect(reparsed.form).toEqual(parsed.form);
    expect(reparsed.extras).toEqual(parsed.extras);
  });
});
