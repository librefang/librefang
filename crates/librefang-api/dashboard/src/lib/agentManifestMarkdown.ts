// Renders a ManifestFormState (+ extras) as human-readable Markdown for
// docs / code-review / sharing. The output mirrors what the form shows
// rather than the literal TOML — so an "enabled at startup" toggle reads
// as a checkmark, capability arrays render as comma-separated lists, etc.
//
// Fields preserved as `extras` (advanced TOML-only sections) are listed
// at the end under an "Advanced configuration" appendix so reviewers can
// see them without having to read raw TOML.

import type { ManifestExtras, ManifestFormState } from "./agentManifest";

export const generateManifestMarkdown = (
  form: ManifestFormState,
  extras: ManifestExtras = {
    topLevel: {},
    model: {},
    resources: {},
    capabilities: {},
  },
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
  if (form.module.trim()) meta.push(`**Module**: \`${form.module.trim()}\``);
  if (form.tags.length) {
    meta.push(`**Tags**: ${form.tags.map((t) => `\`${t}\``).join(" ")}`);
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
    lines.push("```");
    lines.push(form.model.system_prompt.trim());
    lines.push("```");
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
      lines.push(`| ${k} | ${v} |`);
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

  return lines.join("\n").replace(/\n{3,}/g, "\n\n").trimEnd() + "\n";
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
      lines.push(`- \`${key}\` = ${stringifyExtraValue(value)}`);
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
      lines.push(`### \`[[${key}]]\``);
      lines.push("");
      const arr = value as Record<string, unknown>[];
      for (let i = 0; i < arr.length; i++) {
        lines.push(`**[${i}]**`);
        for (const [k, v] of Object.entries(arr[i])) {
          lines.push(`- \`${k}\` = ${stringifyExtraValue(v)}`);
        }
        lines.push("");
      }
    } else if (isPlainObject(value)) {
      renderTable(`\`[${key}]\``, value as Record<string, unknown>);
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
  if (typeof value === "string") return `\`"${value}"\``;
  if (typeof value === "boolean" || typeof value === "number" || typeof value === "bigint") {
    return `\`${String(value)}\``;
  }
  try {
    return `\`${JSON.stringify(value)}\``;
  } catch {
    return "_(unrenderable)_";
  }
};
