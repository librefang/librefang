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
    extras.topLevel.priority = "high";
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
    expect(toml).toContain('priority = "high"');
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

  it("preserves unknown sections as extras", () => {
    const toml = `name = "agent"
priority = "high"

[model]
provider = "openai"
model = "gpt-4o"
api_key_env = "OPENAI_API_KEY"

[capabilities]
memory_read = ["user/*"]

[thinking]
budget_tokens = 10000
stream_thinking = false

[autonomous]
max_iterations = 100
`;
    const result = parseManifestToml(toml);
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.extras.topLevel.priority).toBe("high");
    expect(result.extras.topLevel.thinking).toEqual({
      budget_tokens: 10000,
      stream_thinking: false,
    });
    expect(result.extras.topLevel.autonomous).toEqual({ max_iterations: 100 });
    expect(result.extras.model.api_key_env).toBe("OPENAI_API_KEY");
    expect(result.extras.capabilities.memory_read).toEqual(["user/*"]);
  });

  it("returns a structured error on malformed TOML", () => {
    const result = parseManifestToml('name = "unterminated\n[oops');
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.message.length).toBeGreaterThan(0);
  });

  it("round-trips: serialize(parse(toml)) preserves form + extras", () => {
    const original = `name = "agent"
description = "test"
priority = "high"

[model]
provider = "openai"
model = "gpt-4o"
temperature = 0.5
api_key_env = "OPENAI_API_KEY"

[resources]
max_cost_per_hour_usd = 2

[capabilities]
network = ["api.openai.com:443"]
memory_read = ["user/*"]

[thinking]
budget_tokens = 5000
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
