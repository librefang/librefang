// Structured representation of AgentManifest for the visual editor.
//
// The form covers nearly all fields the kernel understands; anything not
// represented here (tools.* tables, metadata, profile, extra_params,
// internal flags) lives in `extras` so a TOML-tab edit that adds them
// survives a round-trip back through the form.

import { parse, stringify, TomlError, type TomlTable } from "smol-toml";

// Numeric inputs are stored as raw strings so empty fields stay empty
// (instead of becoming 0 and silently overriding kernel defaults).
export interface ManifestFormState {
  name: string;
  description: string;
  version: string;
  author: string;
  module: string;
  priority: "Low" | "Normal" | "High" | "Critical";
  session_mode: "persistent" | "new";
  web_search_augmentation: "off" | "auto" | "always";
  pinned_model: string;
  workspace: string;

  schedule:
    | { mode: "reactive" }
    | { mode: "periodic"; cron: string }
    | { mode: "proactive"; conditions: string[] }
    | { mode: "continuous"; check_interval_secs: string };

  model: {
    provider: string;
    model: string;
    system_prompt: string;
    temperature: string;
    max_tokens: string;
    api_key_env: string;
    base_url: string;
  };

  fallback_models: Array<{
    provider: string;
    model: string;
    api_key_env: string;
    base_url: string;
    // FallbackModel uses #[serde(flatten)] for provider-specific
    // params (e.g. Qwen's enable_memory). Hold them here so round-trips
    // through the form don't strip provider customisations.
    extras: TomlTable;
  }>;

  resources: {
    max_llm_tokens_per_hour: string;
    max_tool_calls_per_minute: string;
    max_cost_per_hour_usd: string;
    max_cost_per_day_usd: string;
    max_cost_per_month_usd: string;
    max_memory_bytes: string;
    max_cpu_time_ms: string;
    max_network_bytes_per_hour: string;
  };

  capabilities: {
    network: string[];
    shell: string[];
    tools: string[];
    memory_read: string[];
    memory_write: string[];
    agent_message: string[];
    ofp_connect: string[];
    agent_spawn: boolean;
    ofp_discover: boolean;
  };

  thinking: {
    enabled: boolean;
    budget_tokens: string;
    stream_thinking: boolean;
  };

  autonomous: {
    enabled: boolean;
    max_iterations: string;
    max_restarts: string;
    heartbeat_interval_secs: string;
    heartbeat_timeout_secs: string;
    heartbeat_keep_recent: string;
    heartbeat_channel: string;
    quiet_hours: string;
  };

  routing: {
    enabled: boolean;
    simple_model: string;
    medium_model: string;
    complex_model: string;
    simple_threshold: string;
    complex_threshold: string;
  };

  context_injection: Array<{
    name: string;
    content: string;
    position: "system" | "before_user" | "after_reset";
    condition: string;
  }>;

  response_format:
    | { mode: "text" }
    | { mode: "json" }
    | { mode: "json_schema"; name: string; schema: string; strict: boolean };

  // Only the shorthand string variants are exposed here. Full ExecPolicy
  // tables (`[exec_policy]` with mode/safe_bins/timeout_secs/…) stay in
  // extras so they're preserved without complicating the form.
  exec_policy_shorthand: "" | "allow" | "deny" | "full" | "allowlist";

  skills: string[];
  mcp_servers: string[];
  tags: string[];
  tool_allowlist: string[];
  tool_blocklist: string[];
  allowed_plugins: string[];

