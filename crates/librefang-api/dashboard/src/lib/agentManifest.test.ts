import { describe, expect, it } from "vitest";
import {
  emptyManifestForm,
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
    // Empty optional sections should not appear.
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
    // Leave max_llm_tokens_per_hour blank — should be omitted entirely
    // so the kernel falls back to its default (None / inherit global).

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
    form.model.max_tokens = "1.5"; // not an integer

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
