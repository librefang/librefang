import { describe, expect, it } from "vitest";
import {
  conversationToMarkdown,
  exportFilename,
  fenceFor,
  type ExportableMessage,
} from "./chatExport";

const AT = new Date(2026, 8, 12, 17, 20);

function msg(over: Partial<ExportableMessage> = {}): ExportableMessage {
  return {
    id: "m1",
    role: "assistant",
    content: "hello",
    timestamp: new Date(2026, 8, 12, 17, 19),
    ...over,
  };
}

describe("conversationToMarkdown", () => {
  it("keeps message bodies as Markdown rather than fencing them", () => {
    // The bodies already are Markdown. Fencing them would turn a table, a
    // diagram or a heading back into the source someone was reading past.
    const out = conversationToMarkdown(
      [msg({ content: "| a | b |\n| - | - |\n| 1 | 2 |" })],
      { agentName: "Deanna", exportedAt: AT },
    );
    expect(out).toContain("| a | b |");
    expect(out).not.toContain("```\n| a | b |");
  });

  it("grows the reasoning fence past any backticks inside it", () => {
    // Model output is mostly code, so a fixed ``` fence around content that
    // contains one ends early and the rest of the transcript renders as prose.
    const thinking = "I will write:\n```rust\nfn main() {}\n```\nthen stop.";
    const out = conversationToMarkdown([msg({ thinking })], {
      agentName: "Deanna",
      exportedAt: AT,
      includeThinking: true,
    });
    expect(out).toContain("````");
    // The inner fence survives intact rather than terminating the block.
    expect(out).toContain("```rust");
  });

  it("leaves reasoning out unless asked", () => {
    const out = conversationToMarkdown([msg({ thinking: "secret plan" })], {
      agentName: "Deanna",
      exportedAt: AT,
    });
    expect(out).not.toContain("secret plan");
    expect(out).not.toContain("Reasoning");
  });

  it("annotates only the messages that have something to annotate", () => {
    const out = conversationToMarkdown(
      [
        msg({ id: "a", tokens: { input: 10, output: 20 }, cost_usd: 0.0012 }),
        msg({ id: "b", content: "plain" }),
      ],
      { agentName: "Deanna", exportedAt: AT },
    );
    expect(out).toContain("10 in / 20 out tokens · $0.0012");
    // Exactly one footnote: a row of zeroes under every message would bury
    // the conversation it annotates.
    expect(out.match(/<sub>/g)).toHaveLength(1);
  });

  it("says so when a message is empty instead of emitting a blank heading", () => {
    const out = conversationToMarkdown([msg({ content: "   " })], {
      agentName: "Deanna",
      exportedAt: AT,
    });
    expect(out).toContain("_(no content)_");
  });

  it("carries the session so an exported file can be traced back", () => {
    const out = conversationToMarkdown([msg()], {
      agentName: "Deanna",
      exportedAt: AT,
      sessionId: "2d3bd80c",
    });
    expect(out).toContain("Session `2d3bd80c`");
  });

  it("records an error next to the message it belongs to", () => {
    const out = conversationToMarkdown([msg({ error: "provider refused" })], {
      agentName: "Deanna",
      exportedAt: AT,
    });
    expect(out).toContain("> **Error:** provider refused");
  });
});

describe("fenceFor", () => {
  it("returns the shortest fence that cannot be closed from inside", () => {
    expect(fenceFor("no backticks")).toBe("```");
    expect(fenceFor("a ``` b")).toBe("````");
    expect(fenceFor("a ````` b")).toBe("``````");
  });
});

describe("exportFilename", () => {
  it("survives a name a filesystem would not accept", () => {
    // A slash in a download name either fails or writes somewhere nobody
    // expects, and an agent may legitimately be called this.
    expect(exportFilename("Deanna/Troi 🖖", AT)).toBe("Deanna-Troi-2026-09-12-17-20.md");
  });

  it("still produces a name when nothing usable survives", () => {
    expect(exportFilename("🖖", AT)).toBe("conversation-2026-09-12-17-20.md");
  });
});

describe("what the previous inline export already did", () => {
  // Replacing an existing feature must not quietly drop part of it.
  it("still lists the tools a turn called", () => {
    const out = conversationToMarkdown(
      [msg({ tools: [{ name: "file_read" }, { name: "shell" }] })],
      { agentName: "Deanna", exportedAt: AT },
    );
    expect(out).toContain("_Tools: file_read, shell_");
  });

  it("still states how many messages the file holds", () => {
    const out = conversationToMarkdown([msg({ id: "a" }), msg({ id: "b" })], {
      agentName: "Deanna",
      exportedAt: AT,
    });
    expect(out).toContain("2 messages.");
    expect(conversationToMarkdown([msg()], { agentName: "D", exportedAt: AT })).toContain(
      "1 message.",
    );
  });
});
