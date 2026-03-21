export type ChatRole = "user" | "assistant" | "system";

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