  enabled: boolean;
  skills_disabled: boolean;
  tools_disabled: boolean;
  inherit_parent_context: boolean;
  generate_identity_files: boolean;
}

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
  priority: "Normal",
  session_mode: "persistent",
  web_search_augmentation: "auto",
  pinned_model: "",
  workspace: "",
  schedule: { mode: "reactive" },
  model: {
    provider: "",
    model: "",
    system_prompt: "",
    temperature: "",
    max_tokens: "",
    api_key_env: "",
    base_url: "",
  },
  fallback_models: [],
  resources: {
    max_llm_tokens_per_hour: "",
    max_tool_calls_per_minute: "",
    max_cost_per_hour_usd: "",
    max_cost_per_day_usd: "",
    max_cost_per_month_usd: "",
    max_memory_bytes: "",
    max_cpu_time_ms: "",
    max_network_bytes_per_hour: "",
  },
  capabilities: {
    network: [],
    shell: [],
    tools: [],
    memory_read: [],
    memory_write: [],
    agent_message: [],
    ofp_connect: [],
    agent_spawn: false,
    ofp_discover: false,
  },
  thinking: { enabled: false, budget_tokens: "", stream_thinking: false },
  autonomous: {
    enabled: false,
    max_iterations: "",
    max_restarts: "",
    heartbeat_interval_secs: "",
    heartbeat_timeout_secs: "",
    heartbeat_keep_recent: "",
    heartbeat_channel: "",
    quiet_hours: "",
  },
  routing: {
    enabled: false,
    simple_model: "",
    medium_model: "",
    complex_model: "",
    simple_threshold: "",
    complex_threshold: "",
  },
  context_injection: [],
  response_format: { mode: "text" },
  exec_policy_shorthand: "",
  skills: [],
  mcp_servers: [],
  tags: [],
  tool_allowlist: [],
  tool_blocklist: [],
  allowed_plugins: [],
  enabled: true,
  skills_disabled: false,
  tools_disabled: false,
  inherit_parent_context: true,
  generate_identity_files: true,
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
  "priority",
  "session_mode",
  "web_search_augmentation",
  "pinned_model",
  "workspace",
  "skills_disabled",
  "tools_disabled",
  "inherit_parent_context",
  "generate_identity_files",
  "tags",
  "skills",
  "mcp_servers",
  "tool_allowlist",
  "tool_blocklist",
  "allowed_plugins",
  "schedule",
  "model",
  "resources",
  "capabilities",
  "fallback_models",
  "thinking",
  "autonomous",
  "routing",
  "context_injection",
  "response_format",
  "exec_policy",
]);
const FORM_MODEL_KEYS = new Set([
  "provider",
  "model",
  "system_prompt",
  "temperature",
  "max_tokens",
  "api_key_env",
  "base_url",
]);
const FORM_RESOURCE_KEYS = new Set([
  "max_llm_tokens_per_hour",
  "max_tool_calls_per_minute",
  "max_cost_per_hour_usd",
  "max_cost_per_day_usd",
  "max_cost_per_month_usd",
  "max_memory_bytes",
  "max_cpu_time_ms",
  "max_network_bytes_per_hour",
]);
const FALLBACK_MODEL_KEYS = new Set([
  "provider",
  "model",
  "api_key_env",
  "base_url",
]);
const FORM_CAPABILITY_KEYS = new Set([
  "network",
  "shell",
  "tools",
  "memory_read",
  "memory_write",
  "agent_message",
  "ofp_connect",
  "agent_spawn",
  "ofp_discover",
]);

const SCHEDULE_DEFAULT_INTERVAL = "300";
const PRIORITIES = ["Low", "Normal", "High", "Critical"] as const;
const SESSION_MODES = ["persistent", "new"] as const;
const WEB_SEARCH_MODES = ["off", "auto", "always"] as const;
const INJECTION_POSITIONS = ["system", "before_user", "after_reset"] as const;
const EXEC_SHORTHANDS = ["allow", "deny", "full", "allowlist"] as const;

const escapeTomlString = (value: string): string =>
  `"${value.replace(/\\/g, "\\\\").replace(/"/g, '\\"').replace(/\n/g, "\\n")}"`;

const tomlArray = (values: string[]): string =>
  `[${values.map(escapeTomlString).join(", ")}]`;

// All integer manifest fields the form touches map to unsigned Rust
// types (u32/u64). Reject negatives and anything beyond JS's safe-integer
// range — the latter would silently lose precision before reaching the
// kernel, the former is rejected server-side anyway. For form UX, "garbage
// in → field omitted" is friendlier than a server-side error after submit.
const parseInteger = (raw: string): number | null => {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  const n = Number(trimmed);
  if (!Number.isFinite(n) || !Number.isInteger(n)) return null;
  if (n < 0 || n > Number.MAX_SAFE_INTEGER) return null;
  return n;
};

