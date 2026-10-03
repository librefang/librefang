// Turning a conversation into a file someone can keep.
//
// Kept out of `ChatPage` because the interesting part is a pure function over
// messages, and a pure function is testable without mounting a chat, a socket
// and a router. The page owns the button; this owns the document.

/** The subset of a chat message this needs. Structural, so `ChatMessage` fits. */
export interface ExportableMessage {
  id: string;
  role: "user" | "assistant" | "system";
  content: string;
  timestamp: Date;
  /** Reasoning trace, when the agent emitted one and the operator kept it. */
  thinking?: string;
  error?: string;
  tokens?: { input?: number; output?: number };
  cost_usd?: number;
  /** Tool calls the turn made. The existing export listed these and dropping
   *  them would lose information someone already relied on.
   *
   *  `name` is optional to match `AgentTool`, whose shape this has to accept
   *  structurally; an entry without one contributes nothing to read and is
   *  left out rather than rendered as `undefined`. */
  tools?: { name?: string }[];
}

export interface ExportOptions {
  agentName: string;
  /** Included in the header so an exported file can be traced back. */
  sessionId?: string | null;
  /** Reasoning traces are off by default: they are long, and they are the part
   *  most likely to carry something the operator would not paste elsewhere. */
  includeThinking?: boolean;
  /** Exact moment the export was taken, supplied so the output is testable. */
  exportedAt: Date;
}

/** `2026-09-12 17:20` — sortable, unambiguous, no locale surprises. */
function stamp(d: Date): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return (
    `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ` +
    `${pad(d.getHours())}:${pad(d.getMinutes())}`
  );
}

/**
 * Fence a block so it cannot break out of its own fence.
 *
 * Model output contains ``` constantly — it is mostly code. A fixed
 * three-backtick fence around content that itself contains one ends the block
 * early and the rest of the message renders as prose, silently mangling the
 * transcript. CommonMark allows longer fences, so the fence grows past the
 * longest run inside.
 */
export function fenceFor(content: string): string {
  const longest = (content.match(/`+/g) ?? []).reduce(
    (max, run) => Math.max(max, run.length),
    0,
  );
  return "`".repeat(Math.max(3, longest + 1));
}

const ROLE_LABEL: Record<ExportableMessage["role"], string> = {
  user: "You",
  assistant: "Assistant",
  system: "System",
};

/**
 * Render a conversation as Markdown.
 *
 * Message bodies are emitted verbatim rather than fenced: they are already
 * Markdown, and wrapping them would turn a diagram or a table back into source.
 * The thinking trace *is* fenced, because it is a transcript of reasoning
 * rather than authored prose and its headings would otherwise fight the
 * document's own.
 */
export function conversationToMarkdown(
  messages: ExportableMessage[],
  options: ExportOptions,
): string {
  const lines: string[] = [
    `# Conversation with ${options.agentName}`,
    "",
    `Exported ${stamp(options.exportedAt)}.`,
  ];
  if (options.sessionId) {
    lines.push(`Session \`${options.sessionId}\`.`);
  }
  lines.push(`${messages.length} message${messages.length === 1 ? "" : "s"}.`, "", "---", "");

  for (const message of messages) {
    lines.push(`## ${ROLE_LABEL[message.role]} · ${stamp(message.timestamp)}`, "");

    if (options.includeThinking && message.thinking?.trim()) {
      const fence = fenceFor(message.thinking);
      lines.push("<details><summary>Reasoning</summary>", "", fence, message.thinking.trim(), fence, "", "</details>", "");
    }

    lines.push(message.content.trim() === "" ? "_(no content)_" : message.content.trim(), "");

    const toolNames = (message.tools ?? [])
      .map((tool) => tool.name)
      .filter((name): name is string => !!name);
    if (toolNames.length > 0) {
      lines.push(`_Tools: ${toolNames.join(", ")}_`, "");
    }

    if (message.error) {
      lines.push(`> **Error:** ${message.error}`, "");
    }

    // Only when there is something to say — a footnote of zeroes on every
    // message would bury the conversation it annotates.
    const parts: string[] = [];
    const inTokens = message.tokens?.input;
    const outTokens = message.tokens?.output;
    if (inTokens || outTokens) {
      parts.push(`${inTokens ?? 0} in / ${outTokens ?? 0} out tokens`);
    }
    if (message.cost_usd) {
      parts.push(`$${message.cost_usd.toFixed(4)}`);
    }
    if (parts.length > 0) {
      lines.push(`<sub>${parts.join(" · ")}</sub>`, "");
    }
  }

  return `${lines.join("\n").trimEnd()}\n`;
}

/**
 * A filename that is safe on every filesystem and still says what it holds.
 *
 * Anything outside `[A-Za-z0-9._-]` becomes `-`: an agent may legitimately be
 * called `Deanna/Troi` or carry an emoji, and a browser download named with a
 * slash in it either fails or writes somewhere nobody expects.
 */
export function exportFilename(agentName: string, exportedAt: Date): string {
  const safe = agentName.replace(/[^A-Za-z0-9._-]+/g, "-").replace(/^-+|-+$/g, "");
  const d = stamp(exportedAt).replace(/[: ]/g, "-");
  return `${safe || "conversation"}-${d}.md`;
}
