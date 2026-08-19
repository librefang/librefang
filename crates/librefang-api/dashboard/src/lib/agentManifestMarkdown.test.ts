import { describe, expect, it } from "vitest";
import {
  emptyManifestExtras,
  emptyManifestForm,
  type ManifestFormState,
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

  it("renders advanced first-class fields when populated", () => {
    const form = emptyManifestForm();
    form.name = "auto";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    form.schedule = { mode: "periodic", cron: "0 9 * * *" };
    form.fallback_models = [
      { _uid: "test-fb-1", provider: "anthropic", model: "claude-3-5-sonnet", api_key_env: "", base_url: "", extras: {} },
    ];
    form.thinking = { enabled: true, budget_tokens: "5000", stream_thinking: true };
    form.autonomous = {
      enabled: true,
      max_iterations: "100",
      max_restarts: "10",
      heartbeat_interval_secs: "30",
      heartbeat_timeout_secs: "",
      heartbeat_keep_recent: "",
      heartbeat_channel: "telegram",
      quiet_hours: "",
    };
    form.routing = {
      enabled: true,
      simple_model: "claude-haiku",
      medium_model: "claude-sonnet",
      complex_model: "claude-opus",
      simple_threshold: "100",
      complex_threshold: "500",
    };
    form.context_injection = [
      { _uid: "test-ci-1", name: "policy", content: "Be polite.", position: "before_user", condition: "" },
    ];
    form.response_format = { mode: "json" };

    const md = generateManifestMarkdown(form);

    expect(md).toContain("## Schedule");
    expect(md).toContain("0 9 * * *");
    expect(md).toContain("## Fallback Models");
    expect(md).toContain("anthropic");
    expect(md).toContain("claude-3-5-sonnet");
    expect(md).toContain("## Extended Thinking");
    expect(md).toContain("5000");
    expect(md).toContain("## Autonomous Guardrails");
    expect(md).toContain("telegram");
    expect(md).toContain("## Model Routing");
    expect(md).toContain("claude-haiku");
    expect(md).toContain("## Context Injections");
    expect(md).toContain("Be polite.");
    expect(md).toContain("## Response Format");
    expect(md).toContain("json");
  });

  it("includes lifecycle overrides when set to non-default values", () => {
    const form = emptyManifestForm();
    form.name = "ops";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    form.priority = "Critical";
    form.session_mode = "new";
    form.web_search_augmentation = "always";
    form.exec_policy_shorthand = "deny";
    form.pinned_model = "gpt-4o-2024-05-13";
    form.workspace = "/var/agents/ops";
    form.allowed_plugins = ["telegram"];
    form.skills_disabled = true;
    form.inherit_parent_context = false;

    const md = generateManifestMarkdown(form);

    expect(md).toContain("## Lifecycle & Overrides");
    expect(md).toContain("Critical");
    expect(md).toContain("session_mode");
    expect(md).toContain("new");
    expect(md).toContain("always");
    expect(md).toContain("deny");
    expect(md).toContain("gpt-4o-2024-05-13");
    expect(md).toContain("/var/agents/ops");
    expect(md).toContain("telegram");
    expect(md).toContain("Skills disabled");
  });

  it("does not emit lifecycle section when everything is default", () => {
    const form = emptyManifestForm();
    form.name = "plain";
    form.model.provider = "openai";
    form.model.model = "gpt-4o";
    const md = generateManifestMarkdown(form);
    expect(md).not.toContain("## Lifecycle & Overrides");
  });

  it("falls back to a placeholder name when blank", () => {
    const form = emptyManifestForm();
    const md = generateManifestMarkdown(form);
    expect(md).toContain("# (unnamed agent)");
  });

  it("escapes table delimiters and normalizes cell newlines", () => {
    const form = emptyManifestForm();
    form.resources.max_tool_calls_per_minute = "30|40\n50";
    form.fallback_models = [{
      _uid: "fallback-1",
      provider: "open|`router",
      model: "line-one\nline-two",
      api_key_env: "",
      base_url: "",
      extras: {},
    }];

    const md = generateManifestMarkdown(form);

    expect(md).toContain("| Tool calls / minute | 30\\|40 50 |");
    expect(md).toContain("| 1 | open\\|\\`router | line-one line-two |");
  });

  it("chooses code fences longer than embedded backtick runs", () => {
    const form = emptyManifestForm();
    form.model.system_prompt = "before\n```\nafter";
    form.context_injection = [{
      _uid: "context-1",
      name: "code",
      content: "inner ```` fence",
      position: "before_user",
      condition: "",
    }];
    form.response_format = {
      mode: "json_schema",
      name: "schema",
      strict: true,
      schema: '{"example":"```"}',
    };

    const md = generateManifestMarkdown(form);

    expect(md).toContain("````\nbefore\n```\nafter\n````");
    expect(md).toContain("`````\ninner ```` fence\n`````");
    expect(md).toContain("````json\n{\"example\":\"```\"}\n````");
  });

  it("preserves repeated blank lines inside fenced content", () => {
    const form = emptyManifestForm();
    form.model.system_prompt = "before\n\n\nafter";
    form.context_injection = [{
      _uid: "context-1",
      name: "spacing",
      content: "first\n\n\n\nsecond",
      position: "before_user",
      condition: "",
    }];

    const md = generateManifestMarkdown(form);

    expect(md).toContain("```\nbefore\n\n\nafter\n```");
    expect(md).toContain("```\nfirst\n\n\n\nsecond\n```");
  });

  it("uses collision-resistant code spans for extras containing backticks", () => {
    const form = emptyManifestForm();
    const extras = emptyManifestExtras();
    extras.topLevel.note = "before `code` after";

    expect(generateManifestMarkdown(form, extras)).toContain(
      '- `note` = ``"before `code` after"``',
    );
  });

  it("uses collision-resistant code spans for first-class inline values and extra keys", () => {
    const form = emptyManifestForm();
    const extras = emptyManifestExtras();
    form.module = "builtin:`chat`";
    form.schedule = { mode: "periodic", cron: "`*/5 * * * *`" };
    extras.topLevel["tick`rate"] = 5;

    const md = generateManifestMarkdown(form, extras);

    expect(md).toContain("**Module**: `` builtin:`chat` ``");
    expect(md).toContain("**Cron**: `` `*/5 * * * *` ``");
    expect(md).toContain("- ``tick`rate`` = `5`");
  });

  it("pads code spans so CommonMark preserves boundary spaces", () => {
    const form = emptyManifestForm();
    const extras = emptyManifestExtras();
    form.tags = [" padded "];
    extras.topLevel[" spaced "] = true;

    const md = generateManifestMarkdown(form, extras);

    expect(md).toContain("**Tags**: `  padded  `");
    expect(md).toContain("- `  spaced  ` = `true`");
  });

  it("handles many separate backtick runs without expanding them as function arguments", () => {
    const form = emptyManifestForm();
    form.model.system_prompt = "x`".repeat(150_000);

    expect(() => generateManifestMarkdown(form)).not.toThrow();
  });

  it("renders unsupported extra values as unrenderable", () => {
    const form = emptyManifestForm();
    const extras = emptyManifestExtras();
    (extras.topLevel as Record<string, unknown>).unsupported = Symbol("unsupported");

    expect(generateManifestMarkdown(form, extras)).toContain(
      "- `unsupported` = _(unrenderable)_",
    );
  });

  it("formats decimal costs only", () => {
    const form = emptyManifestForm();
    form.resources.max_cost_per_hour_usd = ".5";
    form.resources.max_cost_per_day_usd = "1.";

    const decimalMd = generateManifestMarkdown(form);

    expect(decimalMd).toContain("| Max cost / hour | $0.50 |");
    expect(decimalMd).toContain("| Max cost / day | $1.00 |");

    form.resources.max_cost_per_hour_usd = "1e2";
    form.resources.max_cost_per_day_usd = "0x10";

    const nonDecimalMd = generateManifestMarkdown(form);

    expect(nonDecimalMd).toContain("| Max cost / hour | 1e2 |");
    expect(nonDecimalMd).toContain("| Max cost / day | 0x10 |");
  });

  it("records that an unknown schedule mode has no known details", () => {
    const form = emptyManifestForm();
    form.schedule = { mode: "future" } as unknown as ManifestFormState["schedule"];

    expect(generateManifestMarkdown(form)).toContain(
      "Details for this schedule mode are not available.",
    );
  });
});
