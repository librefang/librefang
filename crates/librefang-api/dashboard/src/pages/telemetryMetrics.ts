export interface HttpMetric {
  method: string;
  path: string;
  status: string;
  count: number;
}

export interface AgentTokenMetric {
  agent: string;
  provider: string;
  model: string;
  tokens: number;
  inputTokens: number;
  outputTokens: number;
  toolCalls: number;
  llmCalls: number;
}

interface SystemMetrics {
  uptime: number;
  agentsActive: number;
  agentsTotal: number;
  activeSessions: number;
  costToday: number;
  panics: number;
  restarts: number;
  version: string;
}

export interface ParsedMetrics {
  requests: HttpMetric[];
  agents: AgentTokenMetric[];
  system: SystemMetrics;
}

interface PrometheusSample {
  name: string;
  labels: Map<string, string>;
  value: string;
}

function parseLabels(source: string): Map<string, string> | null {
  const labels = new Map<string, string>();
  let index = 0;

  while (index < source.length) {
    while (source[index] === " " || source[index] === "\t") index += 1;
    const keyMatch = source.slice(index).match(/^([a-zA-Z_][a-zA-Z0-9_]*)/);
    if (!keyMatch) return null;
    const key = keyMatch[1];
    index += key.length;

    while (source[index] === " " || source[index] === "\t") index += 1;
    if (source[index] !== "=") return null;
    index += 1;
    while (source[index] === " " || source[index] === "\t") index += 1;
    if (source[index] !== '"') return null;
    index += 1;

    let value = "";
    let closed = false;
    while (index < source.length) {
      const char = source[index];
      index += 1;
      if (char === '"') {
        closed = true;
        break;
      }
      if (char === "\\") {
        if (index >= source.length) return null;
        const escaped = source[index];
        index += 1;
        if (escaped === "n") value += "\n";
        else if (escaped === "\\" || escaped === '"') value += escaped;
        else return null;
      } else {
        value += char;
      }
    }
    if (!closed) return null;
    labels.set(key, value);

    while (source[index] === " " || source[index] === "\t") index += 1;
    if (index === source.length) break;
    if (source[index] !== ",") return null;
    index += 1;
  }

  return labels;
}

function parseSample(line: string): PrometheusSample | null {
  const match = line.match(
    /^([a-zA-Z_:][a-zA-Z0-9_:]*)(?:\{(.*)\})?\s+(\S+)(?:\s+\d+)?\s*$/,
  );
  if (!match) return null;
  const labels = match[2] === undefined ? new Map<string, string>() : parseLabels(match[2]);
  if (!labels) return null;
  return { name: match[1], labels, value: match[3] };
}

function parseNonNegativeInteger(value: string): number | null {
  if (!/^\d+$/.test(value)) return null;
  const parsed = Number(value);
  return Number.isSafeInteger(parsed) ? parsed : null;
}

function ensureAgentEntry(
  map: Map<string, AgentTokenMetric>,
  agent: string,
): AgentTokenMetric {
  let entry = map.get(agent);
  if (!entry) {
    entry = {
      agent,
      provider: "",
      model: "",
      tokens: 0,
      inputTokens: 0,
      outputTokens: 0,
      toolCalls: 0,
      llmCalls: 0,
    };
    map.set(agent, entry);
  }
  return entry;
}

function hasUsage(metric: AgentTokenMetric): boolean {
  return (
    metric.tokens > 0 ||
    metric.inputTokens > 0 ||
    metric.outputTokens > 0 ||
    metric.toolCalls > 0 ||
    metric.llmCalls > 0
  );
}

export function parseMetrics(text: string): ParsedMetrics {
  const requests: HttpMetric[] = [];
  const agentMap = new Map<string, AgentTokenMetric>();
  const gaugeMap = new Map<string, number>();
  let version = "";

  for (const line of text.split("\n")) {
    if (line === "" || line.startsWith("#")) continue;
    const sample = parseSample(line);
    if (!sample) continue;

    if (sample.labels.size === 0) {
      const value = Number(sample.value);
      if (Number.isFinite(value)) gaugeMap.set(sample.name, value);
    }

    if (sample.name === "librefang_http_requests_total") {
      const method = sample.labels.get("method");
      const path = sample.labels.get("path");
      const status = sample.labels.get("status");
      const count = parseNonNegativeInteger(sample.value);
      if (method && path && status && count !== null) {
        requests.push({ method, path, status, count });
      }
      continue;
    }

    const agent = sample.labels.get("agent");
    if (agent) {
      const value = parseNonNegativeInteger(sample.value);
      if (value === null) continue;
      const entry = ensureAgentEntry(agentMap, agent);
      entry.provider = sample.labels.get("provider") ?? entry.provider;
      entry.model = sample.labels.get("model") ?? entry.model;
      switch (sample.name) {
        case "librefang_tokens":
          entry.tokens = value;
          break;
        case "librefang_tokens_input":
          entry.inputTokens = value;
          break;
        case "librefang_tokens_output":
          entry.outputTokens = value;
          break;
        case "librefang_tool_calls":
          entry.toolCalls = value;
          break;
        case "librefang_llm_calls":
          entry.llmCalls = value;
          break;
      }
    }

    if (!version && sample.name === "librefang_info") {
      version = sample.labels.get("version") ?? "";
    }
  }

  return {
    requests,
    agents: Array.from(agentMap.values()).filter(
      (metric) => !metric.agent.includes(":") && hasUsage(metric),
    ),
    system: {
      uptime: gaugeMap.get("librefang_uptime_seconds") ?? 0,
      agentsActive: gaugeMap.get("librefang_agents_active") ?? 0,
      agentsTotal: gaugeMap.get("librefang_agents_total") ?? 0,
      activeSessions: gaugeMap.get("librefang_active_sessions") ?? 0,
      costToday: gaugeMap.get("librefang_cost_usd_today") ?? 0,
      panics: gaugeMap.get("librefang_panics_total") ?? 0,
      restarts: gaugeMap.get("librefang_restarts_total") ?? 0,
      version,
    },
  };
}