const parseFloatish = (raw: string): number | null => {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  const n = Number(trimmed);
  if (!Number.isFinite(n)) return null;
  if (n < 0) return null; // all our float fields are cost/quota — never negative
  return n;
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

// Render the form (and any preserved extras) as TOML.
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
  if (form.priority !== "Normal") writeStringScalar(lines, "priority", form.priority);
  if (form.session_mode !== "persistent") {
    writeStringScalar(lines, "session_mode", form.session_mode);
  }
  if (form.web_search_augmentation !== "auto") {
    writeStringScalar(lines, "web_search_augmentation", form.web_search_augmentation);
  }
  writeStringScalar(lines, "pinned_model", form.pinned_model.trim());
  writeStringScalar(lines, "workspace", form.workspace.trim());
  if (form.skills_disabled) writeBoolScalar(lines, "skills_disabled", true);
  if (form.tools_disabled) writeBoolScalar(lines, "tools_disabled", true);
  if (!form.inherit_parent_context) {
    writeBoolScalar(lines, "inherit_parent_context", false);
  }
  if (!form.generate_identity_files) {
    writeBoolScalar(lines, "generate_identity_files", false);
  }

  if (form.tags.length) lines.push(`tags = ${tomlArray(form.tags)}`);
  if (form.skills.length) lines.push(`skills = ${tomlArray(form.skills)}`);
  if (form.mcp_servers.length) lines.push(`mcp_servers = ${tomlArray(form.mcp_servers)}`);
  if (form.tool_allowlist.length) lines.push(`tool_allowlist = ${tomlArray(form.tool_allowlist)}`);
  if (form.tool_blocklist.length) lines.push(`tool_blocklist = ${tomlArray(form.tool_blocklist)}`);
  if (form.allowed_plugins.length) {
    lines.push(`allowed_plugins = ${tomlArray(form.allowed_plugins)}`);
  }

  // Schedule — Reactive is the default and emits nothing; tagged variants
  // serialize as the externally-tagged TOML form `schedule = { variant = { … } }`.
  const scheduleLine = renderSchedule(form.schedule);
  if (scheduleLine) lines.push(scheduleLine);

  if (form.exec_policy_shorthand) {
    writeStringScalar(lines, "exec_policy", form.exec_policy_shorthand);
  }

  // response_format — only emit if non-default.
  const responseFormatLine = renderResponseFormat(form.response_format);
  if (responseFormatLine) lines.push(responseFormatLine);

  // Top-level extras — split scalars (BEFORE table headers) and tables (AFTER).
  // Drop any key the form is about to emit itself, otherwise we'd produce
  // a duplicate key that smol-toml (and the kernel) rejects:
  //   - exec_policy: form emits the shorthand, extras may carry the full table
  //   - response_format: form emits text/json/json_schema, extras may carry
  //     an unmappable `type = "future_format"` table that survived parse
  let filteredTopExtras = extras.topLevel;
  if (form.exec_policy_shorthand) {
    filteredTopExtras = omitKey(filteredTopExtras, "exec_policy");
  }
  if (form.response_format.mode !== "text") {
    filteredTopExtras = omitKey(filteredTopExtras, "response_format");
  }
  const { inline: topInlineExtras, tables: topTableExtras } =
    splitTopLevelExtras(filteredTopExtras);
  for (const line of renderExtraScalars(topInlineExtras)) lines.push(line);

  // Section-extras that contain nested tables must NOT be inlined inside
  // the [section] block — a stray `[name]` header would re-anchor TOML
  // scoping for everything that follows. Defer them and emit later with
  // their full dotted key path, e.g. `[model.exotic_subtable]`.
  const deferredSectionExtras: Record<string, TomlTable | TomlTable[]> = {};
  const safeModelExtras = pluckSafeExtras(extras.model, deferredSectionExtras, "model");
  const safeResourceExtras = pluckSafeExtras(extras.resources, deferredSectionExtras, "resources");
  const safeCapabilityExtras = pluckSafeExtras(
    extras.capabilities,
    deferredSectionExtras,
    "capabilities",
  );

  // [model]
  const modelBody: string[] = [];
  writeStringScalar(modelBody, "provider", form.model.provider.trim());
  writeStringScalar(modelBody, "model", form.model.model.trim());
  writeStringScalar(modelBody, "system_prompt", form.model.system_prompt);
  writeNumberScalar(modelBody, "temperature", parseFloatish(form.model.temperature));
  writeNumberScalar(modelBody, "max_tokens", parseInteger(form.model.max_tokens));
  writeStringScalar(modelBody, "api_key_env", form.model.api_key_env.trim());
  writeStringScalar(modelBody, "base_url", form.model.base_url.trim());
  const modelExtras = renderExtraScalars(safeModelExtras);
  if (modelBody.length || modelExtras.length) {
    lines.push("", "[model]", ...modelBody, ...modelExtras);
  }

  // [resources]
  const resourceBody: string[] = [];
  writeNumberScalar(resourceBody, "max_llm_tokens_per_hour", parseInteger(form.resources.max_llm_tokens_per_hour));
  writeNumberScalar(resourceBody, "max_tool_calls_per_minute", parseInteger(form.resources.max_tool_calls_per_minute));
  writeNumberScalar(resourceBody, "max_cost_per_hour_usd", parseFloatish(form.resources.max_cost_per_hour_usd));
  writeNumberScalar(resourceBody, "max_cost_per_day_usd", parseFloatish(form.resources.max_cost_per_day_usd));
  writeNumberScalar(resourceBody, "max_cost_per_month_usd", parseFloatish(form.resources.max_cost_per_month_usd));
  writeNumberScalar(resourceBody, "max_memory_bytes", parseInteger(form.resources.max_memory_bytes));
  writeNumberScalar(resourceBody, "max_cpu_time_ms", parseInteger(form.resources.max_cpu_time_ms));
  writeNumberScalar(resourceBody, "max_network_bytes_per_hour", parseInteger(form.resources.max_network_bytes_per_hour));
  const resourceExtras = renderExtraScalars(safeResourceExtras);
  if (resourceBody.length || resourceExtras.length) {
    lines.push("", "[resources]", ...resourceBody, ...resourceExtras);
  }

  // [capabilities]
  const capabilityBody: string[] = [];
  if (form.capabilities.network.length) capabilityBody.push(`network = ${tomlArray(form.capabilities.network)}`);
  if (form.capabilities.shell.length) capabilityBody.push(`shell = ${tomlArray(form.capabilities.shell)}`);
  if (form.capabilities.tools.length) capabilityBody.push(`tools = ${tomlArray(form.capabilities.tools)}`);
  if (form.capabilities.memory_read.length) capabilityBody.push(`memory_read = ${tomlArray(form.capabilities.memory_read)}`);
  if (form.capabilities.memory_write.length) capabilityBody.push(`memory_write = ${tomlArray(form.capabilities.memory_write)}`);
  if (form.capabilities.agent_message.length) capabilityBody.push(`agent_message = ${tomlArray(form.capabilities.agent_message)}`);
  if (form.capabilities.ofp_connect.length) capabilityBody.push(`ofp_connect = ${tomlArray(form.capabilities.ofp_connect)}`);
  if (form.capabilities.agent_spawn) writeBoolScalar(capabilityBody, "agent_spawn", true);
  if (form.capabilities.ofp_discover) writeBoolScalar(capabilityBody, "ofp_discover", true);
  const capabilityExtras = renderExtraScalars(safeCapabilityExtras);
  if (capabilityBody.length || capabilityExtras.length) {
    lines.push("", "[capabilities]", ...capabilityBody, ...capabilityExtras);
  }

  // [thinking]
  if (form.thinking.enabled) {
    const body: string[] = [];
    writeNumberScalar(body, "budget_tokens", parseInteger(form.thinking.budget_tokens));
    writeBoolScalar(body, "stream_thinking", form.thinking.stream_thinking);
    lines.push("", "[thinking]", ...body);
  }

  // [autonomous]
  if (form.autonomous.enabled) {
    const body: string[] = [];
    writeNumberScalar(body, "max_iterations", parseInteger(form.autonomous.max_iterations));
    writeNumberScalar(body, "max_restarts", parseInteger(form.autonomous.max_restarts));
    writeNumberScalar(body, "heartbeat_interval_secs", parseInteger(form.autonomous.heartbeat_interval_secs));
    writeNumberScalar(body, "heartbeat_timeout_secs", parseInteger(form.autonomous.heartbeat_timeout_secs));
    writeNumberScalar(body, "heartbeat_keep_recent", parseInteger(form.autonomous.heartbeat_keep_recent));
    writeStringScalar(body, "heartbeat_channel", form.autonomous.heartbeat_channel.trim());
    writeStringScalar(body, "quiet_hours", form.autonomous.quiet_hours.trim());
    lines.push("", "[autonomous]", ...body);
  }

  // [routing]
  if (form.routing.enabled) {
    const body: string[] = [];
    writeStringScalar(body, "simple_model", form.routing.simple_model.trim());
    writeStringScalar(body, "medium_model", form.routing.medium_model.trim());
    writeStringScalar(body, "complex_model", form.routing.complex_model.trim());
    writeNumberScalar(body, "simple_threshold", parseInteger(form.routing.simple_threshold));
    writeNumberScalar(body, "complex_threshold", parseInteger(form.routing.complex_threshold));
    lines.push("", "[routing]", ...body);
  }

  // [[fallback_models]]
  for (const fb of form.fallback_models) {
    const body: string[] = [];
    writeStringScalar(body, "provider", fb.provider.trim());
    writeStringScalar(body, "model", fb.model.trim());
    writeStringScalar(body, "api_key_env", fb.api_key_env.trim());
    writeStringScalar(body, "base_url", fb.base_url.trim());
    // Re-emit provider-specific extras (e.g. Qwen's enable_memory). Same
    // newline-defence as the section-extras path: refuse multi-line
    // output that would re-anchor scoping inside this `[[fallback_models]]`
    // table item.
    body.push(...renderExtraScalars(fb.extras ?? {}));
    if (body.length) lines.push("", "[[fallback_models]]", ...body);
  }

  // [[context_injection]]
  for (const ci of form.context_injection) {
    const body: string[] = [];
    writeStringScalar(body, "name", ci.name.trim());
    writeStringScalar(body, "content", ci.content);
    if (ci.position !== "system") writeStringScalar(body, "position", ci.position);
    writeStringScalar(body, "condition", ci.condition.trim());
    if (body.length) lines.push("", "[[context_injection]]", ...body);
  }

  // Deferred section sub-tables (e.g. [model.exotic_subtable]) — must be
  // emitted at top-level scope, not inside the [section] block. Build a
  // nested object so smol-toml uses dotted-key headers like
  // `[model.exotic_subtable]` rather than quoting the dotted name.
  const nestedDeferred: TomlTable = {};
  for (const [dottedKey, value] of Object.entries(deferredSectionExtras)) {
    const [section, subKey] = dottedKey.split(".", 2);
    if (!subKey) continue;
    if (!isTomlTable(nestedDeferred[section])) {
      nestedDeferred[section] = {};
    }
    (nestedDeferred[section] as TomlTable)[subKey] = value;
  }
  if (Object.keys(nestedDeferred).length > 0) {
    try {
      const block = stringify(nestedDeferred).trimEnd();
      if (block) lines.push("", block);
    } catch {
      // Skip unrenderable values rather than corrupting the document.
    }
  }

  // Top-level extras' sub-tables come last (TOML scoping requires it).
  const trailer = stringifyExtras(topTableExtras);
  if (trailer) lines.push("", trailer.trimEnd());

  return lines.join("\n") + "\n";
};

