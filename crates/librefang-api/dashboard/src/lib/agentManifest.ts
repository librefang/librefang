// Structured representation of the subset of AgentManifest fields exposed
// in the visual editor.
//
// The form covers the common 20% (basics, model, resources, capabilities,
// discovery). Anything else lives in `extras` so a TOML-tab edit that
// adds [thinking], [autonomous], [tools.foo], etc. survives a round-trip
// back through the form. The serializer emits form fields first in a
// stable layout, then appends extras using smol-toml's stringify.

import { parse, stringify, TomlError, type TomlTable } from "smol-toml";

export interface ManifestFormState {
  name: string;
  description: string;
  version: string;
  author: string;
  module: string;

  model: {
    provider: string;
    model: string;
    system_prompt: string;
    temperature: string;
    max_tokens: string;
  };

  resources: {
    max_llm_tokens_per_hour: string;
    max_tool_calls_per_minute: string;
    max_cost_per_hour_usd: string;
    max_cost_per_day_usd: string;
  };

  capabilities: {
    network: string[];
    shell: string[];
    tools: string[];
    agent_spawn: boolean;
    ofp_discover: boolean;
  };

  skills: string[];
  mcp_servers: string[];
  tags: string[];
  tool_allowlist: string[];
  tool_blocklist: string[];

  enabled: boolean;
}

// Non-form-owned fields preserved through round-trips. Top-level keys
// that the form doesn't manage live under `topLevel`; sub-table extras
// (e.g. extra_params under [model], memory_read under [capabilities])
// live under their respective namespace.
export interface ManifestExtras {
  topLevel: TomlTable;
  model: TomlTable;
  resources: TomlTable;
  capabilities: TomlTable;
}

export const emptyManifestExtras = (): ManifestExtras => ({
  topLevel: {},
  model: {},
  resources: {},
  capabilities: {},
});

export const emptyManifestForm = (): ManifestFormState => ({
  name: "",
  description: "",
  version: "1.0.0",
  author: "",
  module: "builtin:chat",
  model: {
    provider: "",
    model: "",
    system_prompt: "",
    temperature: "",
    max_tokens: "",
  },
  resources: {
    max_llm_tokens_per_hour: "",
    max_tool_calls_per_minute: "",
    max_cost_per_hour_usd: "",
    max_cost_per_day_usd: "",
  },
  capabilities: {
    network: [],
    shell: [],
    tools: [],
    agent_spawn: false,
    ofp_discover: false,
  },
  skills: [],
  mcp_servers: [],
  tags: [],
  tool_allowlist: [],
  tool_blocklist: [],
  enabled: true,
});

// Keys the form fully owns within each scope. Anything else is preserved
// as `extras` and re-emitted on serialize.
const FORM_TOP_LEVEL_KEYS = new Set([
  "name",
  "version",
  "description",
  "author",
  "module",
  "enabled",
  "tags",
  "skills",
  "mcp_servers",
  "tool_allowlist",
  "tool_blocklist",
  "model",
  "resources",
  "capabilities",
]);
const FORM_MODEL_KEYS = new Set([
  "provider",
  "model",
  "system_prompt",
  "temperature",
  "max_tokens",
]);
const FORM_RESOURCE_KEYS = new Set([
  "max_llm_tokens_per_hour",
  "max_tool_calls_per_minute",
  "max_cost_per_hour_usd",
  "max_cost_per_day_usd",
]);
const FORM_CAPABILITY_KEYS = new Set([
  "network",
  "shell",
  "tools",
  "agent_spawn",
  "ofp_discover",
]);

const escapeTomlString = (value: string): string =>
  `"${value.replace(/\\/g, "\\\\").replace(/"/g, '\\"').replace(/\n/g, "\\n")}"`;

const tomlArray = (values: string[]): string =>
  `[${values.map(escapeTomlString).join(", ")}]`;

const parseInteger = (raw: string): number | null => {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  const n = Number(trimmed);
  if (!Number.isFinite(n) || !Number.isInteger(n)) return null;
  return n;
};

const parseFloatish = (raw: string): number | null => {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  const n = Number(trimmed);
  return Number.isFinite(n) ? n : null;
};

const writeStringScalar = (lines: string[], key: string, value: string): void => {
  if (!value) return;
  lines.push(`${key} = ${escapeTomlString(value)}`);
};
const writeNumberScalar = (lines: string[], key: string, value: number | null): void => {
  if (value === null) return;
  lines.push(`${key} = ${value}`);
};
const writeBoolScalar = (lines: string[], key: string, value: boolean): void => {
  lines.push(`${key} = ${value}`);
};

