import { createClientId } from "./store";
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
  if (raw === "Assistant") return "assistant";
  if (import.meta.env.DEV) {
    console.warn(`normalizeRole: unknown role "${raw}", falling back to "assistant"`);
  }
  return "assistant";
}

export function asText(value: unknown): string {
  if (typeof value === "string") return value;
  if (value == null) return "";
  try {
    const seen = new WeakSet();
    return JSON.stringify(value, (_, v) => {
      if (typeof v === "object" && v !== null) {
        if (seen.has(v)) return "[circular]";
        seen.add(v);
      }
      return v;
    }, 2);
  } catch {
    return "[unserializable]";
  }
}

export function formatMeta(response: {
  input_tokens?: number;
  output_tokens?: number;
  iterations?: number;
  cost_usd?: number;
}): string {
  const parts: string[] = [];
  if (response.input_tokens != null || response.output_tokens != null) {
    parts.push(`${response.input_tokens ?? 0} in / ${response.output_tokens ?? 0} out`);
  }
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
    id: `${tool}-${createClientId()}`,
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

/** Terminal WS frame types — the frames that end a turn's lifecycle (#6390). */
export function isTerminalFrameType(t: unknown): t is "response" | "silent_complete" | "error" {
  return t === "response" || t === "silent_complete" || t === "error";
}

/**
 * Decide which turn a terminal WS frame belongs to (#6390).
 *
 * The daemon echoes the `message_id` the client sent with the turn.
 * A terminal frame can arrive after a newer turn replaced the socket listener — post-turn memory extraction delays `response` past the next user send — so a frame whose echoed id differs from the current bubble's id belongs to a previous turn ("foreign") and must be routed to its own bubble, never bound to the current one.
 * Frames without an id (pre-correlation daemons) are treated as the current turn's, preserving the legacy behavior.
 */
export function terminalFrameOwner(
  frameMessageId: unknown,
  currentBotMsgId: string,
): "current" | "foreign" {
  if (typeof frameMessageId !== "string" || frameMessageId.length === 0) return "current";
  return frameMessageId === currentBotMsgId ? "current" : "foreign";
}

/**
 * The subset of a chat message the terminal-frame router reads and writes.
 *
 * Kept structural rather than importing the page's `ChatMessage` so this lib module stays free of a lib→page dependency cycle; `ChatMessage` satisfies this shape.
 */
export interface TerminalRoutableMessage {
  id: string;
  content: string;
  isStreaming?: boolean;
  error?: string;
  tokens?: { input?: number; output?: number };
  cost_usd?: number;
  memories_saved?: string[];
  memories_used?: string[];
  thinking?: string;
  thinkingCollapsed?: boolean;
}

/** A terminal WS frame carrying the `message_id` that correlates it to its turn (#6390). */
export interface TerminalFrame {
  type: "response" | "silent_complete" | "error";
  message_id: string;
  content?: string;
  output_tokens?: number;
  input_tokens?: number;
  cost_usd?: number;
  memories_saved?: string[];
  memories_used?: string[];
  thinking?: unknown;
}

/**
 * Apply a *foreign* terminal frame — one whose `message_id` identifies a previous turn's bubble, not the current turn's — to the message list (#6390).
 *
 * This is the routing effect that prevents the original misbind: a late `response` from turn A, delivered to turn B's socket listener after B replaced it, patches A's own bubble and leaves every other bubble (B's in-flight one) untouched.
 * `silent_complete` removes the owning bubble; `error` marks it with the failure.
 * Pure so the #6390 race is unit-testable without mounting the chat page.
 */
export function applyForeignTerminalFrame<M extends TerminalRoutableMessage>(
  messages: M[],
  frame: TerminalFrame,
): M[] {
  const owner = frame.message_id;
  if (frame.type === "silent_complete") {
    return messages.filter(m => m.id !== owner);
  }
  if (frame.type === "response") {
    return messages.map((m): M =>
      m.id === owner
        ? {
            ...m,
            content: frame.content || m.content,
            isStreaming: false,
            tokens: { output: frame.output_tokens, input: frame.input_tokens },
            cost_usd: frame.cost_usd,
            memories_saved: frame.memories_saved,
            memories_used: frame.memories_used,
            thinking: typeof frame.thinking === "string" ? frame.thinking : m.thinking,
            thinkingCollapsed: m.thinkingCollapsed ?? true,
          }
        : m,
    );
  }
  return messages.map((m): M =>
    m.id === owner
      ? { ...m, isStreaming: false, error: frame.content || "WebSocket error" }
      : m,
  );
}