// Walk a section's extras: scalar/array values stay (safe to inline);
// table-typed values (objects, arrays-of-tables) are moved to `deferred`
// keyed by the full dotted path so they get emitted as proper top-level
// sub-tables. We defer aggressively because smol-toml's stringify
// renders any object value as a multi-line `[name]` block, even when the
// inner content would have fit in inline-table syntax.
const pluckSafeExtras = (
  table: TomlTable,
  deferred: Record<string, TomlTable | TomlTable[]>,
  sectionName: string,
): TomlTable => {
  const safe: TomlTable = {};
  for (const [key, value] of Object.entries(table)) {
    if (isTomlTable(value) || isArrayOfTables(value)) {
      deferred[`${sectionName}.${key}`] = value as TomlTable | TomlTable[];
    } else {
      safe[key] = value;
    }
  }
  return safe;
};

const renderSchedule = (s: ManifestFormState["schedule"]): string => {
  switch (s.mode) {
    case "reactive":
      return ""; // default
    case "periodic":
      return `schedule = { periodic = { cron = ${escapeTomlString(s.cron)} } }`;
    case "proactive":
      return `schedule = { proactive = { conditions = ${tomlArray(s.conditions)} } }`;
    case "continuous": {
      const interval = parseInteger(s.check_interval_secs) ?? Number(SCHEDULE_DEFAULT_INTERVAL);
      return `schedule = { continuous = { check_interval_secs = ${interval} } }`;
    }
  }
};

