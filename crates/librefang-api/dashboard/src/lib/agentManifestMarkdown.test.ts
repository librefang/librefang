import { describe, expect, it } from "vitest";
import {
  emptyManifestExtras,
  emptyManifestForm,
} from "./agentManifest";
import { generateManifestMarkdown } from "./agentManifestMarkdown";

describe("generateManifestMarkdown", () => {
  it("renders a minimum-viable agent", () => {
    const form = emptyManifestForm();
    form.name = "researcher";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";

    const md = generateManifestMarkdown(form);

    expect(md).toContain("# researcher v1.0.0");
    expect(md).toContain("## Model");
    expect(md).toContain("**Provider**: openai");
    expect(md).toContain("**Model**: gpt-4o");
    // Empty resource/capability sections are omitted entirely.
    expect(md).not.toContain("## Resources");
    expect(md).not.toContain("## Capabilities");
    expect(md).not.toContain("## Skills");
  });

  it("includes description, tags, and system prompt", () => {
    const form = emptyManifestForm();
    form.name = "ops";
    form.description = "monitors deploys";
    form.tags = ["beta", "ops"];
    form.author = "evan";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    form.model.system_prompt = "You watch the deploys.";

    const md = generateManifestMarkdown(form);

    expect(md).toContain("> monitors deploys");
    expect(md).toContain("**Tags**: `beta` `ops`");
    expect(md).toContain("**Author**: evan");
    expect(md).toContain("### System Prompt");
    expect(md).toContain("You watch the deploys.");
  });

  it("renders resources as a table when set", () => {
    const form = emptyManifestForm();
    form.name = "agent";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    form.resources.max_cost_per_hour_usd = "1.5";
    form.resources.max_tool_calls_per_minute = "30";

    const md = generateManifestMarkdown(form);

    expect(md).toContain("## Resources");
    expect(md).toContain("| Limit | Value |");
    expect(md).toContain("| Max cost / hour | $1.50 |");
    expect(md).toContain("| Tool calls / minute | 30 |");
  });

  it("renders capabilities and lists when populated", () => {
    const form = emptyManifestForm();
    form.name = "agent";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    form.capabilities.network = ["api.openai.com:443"];
    form.capabilities.agent_spawn = true;
    form.skills = ["coder", "search"];
    form.mcp_servers = ["filesystem"];

    const md = generateManifestMarkdown(form);

    expect(md).toContain("## Capabilities");
    expect(md).toContain("- **Network**: api.openai.com:443");
    expect(md).toContain("- ✓ Can spawn sub-agents");
    expect(md).toContain("## Skills");
    expect(md).toContain("- coder");
    expect(md).toContain("- search");
    expect(md).toContain("## MCP servers");
    expect(md).toContain("- filesystem");
  });

  it("appends an Advanced section when extras are present", () => {
    const form = emptyManifestForm();
    form.name = "agent";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    const extras = emptyManifestExtras();
    extras.topLevel.priority = "high";
    extras.topLevel.thinking = { budget_tokens: 5000 };
    extras.model.api_key_env = "OPENAI_API_KEY";

    const md = generateManifestMarkdown(form, extras);

    expect(md).toContain("## Advanced configuration");
    expect(md).toContain("### Top-level overrides");
    expect(md).toContain('- `priority` = `"high"`');
    expect(md).toContain("### `[model]` extras");
    expect(md).toContain('- `api_key_env` = `"OPENAI_API_KEY"`');
    expect(md).toContain("### `[thinking]`");
    expect(md).toContain("- `budget_tokens` = `5000`");
  });

  it("flags disabled agents", () => {
    const form = emptyManifestForm();
    form.name = "agent";
    form.enabled = false;
    form.model.provider = "openai";
    form.model.model = "gpt-4o";

    const md = generateManifestMarkdown(form);
    expect(md).toContain("**Enabled**: ✗");
  });

  it("falls back to a placeholder name when blank", () => {
    const form = emptyManifestForm();
    const md = generateManifestMarkdown(form);
    expect(md).toContain("# (unnamed agent)");
  });
});
