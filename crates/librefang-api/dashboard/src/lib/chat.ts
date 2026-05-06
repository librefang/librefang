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
 *  thinking / text blocks in one turn are joined with a blank line so
 *  the collapsible drawer (and the markdown renderer for the visible
 *  body) read naturally — markdown needs a blank line between adjacent
 *  blocks to produce a paragraph break, otherwise two blocks fuse into
 *  one paragraph.
 *
 *  Block ordering: a turn ordered `[thinking A, text X, thinking B,
 *  text Y]` collapses into `text: "X\n\nY"` and `thinking: "A\n\nB"`,
 *  losing the original interleave. This is intentional and matches the
 *  live-streaming path, where `text_delta` and `thinking_delta` are
 *  accumulated into independent strings in real time. The chat UI
 *  renders thinking in a separate collapsible drawer above the visible
 *  body, so per-turn interleave isn't observable to the user on either
 *  path; reload-time and live-time presentation stay consistent.
 *
 *  `tool_use` / `tool_result` blocks are intentionally ignored here —
 *  the mapper at `ChatPage.tsx:542-579` reads tool data from the
 *  separate `msg.tools` field instead.
 *
 *  `redacted_thinking` blocks (if/when the backend emits them) are
 *  silently skipped by the runtime "type" check below — they fall
 *  through neither the `text` nor the `thinking` branch. A follow-up
 *  will add a placeholder UI; until then, the plaintext-thinking path
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
    // Runtime guard tolerates forward-compat unknown variants without
    // collapsing the union's narrowing (see `ContentBlockUnknown` in
    // `api.ts` for the rationale).
    if (!block || typeof block !== "object" || !("type" in block)) continue;
    // `block.type` narrows cleanly now that `ContentBlock` is a tight
    // discriminated union — no `as` casts needed in the typed branches.
    if (block.type === "text") {
      textParts.push(block.text);
    } else if (block.type === "thinking") {
      thinkingParts.push(block.thinking);
    }
    // tool_use / tool_result / image / image_file / redacted_thinking /
    // unknown future variants — skipped intentionally.
  }
  return {
    // Both buckets join with `\n\n` (paragraph break for markdown);
    // a single `\n` between adjacent blocks would render as one
    // paragraph in react-markdown, fusing distinct chunks.
    text: textParts.join("\n\n"),
    thinking: thinkingParts.join("\n\n"),
  };
}