// Render the form (and any preserved extras) as TOML. Form-known fields
// come first in a stable, hand-friendly layout; extras are appended via
// smol-toml's stringify so user-added [thinking] / [autonomous] / inline
// keys survive untouched.
export const serializeManifestForm = (
  form: ManifestFormState,
  extras: ManifestExtras = emptyManifestExtras(),
): string => {
  const lines: string[] = [];

  writeStringScalar(lines, "name", form.name.trim());
  writeStringScalar(lines, "version", form.version.trim());
  writeStringScalar(lines, "description", form.description.trim());
  writeStringScalar(lines, "author", form.author.trim());
  writeStringScalar(lines, "module", form.module.trim());
  if (!form.enabled) writeBoolScalar(lines, "enabled", false);

  if (form.tags.length) lines.push(`tags = ${tomlArray(form.tags)}`);
  if (form.skills.length) lines.push(`skills = ${tomlArray(form.skills)}`);
  if (form.mcp_servers.length) lines.push(`mcp_servers = ${tomlArray(form.mcp_servers)}`);
  if (form.tool_allowlist.length) lines.push(`tool_allowlist = ${tomlArray(form.tool_allowlist)}`);
  if (form.tool_blocklist.length) lines.push(`tool_blocklist = ${tomlArray(form.tool_blocklist)}`);

  // Top-level extras must split: scalars/arrays go BEFORE any [table]
  // header (otherwise TOML parses them as belonging to the last table),
  // and sub-tables go AFTER the form's own tables. We do the split here
  // and stash the trailing sub-tables for emission below.
  const { inline: topInlineExtras, tables: topTableExtras } =
    splitTopLevelExtras(extras.topLevel);
  for (const line of renderExtraScalars(topInlineExtras)) {
    lines.push(line);
  }

  // [model]
  const modelBody: string[] = [];
  writeStringScalar(modelBody, "provider", form.model.provider.trim());
  writeStringScalar(modelBody, "model", form.model.model.trim());
  writeStringScalar(modelBody, "system_prompt", form.model.system_prompt);
  writeNumberScalar(modelBody, "temperature", parseFloatish(form.model.temperature));
  writeNumberScalar(modelBody, "max_tokens", parseInteger(form.model.max_tokens));
  const modelExtras = renderExtraScalars(extras.model);
  if (modelBody.length || modelExtras.length) {
    lines.push("", "[model]", ...modelBody, ...modelExtras);
  }

  // [resources]
  const resourceBody: string[] = [];
  writeNumberScalar(resourceBody, "max_llm_tokens_per_hour", parseInteger(form.resources.max_llm_tokens_per_hour));
  writeNumberScalar(resourceBody, "max_tool_calls_per_minute", parseInteger(form.resources.max_tool_calls_per_minute));
  writeNumberScalar(resourceBody, "max_cost_per_hour_usd", parseFloatish(form.resources.max_cost_per_hour_usd));
  writeNumberScalar(resourceBody, "max_cost_per_day_usd", parseFloatish(form.resources.max_cost_per_day_usd));
  const resourceExtras = renderExtraScalars(extras.resources);
  if (resourceBody.length || resourceExtras.length) {
    lines.push("", "[resources]", ...resourceBody, ...resourceExtras);
  }

  // [capabilities]
  const capabilityBody: string[] = [];
  if (form.capabilities.network.length) {
    capabilityBody.push(`network = ${tomlArray(form.capabilities.network)}`);
  }
  if (form.capabilities.shell.length) {
    capabilityBody.push(`shell = ${tomlArray(form.capabilities.shell)}`);
  }
  if (form.capabilities.tools.length) {
    capabilityBody.push(`tools = ${tomlArray(form.capabilities.tools)}`);
  }
  if (form.capabilities.agent_spawn) writeBoolScalar(capabilityBody, "agent_spawn", true);
  if (form.capabilities.ofp_discover) writeBoolScalar(capabilityBody, "ofp_discover", true);
  const capabilityExtras = renderExtraScalars(extras.capabilities);
  if (capabilityBody.length || capabilityExtras.length) {
    lines.push("", "[capabilities]", ...capabilityBody, ...capabilityExtras);
  }

  // Top-level sub-table extras (e.g. [thinking], [autonomous]) come
  // AFTER all form-known sections. smol-toml's stringify renders these
  // as `[name]\nkey = value\n…` blocks.
  const trailer = stringifyExtras(topTableExtras);
  if (trailer) {
    lines.push("", trailer.trimEnd());
  }

  return lines.join("\n") + "\n";
};

const splitTopLevelExtras = (
  extras: TomlTable,
): { inline: TomlTable; tables: TomlTable } => {
  const inline: TomlTable = {};
  const tables: TomlTable = {};
  for (const [key, value] of Object.entries(extras)) {
    if (isTomlTable(value) || isArrayOfTables(value)) {
      tables[key] = value;
    } else {
      inline[key] = value;
    }
  }
  return { inline, tables };
};

const isArrayOfTables = (v: unknown): boolean =>
  Array.isArray(v) && v.length > 0 && v.every((item) => isTomlTable(item));

// smol-toml's stringify does the heavy lifting — its output is valid
// TOML that round-trips. We only call it for the leftover bag, so the
// hand-tuned form layout above stays untouched.
const stringifyExtras = (extras: TomlTable): string => {
  if (Object.keys(extras).length === 0) return "";
  return stringify(extras);
};