const renderResponseFormat = (rf: ManifestFormState["response_format"]): string => {
  if (rf.mode === "text") return "";
  if (rf.mode === "json") return 'response_format = { type = "json" }';
  // json_schema — schemas can be deeply nested, which makes inline-table
  // syntax brittle. Build the value once via JSON, then convert to TOML
  // using a small recursive emitter that always produces inline syntax.
  let schemaValue: unknown = {};
  try {
    schemaValue = JSON.parse(rf.schema || "{}");
  } catch {
    // Bad JSON in the schema field — fall back to {} rather than emit
    // garbage. The user sees the parse error live in the form anyway.
  }
  const parts: string[] = [`type = "json_schema"`, `name = ${escapeTomlString(rf.name || "response")}`];
  parts.push(`schema = ${jsonValueToInlineToml(schemaValue)}`);
  if (rf.strict) parts.push("strict = true");
  return `response_format = { ${parts.join(", ")} }`;
};

// Recursively render a JSON value as TOML inline syntax (no [headers],
// no newlines). Suitable for embedding inside an inline-table key.
const jsonValueToInlineToml = (value: unknown): string => {
  if (value === null || value === undefined) return '""'; // TOML has no null
  if (typeof value === "string") return escapeTomlString(value);
  if (typeof value === "boolean") return String(value);
  if (typeof value === "number") {
    return Number.isFinite(value) ? String(value) : "0";
  }
  if (Array.isArray(value)) {
    return `[${value.map(jsonValueToInlineToml).join(", ")}]`;
  }
  if (typeof value === "object") {
    const entries = Object.entries(value as Record<string, unknown>).map(
      ([k, v]) => `${tomlBareKeyOrQuoted(k)} = ${jsonValueToInlineToml(v)}`,
    );
    return `{ ${entries.join(", ")} }`;
  }
  return '""';
};

