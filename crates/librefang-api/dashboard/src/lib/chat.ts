export type ChatRole = "user" | "assistant" | "system";

export interface ToolOutputEntry {
  id: string;
  tool: string;
  content: string;
  isError: boolean;
  timestamp: Date;
}

export function normalizeRole(raw?: string): ChatRole {
  if (raw === "User") return "user";
  if (raw === "System") return "system";
  return "assistant";
}

export function asText(value: unknown): string {
  if (typeof value === "string") return value;
  if (value == null) return "";
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

export function formatMeta(response: {
  input_tokens?: number;
  output_tokens?: number;
  iterations?: number;
  cost_usd?: number;
}): string {
  const parts = [`${response.input_tokens ?? 0} in / ${response.output_tokens ?? 0} out`];
  if (typeof response.iterations === "number" && response.iterations > 0) {
    parts.push(`${response.iterations} iter`);
  }
  if (typeof response.cost_usd === "number") {
    parts.push(`$${response.cost_usd.toFixed(4)}`);
  }
  return parts.join(" | ");
}

export function normalizeToolOutput(event: {
  tool?: unknown;
  result?: unknown;
  is_error?: unknown;
}): ToolOutputEntry | null {
  const tool = typeof event.tool === "string" ? event.tool.trim() : "";
  if (!tool) return null;

  const isError = Boolean(event.is_error);
  const rawResult = asText(event.result).trim();
  const content = rawResult || (isError ? "Tool failed without a preview." : "Tool finished.");

  return {
    id: `${tool}-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
    tool,
    content,
    isError,
    timestamp: new Date(),
  };
}
