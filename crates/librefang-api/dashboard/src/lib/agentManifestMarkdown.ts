// Renders a ManifestFormState (+ extras) as human-readable Markdown for
// docs / code-review / sharing. The output mirrors what the form shows
// rather than the literal TOML — so an "enabled at startup" toggle reads
// as a checkmark, capability arrays render as comma-separated lists, etc.
//
// Fields preserved as `extras` (advanced TOML-only sections) are listed
// at the end under an "Advanced configuration" appendix so reviewers can
// see them without having to read raw TOML.

import { emptyManifestExtras } from "./agentManifest";
import type { ManifestExtras, ManifestFormState } from "./agentManifest";

const escapeTableCell = (value: string): string =>
  value
    .replace(/\\/g, "\\\\")
    .replace(/`/g, "\\`")
    .replace(/\|/g, "\\|")
    .replace(/\r\n|\r|\n/g, " ");

const longestBacktickRun = (content: string): number => {
  let longestRun = 0;
  for (const match of content.matchAll(/`+/g)) {
    longestRun = Math.max(longestRun, match[0].length);
  }
  return longestRun;
};

const markdownCodeSpan = (content: string): string => {
  const longestRun = longestBacktickRun(content);
  const fence = "`".repeat(longestRun + 1);
  const hasBoundarySpaces = content.startsWith(" ") && content.endsWith(" ");
  const needsPadding =
    content.startsWith("`") ||
    content.endsWith("`") ||
    (hasBoundarySpaces && content.trim() !== "");
  const padding = needsPadding ? " " : "";
  return `${fence}${padding}${content}${padding}${fence}`;
};

const pushFencedBlock = (lines: string[], content: string, language = ""): void => {
  const longestRun = longestBacktickRun(content);
  const fence = "`".repeat(Math.max(3, longestRun + 1));
  lines.push(`${fence}${language}`, content, fence);
};

const compactBlankLineElements = (lines: string[]): string[] => {
  const compacted: string[] = [];
  for (const line of lines) {
    if (line === "" && compacted[compacted.length - 1] === "") continue;
    compacted.push(line);
  }
  return compacted;
};

export const generateManifestMarkdown = (
  form: ManifestFormState,
  extras: ManifestExtras = emptyManifestExtras(),
): string => {
  const lines: string[] = [];
  const name = form.name.trim() || "(unnamed agent)";

  lines.push(`# ${name}${form.version ? ` v${form.version.trim()}` : ""}`);
  lines.push("");

  if (form.description.trim()) {
    lines.push(`> ${form.description.trim()}`);
    lines.push("");
  }

  const meta: string[] = [];
  if (form.author.trim()) meta.push(`**Author**: ${form.author.trim()}`);
  if (form.module.trim()) meta.push(`**Module**: ${markdownCodeSpan(form.module.trim())}`);
  if (form.tags.length) {
    meta.push(`**Tags**: ${form.tags.map(markdownCodeSpan).join(" ")}`);
  }
  meta.push(`**Enabled**: ${form.enabled ? "✓" : "✗"}`);
  if (meta.length) {
    lines.push(meta.join("  \n"));
    lines.push("");
  }

  // Model
  lines.push("## Model");
  lines.push("");
  pushBullet(lines, "Provider", form.model.provider);
  pushBullet(lines, "Model", form.model.model);
  pushBullet(lines, "Temperature", form.model.temperature);
  pushBullet(lines, "Max tokens", form.model.max_tokens);
  if (form.model.system_prompt.trim()) {
    lines.push("");
    lines.push("### System Prompt");
    lines.push("");
    pushFencedBlock(lines, form.model.system_prompt.trim());
  }
  lines.push("");

  // Resources — only emit the section if at least one limit is set.
  const resourceRows: [string, string][] = [
    ["LLM tokens / hour", form.resources.max_llm_tokens_per_hour],
    ["Tool calls / minute", form.resources.max_tool_calls_per_minute],
    ["Max cost / hour", formatCost(form.resources.max_cost_per_hour_usd)],
    ["Max cost / day", formatCost(form.resources.max_cost_per_day_usd)],
  ].filter(([, v]) => v.trim() !== "") as [string, string][];
  if (resourceRows.length) {
    lines.push("## Resources");
    lines.push("");
    lines.push("| Limit | Value |");
    lines.push("|-------|-------|");
    for (const [k, v] of resourceRows) {
      lines.push(`| ${escapeTableCell(k)} | ${escapeTableCell(v)} |`);
    }
    lines.push("");
  }

  // Capabilities — only emit if anything is set.
  const capLines: string[] = [];
  if (form.capabilities.network.length) {
    capLines.push(`- **Network**: ${form.capabilities.network.join(", ")}`);
  }
  if (form.capabilities.shell.length) {
    capLines.push(`- **Shell commands**: ${form.capabilities.shell.join(", ")}`);
  }
  if (form.capabilities.tools.length) {
    capLines.push(`- **Tools**: ${form.capabilities.tools.join(", ")}`);
  }
  if (form.capabilities.agent_spawn) capLines.push("- ✓ Can spawn sub-agents");
  if (form.capabilities.ofp_discover) capLines.push("- ✓ Can discover OFP peers");
  if (capLines.length) {
    lines.push("## Capabilities");
    lines.push("");
    for (const l of capLines) lines.push(l);
    lines.push("");
  }

  pushList(lines, "Skills", form.skills);
  pushList(lines, "MCP servers", form.mcp_servers);
  pushList(lines, "Tool allowlist", form.tool_allowlist);
  pushList(lines, "Tool blocklist", form.tool_blocklist);

  pushAdvancedFormSections(lines, form);
  pushLifecycleOverrides(lines, form);

  // Advanced — anything in extras that survived round-trip.
  const advancedLines = renderExtras(extras);
  if (advancedLines.length) {
    lines.push("## Advanced configuration");
    lines.push("");
    lines.push(
      "_Fields below are preserved from the TOML editor; they have no first-class form widget yet._",
    );
    lines.push("");
    for (const l of advancedLines) lines.push(l);
    lines.push("");
  }

  return compactBlankLineElements(lines).join("\n").trimEnd() + "\n";
};

// Render the advanced first-class form fields when populated. Keeping
// these here (rather than in the extras appendix) means a generated
// Markdown for an autonomous-mode agent actually documents that it's
// autonomous, instead of silently omitting the gating config.
const pushAdvancedFormSections = (lines: string[], form: ManifestFormState): void => {
  if (form.schedule.mode !== "reactive") {
    lines.push("## Schedule");
    lines.push("");
    lines.push(`- **Mode**: ${markdownCodeSpan(form.schedule.mode)}`);
    if (form.schedule.mode === "periodic") {
      lines.push(`- **Cron**: ${markdownCodeSpan(form.schedule.cron)}`);
    } else if (form.schedule.mode === "proactive") {
      if (form.schedule.conditions.length) {
        lines.push(`- **Conditions**: ${form.schedule.conditions.map(markdownCodeSpan).join(", ")}`);
      }
    } else if (form.schedule.mode === "continuous") {
      lines.push(`- **Check interval**: ${markdownCodeSpan(`${form.schedule.check_interval_secs}s`)}`);
    } else {
      lines.push("- _Details for this schedule mode are not available._");
    }
    lines.push("");
  }

  if (form.fallback_models.length) {
    lines.push("## Fallback Models");
    lines.push("");
    lines.push("| # | Provider | Model |");
    lines.push("|---|----------|-------|");
    form.fallback_models.forEach((fb, i) => {
      lines.push(`| ${i + 1} | ${escapeTableCell(fb.provider || "_(empty)_")} | ${escapeTableCell(fb.model || "_(empty)_")} |`);
    });
    lines.push("");
  }

  if (form.thinking.enabled) {
    lines.push("## Extended Thinking");
    lines.push("");
    pushBullet(lines, "Budget tokens", form.thinking.budget_tokens);
    lines.push(`- **Stream thinking**: ${form.thinking.stream_thinking ? "✓" : "✗"}`);
    lines.push("");
  }

  if (form.autonomous.enabled) {
    lines.push("## Autonomous Guardrails");
    lines.push("");
    pushBullet(lines, "Max iterations", form.autonomous.max_iterations);
    pushBullet(lines, "Max restarts", form.autonomous.max_restarts);
    pushBullet(lines, "Heartbeat interval", form.autonomous.heartbeat_interval_secs);
    pushBullet(lines, "Heartbeat timeout", form.autonomous.heartbeat_timeout_secs);
    pushBullet(lines, "Heartbeat keep recent", form.autonomous.heartbeat_keep_recent);
    pushBullet(lines, "Heartbeat channel", form.autonomous.heartbeat_channel);
    pushBullet(lines, "Quiet hours", form.autonomous.quiet_hours);
    lines.push("");
  }

  if (form.routing.enabled) {
    lines.push("## Model Routing");
    lines.push("");
    pushBullet(lines, "Simple", form.routing.simple_model);
    pushBullet(lines, "Medium", form.routing.medium_model);
    pushBullet(lines, "Complex", form.routing.complex_model);
    pushBullet(lines, "Simple threshold", form.routing.simple_threshold);
    pushBullet(lines, "Complex threshold", form.routing.complex_threshold);
    lines.push("");
  }

  if (form.context_injection.length) {
    lines.push("## Context Injections");
    lines.push("");
    form.context_injection.forEach((ci, i) => {
      lines.push(`**${i + 1}. ${ci.name || "(unnamed)"}** _(${ci.position})_`);
      if (ci.condition) lines.push(`- **Condition**: ${markdownCodeSpan(ci.condition)}`);
      lines.push("");
      pushFencedBlock(lines, ci.content);
      lines.push("");
    });
  }

  if (form.response_format.mode !== "text") {
    lines.push("## Response Format");
    lines.push("");
    lines.push(`- **Mode**: \`${form.response_format.mode}\``);
    if (form.response_format.mode === "json_schema") {
      pushBullet(lines, "Schema name", form.response_format.name);
      lines.push(`- **Strict**: ${form.response_format.strict ? "✓" : "✗"}`);
      if (form.response_format.schema.trim()) {
        lines.push("");
        pushFencedBlock(lines, form.response_format.schema, "json");
      }
    }
    lines.push("");
  }
};

// Lifecycle overrides — only emit values that differ from kernel defaults,
// so a vanilla agent stays clean and an unusual config stands out.
const pushLifecycleOverrides = (lines: string[], form: ManifestFormState): void => {
  const items: string[] = [];
  if (form.priority !== "Normal") {
    items.push(`- **Priority**: ${markdownCodeSpan(form.priority)}`);
  }
  if (form.session_mode !== "persistent") {
    items.push(`- **session_mode**: ${markdownCodeSpan(form.session_mode)}`);
  }
  if (form.web_search_augmentation !== "auto") {
    items.push(`- **web_search_augmentation**: ${markdownCodeSpan(form.web_search_augmentation)}`);
  }
  if (form.exec_policy_shorthand) {
    items.push(`- **exec_policy**: ${markdownCodeSpan(form.exec_policy_shorthand)}`);
  }
  if (form.pinned_model.trim()) {
    items.push(`- **Pinned model**: ${markdownCodeSpan(form.pinned_model.trim())}`);
  }
  if (form.workspace.trim()) {
    items.push(`- **Workspace**: ${markdownCodeSpan(form.workspace.trim())}`);
  }
  if (form.allowed_plugins.length) {
    items.push(`- **Allowed plugins**: ${form.allowed_plugins.map(markdownCodeSpan).join(", ")}`);
  }
  if (form.skills_disabled) items.push("- ⚠️ **Skills disabled**");
  if (form.tools_disabled) items.push("- ⚠️ **Tools disabled**");
  if (!form.inherit_parent_context) {
    items.push("- **inherit_parent_context**: `false`");
  }
  if (!form.generate_identity_files) {
    items.push("- **generate_identity_files**: `false`");
  }

  if (items.length === 0) return;
  lines.push("## Lifecycle & Overrides");
  lines.push("");
  for (const item of items) lines.push(item);
  lines.push("");
};

const pushBullet = (lines: string[], label: string, value: string): void => {
  if (!value.trim()) return;
  lines.push(`- **${label}**: ${value.trim()}`);
};

const pushList = (lines: string[], heading: string, items: string[]): void => {
  if (items.length === 0) return;
  lines.push(`## ${heading}`);
  lines.push("");
  for (const item of items) lines.push(`- ${item}`);
  lines.push("");
};

const formatCost = (raw: string): string => {
  const trimmed = raw.trim();
  if (!trimmed) return "";
  if (!/^(?:\d+(?:\.\d*)?|\.\d+)$/.test(trimmed)) return trimmed;
  const n = Number(trimmed);
  if (!Number.isFinite(n)) return trimmed;
  return `$${n.toFixed(2)}`;
};

const renderExtras = (extras: ManifestExtras): string[] => {
  const lines: string[] = [];
  const renderTable = (label: string, table: Record<string, unknown>): void => {
    const entries = Object.entries(table);
    if (entries.length === 0) return;
    lines.push(`### ${label}`);
    lines.push("");
    for (const [key, value] of entries) {
      lines.push(`- ${markdownCodeSpan(key)} = ${stringifyExtraValue(value)}`);
    }
    lines.push("");
  };

  // Top-level extras: split scalars (rendered first) from sub-tables/arrays.
  const topInline: Record<string, unknown> = {};
  const topNested: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(extras.topLevel)) {
    if (isPlainObject(v) || isArrayOfObjects(v)) topNested[k] = v;
    else topInline[k] = v;
  }
  renderTable("Top-level overrides", topInline);
  renderTable("`[model]` extras", extras.model);
  renderTable("`[resources]` extras", extras.resources);
  renderTable("`[capabilities]` extras", extras.capabilities);
  for (const [key, value] of Object.entries(topNested)) {
    if (isArrayOfObjects(value)) {
      lines.push(`### ${markdownCodeSpan(`[[${key}]]`)}`);
      lines.push("");
      const arr = value as Record<string, unknown>[];
      for (let i = 0; i < arr.length; i++) {
        lines.push(`**[${i}]**`);
        for (const [k, v] of Object.entries(arr[i])) {
          lines.push(`- ${markdownCodeSpan(k)} = ${stringifyExtraValue(v)}`);
        }
        lines.push("");
      }
    } else if (isPlainObject(value)) {
      renderTable(markdownCodeSpan(`[${key}]`), value as Record<string, unknown>);
    }
  }
  return lines;
};

const isPlainObject = (v: unknown): v is Record<string, unknown> =>
  typeof v === "object" && v !== null && !Array.isArray(v);

const isArrayOfObjects = (v: unknown): boolean =>
  Array.isArray(v) && v.length > 0 && v.every(isPlainObject);

const stringifyExtraValue = (value: unknown): string => {
  if (value === null || value === undefined) return "_(empty)_";
  if (typeof value === "string") return markdownCodeSpan(`"${value}"`);
  if (typeof value === "boolean" || typeof value === "number" || typeof value === "bigint") {
    return markdownCodeSpan(String(value));
  }
  try {
    const serialized = JSON.stringify(value);
    return serialized === undefined ? "_(unrenderable)_" : markdownCodeSpan(serialized);
  } catch {
    return "_(unrenderable)_";
  }
};