// TOML bare keys allow only [A-Za-z0-9_-]. Anything else needs quoting.
const tomlBareKeyOrQuoted = (key: string): string =>
  /^[A-Za-z0-9_-]+$/.test(key) ? key : escapeTomlString(key);

const stringifyExtras = (extras: TomlTable): string => {
  if (Object.keys(extras).length === 0) return "";
  return stringify(extras);
};

const renderExtraScalars = (extras: TomlTable): string[] => {
  const lines: string[] = [];
  for (const [key, value] of Object.entries(extras)) {
    if (value === null || value === undefined) continue;
    try {
      const rendered = stringify({ [key]: value }).trimEnd();
      // Defensive: refuse multi-line output. Inserted inside a [section]
      // block, an embedded `[name]` header would re-anchor TOML scoping
      // for everything below. Callers should have routed these through
      // pluckSafeExtras already, but belt-and-braces.
      if (rendered.includes("\n")) continue;
      lines.push(rendered);
    } catch {
      // Drop unrenderable values rather than crashing the form preview.
    }
  }
  return lines;
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

const omitKey = (table: TomlTable, key: string): TomlTable => {
  const { [key]: _omit, ...rest } = table;
  return rest;
};

const stringifyOrEmpty = (value: unknown): string => {
  if (value === undefined || value === null) return "{}";
  try {
    const out = JSON.stringify(value, null, 2);
    return typeof out === "string" ? out : "{}";
  } catch {
    return "{}";
  }
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
const asEnum = <T extends readonly string[]>(
  v: unknown,
  allowed: T,
  fallback: T[number],
): T[number] => {
  if (typeof v === "string" && (allowed as readonly string[]).includes(v)) {
    return v as T[number];
  }
  return fallback;
};

export const parseManifestToml = (toml: string): ParseResult | ParseError => {
  let parsed: TomlTable;
  try {
    parsed = parse(toml);
  } catch (e) {
    if (e instanceof TomlError) {
      return { ok: false, message: e.message, line: e.line, column: e.column };
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
  form.priority = asEnum(parsed.priority, PRIORITIES, "Normal");
  form.session_mode = asEnum(parsed.session_mode, SESSION_MODES, "persistent");
  form.web_search_augmentation = asEnum(
    parsed.web_search_augmentation,
    WEB_SEARCH_MODES,
    "auto",
  );
  form.pinned_model = asString(parsed.pinned_model);
  form.workspace = asString(parsed.workspace);
  form.skills_disabled = asBoolean(parsed.skills_disabled, false);
  form.tools_disabled = asBoolean(parsed.tools_disabled, false);
  form.inherit_parent_context = asBoolean(parsed.inherit_parent_context, true);
  form.generate_identity_files = asBoolean(parsed.generate_identity_files, true);
  form.tags = asStringArray(parsed.tags);
  form.skills = asStringArray(parsed.skills);
  form.mcp_servers = asStringArray(parsed.mcp_servers);
  form.tool_allowlist = asStringArray(parsed.tool_allowlist);
  form.tool_blocklist = asStringArray(parsed.tool_blocklist);
  form.allowed_plugins = asStringArray(parsed.allowed_plugins);
  form.schedule = parseScheduleField(parsed.schedule);
  form.exec_policy_shorthand = parseExecPolicyShorthand(parsed.exec_policy);
  form.response_format = parseResponseFormatField(parsed.response_format);

  // Extras for top-level: capture exec_policy only when it's a table
  // (the form owns the shorthand string form).
  const topExtras: TomlTable = {};
  for (const [k, v] of Object.entries(parsed)) {
    if (FORM_TOP_LEVEL_KEYS.has(k)) continue;
    topExtras[k] = v;
  }
  // exec_policy as a full table (not a shorthand string) is preserved in extras.
  if (isTomlTable(parsed.exec_policy)) topExtras.exec_policy = parsed.exec_policy;
  // response_format that we couldn't fully map to the form's enum (e.g.
  // unknown `type`) goes back into extras to avoid silent loss.
  if (
    isTomlTable(parsed.response_format) &&
    form.response_format.mode === "text" &&
    asString((parsed.response_format as TomlTable).type) !== "text"
  ) {
    topExtras.response_format = parsed.response_format;
  }
  extras.topLevel = topExtras;

  // [model]
  const modelTable = isTomlTable(parsed.model) ? parsed.model : {};
  form.model.provider = asString(modelTable.provider);
  form.model.model = asString(modelTable.model);
  form.model.system_prompt = asString(modelTable.system_prompt);
  form.model.temperature = asNumberString(modelTable.temperature);
  form.model.max_tokens = asNumberString(modelTable.max_tokens);
  form.model.api_key_env = asString(modelTable.api_key_env);
  form.model.base_url = asString(modelTable.base_url);
  extras.model = stripKnown(modelTable, FORM_MODEL_KEYS);

  // [[fallback_models]] — capture provider-specific flatten extras too,
  // so e.g. Qwen's enable_memory survives a TOML→Form→TOML round-trip.
  if (Array.isArray(parsed.fallback_models)) {
    form.fallback_models = parsed.fallback_models.filter(isTomlTable).map((fb) => ({
      provider: asString(fb.provider),
      model: asString(fb.model),
      api_key_env: asString(fb.api_key_env),
      base_url: asString(fb.base_url),
      extras: stripKnown(fb, FALLBACK_MODEL_KEYS),
    }));
  }

  // [resources]
  const resourceTable = isTomlTable(parsed.resources) ? parsed.resources : {};
  form.resources.max_llm_tokens_per_hour = asNumberString(resourceTable.max_llm_tokens_per_hour);
  form.resources.max_tool_calls_per_minute = asNumberString(resourceTable.max_tool_calls_per_minute);
  form.resources.max_cost_per_hour_usd = asNumberString(resourceTable.max_cost_per_hour_usd);
  form.resources.max_cost_per_day_usd = asNumberString(resourceTable.max_cost_per_day_usd);
  form.resources.max_cost_per_month_usd = asNumberString(resourceTable.max_cost_per_month_usd);
  form.resources.max_memory_bytes = asNumberString(resourceTable.max_memory_bytes);
  form.resources.max_cpu_time_ms = asNumberString(resourceTable.max_cpu_time_ms);
  form.resources.max_network_bytes_per_hour = asNumberString(resourceTable.max_network_bytes_per_hour);
  extras.resources = stripKnown(resourceTable, FORM_RESOURCE_KEYS);

  // [capabilities]
  const capTable = isTomlTable(parsed.capabilities) ? parsed.capabilities : {};
  form.capabilities.network = asStringArray(capTable.network);
  form.capabilities.shell = asStringArray(capTable.shell);
  form.capabilities.tools = asStringArray(capTable.tools);
  form.capabilities.memory_read = asStringArray(capTable.memory_read);
  form.capabilities.memory_write = asStringArray(capTable.memory_write);
  form.capabilities.agent_message = asStringArray(capTable.agent_message);
  form.capabilities.ofp_connect = asStringArray(capTable.ofp_connect);
  form.capabilities.agent_spawn = asBoolean(capTable.agent_spawn, false);
  form.capabilities.ofp_discover = asBoolean(capTable.ofp_discover, false);
  extras.capabilities = stripKnown(capTable, FORM_CAPABILITY_KEYS);

  // [thinking]
  if (isTomlTable(parsed.thinking)) {
    form.thinking.enabled = true;
    form.thinking.budget_tokens = asNumberString(parsed.thinking.budget_tokens);
    form.thinking.stream_thinking = asBoolean(parsed.thinking.stream_thinking, false);
  }

  // [autonomous]
  if (isTomlTable(parsed.autonomous)) {
    const a = parsed.autonomous;
    form.autonomous.enabled = true;
    form.autonomous.max_iterations = asNumberString(a.max_iterations);
    form.autonomous.max_restarts = asNumberString(a.max_restarts);
    form.autonomous.heartbeat_interval_secs = asNumberString(a.heartbeat_interval_secs);
    form.autonomous.heartbeat_timeout_secs = asNumberString(a.heartbeat_timeout_secs);
    form.autonomous.heartbeat_keep_recent = asNumberString(a.heartbeat_keep_recent);
    form.autonomous.heartbeat_channel = asString(a.heartbeat_channel);
    form.autonomous.quiet_hours = asString(a.quiet_hours);
  }

  // [routing]
  if (isTomlTable(parsed.routing)) {
    const r = parsed.routing;
    form.routing.enabled = true;
    form.routing.simple_model = asString(r.simple_model);
    form.routing.medium_model = asString(r.medium_model);
    form.routing.complex_model = asString(r.complex_model);
    form.routing.simple_threshold = asNumberString(r.simple_threshold);
    form.routing.complex_threshold = asNumberString(r.complex_threshold);
  }

  // [[context_injection]]
  if (Array.isArray(parsed.context_injection)) {
    form.context_injection = parsed.context_injection
      .filter(isTomlTable)
      .map((ci) => ({
        name: asString(ci.name),
        content: asString(ci.content),
        position: asEnum(ci.position, INJECTION_POSITIONS, "system"),
        condition: asString(ci.condition),
      }));
  }

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

const parseScheduleField = (raw: unknown): ManifestFormState["schedule"] => {
  if (typeof raw === "string") {
    if (raw === "reactive") return { mode: "reactive" };
    return { mode: "reactive" };
  }
  if (!isTomlTable(raw)) return { mode: "reactive" };
  if (isTomlTable(raw.periodic)) {
    return { mode: "periodic", cron: asString(raw.periodic.cron) };
  }
  if (isTomlTable(raw.proactive)) {
    return { mode: "proactive", conditions: asStringArray(raw.proactive.conditions) };
  }
  if (isTomlTable(raw.continuous)) {
    return {
      mode: "continuous",
      check_interval_secs:
        asNumberString(raw.continuous.check_interval_secs) || SCHEDULE_DEFAULT_INTERVAL,
    };
  }
  return { mode: "reactive" };
};

// exec_policy_lenient on the kernel side (serde_compat.rs) accepts
// aliases for each canonical mode. The form's dropdown only knows the
// canonical names, so normalize aliases at the parse boundary —
// otherwise the alias spelling rounds-trips to an empty shorthand and
// the user's intent (deny / allowlist / full) is silently lost.
const EXEC_POLICY_ALIASES: Record<string, ManifestFormState["exec_policy_shorthand"]> = {
  none: "deny",
  disabled: "deny",
  restricted: "allowlist",
  all: "full",
  unrestricted: "full",
};

const parseExecPolicyShorthand = (
  raw: unknown,
): ManifestFormState["exec_policy_shorthand"] => {
  if (typeof raw !== "string") return "";
  if ((EXEC_SHORTHANDS as readonly string[]).includes(raw)) {
    return raw as ManifestFormState["exec_policy_shorthand"];
  }
  return EXEC_POLICY_ALIASES[raw] ?? "";
};

const parseResponseFormatField = (raw: unknown): ManifestFormState["response_format"] => {
  if (!isTomlTable(raw)) return { mode: "text" };
  const type = asString(raw.type);
  if (type === "json") return { mode: "json" };
  if (type === "json_schema") {
    return {
      mode: "json_schema",
      name: asString(raw.name),
      // JSON.stringify(undefined) returns undefined (not a string!), which
      // would break the `schema: string` type and trigger React's
      // uncontrolled→controlled warning when fed to <textarea value={…}>.
      // Default to `{}` whenever the source schema is missing or
      // unrenderable.
      schema: stringifyOrEmpty(raw.schema),
      strict: asBoolean(raw.strict, false),
    };
  }
  return { mode: "text" };
};
