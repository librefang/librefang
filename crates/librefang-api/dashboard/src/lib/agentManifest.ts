// Structured representation of the subset of AgentManifest fields exposed
// in the visual editor. The form keeps state in this shape and serializes
// to TOML on submit / for the live preview pane.
//
// Only fields most users want to set are first-class here. Everything else
// stays accessible via the raw-TOML tab once the user clicks "Switch to TOML".
//
// Numeric inputs are stored as raw strings so empty fields stay empty
// (instead of becoming 0 and silently overriding kernel defaults).

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

  // ManifestCapabilities — string-list fields are exposed as multi-input,
  // booleans as checkboxes. Anything left empty is omitted from the TOML.
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

const escapeTomlString = (value: string): string =>
  `"${value.replace(/\\/g, "\\\\").replace(/"/g, '\\"').replace(/\n/g, "\\n")}"`;

const tomlArray = (values: string[]): string =>
  `[${values.map(escapeTomlString).join(", ")}]`;

const parseInteger = (raw: string): number | null => {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  const n = Number(trimmed);
  if (!Number.isFinite(n)) return null;
  if (!Number.isInteger(n)) return null;
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

// Render the form as TOML. We deliberately produce a flat, predictable
// layout (top-level keys, then [model], [resources], [capabilities]) so
// users who graduate to the raw-TOML tab see something legible.
export const serializeManifestForm = (form: ManifestFormState): string => {
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

  const modelLines: string[] = [];
  writeStringScalar(modelLines, "provider", form.model.provider.trim());
  writeStringScalar(modelLines, "model", form.model.model.trim());
  writeStringScalar(modelLines, "system_prompt", form.model.system_prompt);
  writeNumberScalar(modelLines, "temperature", parseFloatish(form.model.temperature));
  writeNumberScalar(modelLines, "max_tokens", parseInteger(form.model.max_tokens));
  if (modelLines.length) {
    lines.push("", "[model]", ...modelLines);
  }

  const resourceLines: string[] = [];
  writeNumberScalar(resourceLines, "max_llm_tokens_per_hour", parseInteger(form.resources.max_llm_tokens_per_hour));
  writeNumberScalar(resourceLines, "max_tool_calls_per_minute", parseInteger(form.resources.max_tool_calls_per_minute));
  writeNumberScalar(resourceLines, "max_cost_per_hour_usd", parseFloatish(form.resources.max_cost_per_hour_usd));
  writeNumberScalar(resourceLines, "max_cost_per_day_usd", parseFloatish(form.resources.max_cost_per_day_usd));
  if (resourceLines.length) {
    lines.push("", "[resources]", ...resourceLines);
  }

  const capabilityLines: string[] = [];
  if (form.capabilities.network.length) {
    capabilityLines.push(`network = ${tomlArray(form.capabilities.network)}`);
  }
  if (form.capabilities.shell.length) {
    capabilityLines.push(`shell = ${tomlArray(form.capabilities.shell)}`);
  }
  if (form.capabilities.tools.length) {
    capabilityLines.push(`tools = ${tomlArray(form.capabilities.tools)}`);
  }
  if (form.capabilities.agent_spawn) writeBoolScalar(capabilityLines, "agent_spawn", true);
  if (form.capabilities.ofp_discover) writeBoolScalar(capabilityLines, "ofp_discover", true);
  if (capabilityLines.length) {
    lines.push("", "[capabilities]", ...capabilityLines);
  }

  return lines.join("\n") + "\n";
};

// Surface form-validation errors. Returns an empty array when the form
// is submittable. The kernel does the heavy validation server-side, but
// instant feedback on the obvious mistakes (no name, no model) avoids
// a round-trip.
export const validateManifestForm = (form: ManifestFormState): string[] => {
  const errors: string[] = [];
  if (!form.name.trim()) errors.push("name");
  if (!form.model.provider.trim()) errors.push("model.provider");
  if (!form.model.model.trim()) errors.push("model.model");
  return errors;
};
