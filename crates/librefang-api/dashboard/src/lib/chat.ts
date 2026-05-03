import { formatCost } from "./format";
import type { ContentBlock } from "../api";

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
    parts.push(formatCost(response.cost_usd));
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

/** Result of walking a persisted assistant message's `content` field
 *  (`string | ContentBlock[]`) and pulling out the two display strings
 *  the chat UI tracks: visible text and the collapsible reasoning trace.
 *
 *  Mirrors the live-streaming model where `ChatMessage.thinking` is a
 *  flat string accumulated from `thinking_delta` events. Multiple
 *  thinking blocks in one turn are joined with a blank line so the
 *  collapsible drawer reads naturally.
 *
 *  `tool_use` / `tool_result` blocks are intentionally ignored here —
 *  the mapper at `ChatPage.tsx:542-579` reads tool data from the
 *  separate `msg.tools` field instead.
 *
 *  `redacted_thinking` blocks (if/when the backend emits them) are
 *  silently skipped via the unknown-variant fallback. A follow-up will
 *  add a placeholder UI; until then, the plaintext-thinking path
 *  matches the live-streaming behavior. */
export interface AssistantHistoryParts {
  text: string;
  thinking: string;
}

export function extractAssistantHistoryParts(
  content: string | ContentBlock[] | null | undefined,
): AssistantHistoryParts {
  if (content == null) return { text: "", thinking: "" };
  if (typeof content === "string") return { text: content, thinking: "" };
  if (!Array.isArray(content)) return { text: String(content), thinking: "" };

  const textParts: string[] = [];
  const thinkingParts: string[] = [];
  for (const block of content) {
    if (block && typeof block === "object" && "type" in block) {
      if (block.type === "text" && typeof (block as { text?: unknown }).text === "string") {
        textParts.push((block as { text: string }).text);
      } else if (
        block.type === "thinking" &&
        typeof (block as { thinking?: unknown }).thinking === "string"
      ) {
        thinkingParts.push((block as { thinking: string }).thinking);
      }
      // tool_use / tool_result / image / image_file / redacted_thinking /
      // unknown future variants — skipped intentionally.
    }
  }
  return {
    text: textParts.join("\n"),
    thinking: thinkingParts.join("\n\n"),
  };
}