// Render scalar/array extras for a sub-table inline (sub-table headers
// nested inside the section would be invalid TOML — extras inside a
// form-owned section are restricted to scalars/arrays. Nested tables
// like [model.foo] would be top-level concerns and aren't expected here).
const renderExtraScalars = (extras: TomlTable): string[] => {
  const lines: string[] = [];
  for (const [key, value] of Object.entries(extras)) {
    if (value === null || value === undefined) continue;
    // smol-toml.stringify with a single-key object yields "key = value".
    try {
      lines.push(stringify({ [key]: value }).trimEnd());
    } catch {
      // Drop unrenderable values rather than crashing the form preview.
    }
  }
  return lines;
};

// Form-validation errors. Returns an empty array when submittable.
export const validateManifestForm = (form: ManifestFormState): string[] => {
  const errors: string[] = [];
  if (!form.name.trim()) errors.push("name");
  if (!form.model.provider.trim()) errors.push("model.provider");
  if (!form.model.model.trim()) errors.push("model.model");
  return errors;
};

export interface ParseResult {
  ok: true;
  form: ManifestFormState;
  extras: ManifestExtras;
}
export interface ParseError {
  ok: false;
  message: string;
  line?: number;
  column?: number;
}

const asString = (v: unknown): string => (typeof v === "string" ? v : "");
const asNumberString = (v: unknown): string => {
  if (typeof v === "number" && Number.isFinite(v)) return String(v);
  if (typeof v === "bigint") return v.toString();
  return "";
};
const asBoolean = (v: unknown, fallback: boolean): boolean =>
  typeof v === "boolean" ? v : fallback;
const asStringArray = (v: unknown): string[] => {
  if (!Array.isArray(v)) return [];
  return v.filter((x): x is string => typeof x === "string");
};

// Parse a TOML string into form state + preserved extras. Anything the
// form can render natively becomes form state; everything else stays in
// `extras` so re-serializing produces an equivalent manifest.
//
// We only fail (ok: false) on TOML syntax errors. Type mismatches in
// individual fields are ignored — the form just leaves them blank — so
// users editing TOML can switch back to the form mid-typing without
// losing the rest of their work.
export const parseManifestToml = (toml: string): ParseResult | ParseError => {
  let parsed: TomlTable;
  try {
    parsed = parse(toml);
  } catch (e) {
    if (e instanceof TomlError) {
      return {
        ok: false,
        message: e.message,
        line: e.line,
        column: e.column,
      };
    }
    return { ok: false, message: e instanceof Error ? e.message : String(e) };
  }

  const form = emptyManifestForm();
  const extras = emptyManifestExtras();

  form.name = asString(parsed.name);
  form.version = asString(parsed.version) || form.version;
  form.description = asString(parsed.description);
  form.author = asString(parsed.author);
  form.module = asString(parsed.module) || form.module;
  form.enabled = asBoolean(parsed.enabled, true);
  form.tags = asStringArray(parsed.tags);
  form.skills = asStringArray(parsed.skills);
  form.mcp_servers = asStringArray(parsed.mcp_servers);
  form.tool_allowlist = asStringArray(parsed.tool_allowlist);
  form.tool_blocklist = asStringArray(parsed.tool_blocklist);

  const modelTable = isTomlTable(parsed.model) ? parsed.model : {};
  form.model.provider = asString(modelTable.provider);
  form.model.model = asString(modelTable.model);
  form.model.system_prompt = asString(modelTable.system_prompt);
  form.model.temperature = asNumberString(modelTable.temperature);
  form.model.max_tokens = asNumberString(modelTable.max_tokens);
  extras.model = stripKnown(modelTable, FORM_MODEL_KEYS);

  const resourceTable = isTomlTable(parsed.resources) ? parsed.resources : {};
  form.resources.max_llm_tokens_per_hour = asNumberString(resourceTable.max_llm_tokens_per_hour);
  form.resources.max_tool_calls_per_minute = asNumberString(resourceTable.max_tool_calls_per_minute);
  form.resources.max_cost_per_hour_usd = asNumberString(resourceTable.max_cost_per_hour_usd);
  form.resources.max_cost_per_day_usd = asNumberString(resourceTable.max_cost_per_day_usd);
  extras.resources = stripKnown(resourceTable, FORM_RESOURCE_KEYS);

  const capTable = isTomlTable(parsed.capabilities) ? parsed.capabilities : {};
  form.capabilities.network = asStringArray(capTable.network);
  form.capabilities.shell = asStringArray(capTable.shell);
  form.capabilities.tools = asStringArray(capTable.tools);
  form.capabilities.agent_spawn = asBoolean(capTable.agent_spawn, false);
  form.capabilities.ofp_discover = asBoolean(capTable.ofp_discover, false);
  extras.capabilities = stripKnown(capTable, FORM_CAPABILITY_KEYS);

  extras.topLevel = stripKnown(parsed, FORM_TOP_LEVEL_KEYS);

  return { ok: true, form, extras };
};

const isTomlTable = (v: unknown): v is TomlTable =>
  typeof v === "object" && v !== null && !Array.isArray(v);

const stripKnown = (table: TomlTable, knownKeys: Set<string>): TomlTable => {
  const out: TomlTable = {};
  for (const [key, value] of Object.entries(table)) {
    if (!knownKeys.has(key)) out[key] = value;
  }
  return out;
};
