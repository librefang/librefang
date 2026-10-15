import { describe, expect, it } from "vitest";

import {
  IDENTITY_FIELD_KEYS,
  identityFieldsDiffer,
  parseIdentityFrontMatter,
  readIdentityField,
  readIdentityFields,
  serializeIdentityFrontMatter,
} from "./identityFrontMatter";

/**
 * The real file of a deployed agent, byte for byte, including the empty keys a
 * different control owns and the operator-facing HTML comment in the body.
 * Every round-trip assertion in this file is anchored on this fixture, so a
 * regression that would rewrite a real identity file fails here first.
 */
const DEPLOYED = `---
name: deannatroi
archetype: assistant
vibe: helpful
emoji:
avatar_url:
greeting_style: warm
color:
---
# Identity
<!-- Visual identity and personality at a glance. Edit these fields freely. -->
`;

/** Parses and re-serializes with the values the file already holds. */
function roundTrip(content: string): string {
  const parsed = parseIdentityFrontMatter(content);
  return serializeIdentityFrontMatter(parsed, readIdentityFields(parsed));
}

describe("parseIdentityFrontMatter", () => {
  it("reads the three owned keys out of a deployed agent file", () => {
    const parsed = parseIdentityFrontMatter(DEPLOYED);
    expect(parsed.kind).toBe("ok");
    expect(readIdentityFields(parsed)).toEqual({
      archetype: "assistant",
      vibe: "helpful",
      greeting_style: "warm",
    });
  });

  it("reports a file with no front-matter block as absent, content intact", () => {
    const parsed = parseIdentityFrontMatter("# Identity\n\nSome prose.\n");
    expect(parsed.kind).toBe("absent");
    expect(parsed.kind === "absent" && parsed.content).toBe("# Identity\n\nSome prose.\n");
  });

  it("reports an empty file as absent", () => {
    const parsed = parseIdentityFrontMatter("");
    expect(parsed.kind).toBe("absent");
  });

  it("reports an opening --- with no closing --- as malformed", () => {
    const content = "---\nvibe: warm\n# Identity\n";
    const parsed = parseIdentityFrontMatter(content);
    expect(parsed.kind).toBe("malformed");
    expect(parsed.kind === "malformed" && parsed.content).toBe(content);
  });

  it("treats a lone horizontal rule as malformed rather than front-matter", () => {
    expect(parseIdentityFrontMatter("---\n\nA document.\n").kind).toBe("malformed");
  });

  it("reads a key with no value as the empty string, not as missing", () => {
    const parsed = parseIdentityFrontMatter("---\nvibe:\n---\n");
    expect(readIdentityField(parsed, "vibe")).toBe("");
    expect(readIdentityField(parsed, "archetype")).toBeNull();
  });

  it("splits on the first colon only, so colons and spaces survive in values", () => {
    const parsed = parseIdentityFrontMatter(
      "---\nvibe: calm: relaxed and warm\narchetype: a b c\n---\n",
    );
    expect(readIdentityField(parsed, "vibe")).toBe("calm: relaxed and warm");
    expect(readIdentityField(parsed, "archetype")).toBe("a b c");
  });

  it("does not hijack an indented key nested under another mapping", () => {
    const parsed = parseIdentityFrontMatter(
      "---\nmetadata:\n  archetype: nested-not-ours\narchetype: top-level\n---\n",
    );
    expect(readIdentityField(parsed, "archetype")).toBe("top-level");
    expect(roundTrip("---\nmetadata:\n  archetype: nested-not-ours\narchetype: top-level\n---\n")).toBe(
      "---\nmetadata:\n  archetype: nested-not-ours\narchetype: top-level\n---\n",
    );
  });

  it("does not read a commented-out key", () => {
    const parsed = parseIdentityFrontMatter("---\n# vibe: warm\n---\n");
    expect(readIdentityField(parsed, "vibe")).toBeNull();
  });

  it("takes the last occurrence when a key is duplicated", () => {
    const parsed = parseIdentityFrontMatter("---\nvibe: first\nvibe: second\n---\n");
    expect(readIdentityField(parsed, "vibe")).toBe("second");
  });
});

describe("serializeIdentityFrontMatter — nothing is lost", () => {
  it("round-trips the deployed file byte for byte", () => {
    expect(roundTrip(DEPLOYED)).toBe(DEPLOYED);
  });

  it("changes only the edited key's line", () => {
    const parsed = parseIdentityFrontMatter(DEPLOYED);
    const output = serializeIdentityFrontMatter(parsed, {
      ...readIdentityFields(parsed),
      vibe: "friendly",
    });
    expect(output).toBe(DEPLOYED.replace("vibe: helpful", "vibe: friendly"));
  });

  it("keeps the body, the comment and the key order when editing", () => {
    const parsed = parseIdentityFrontMatter(DEPLOYED);
    const output = serializeIdentityFrontMatter(parsed, {
      archetype: "researcher",
      vibe: "precise",
      greeting_style: "formal",
    });
    expect(output).toBe(
      DEPLOYED.replace("archetype: assistant", "archetype: researcher")
        .replace("vibe: helpful", "vibe: precise")
        .replace("greeting_style: warm", "greeting_style: formal"),
    );
    // The comment and heading are the operator's; they are not this editor's to touch.
    expect(output).toContain("<!-- Visual identity and personality at a glance. Edit these fields freely. -->");
    expect(output.endsWith("# Identity\n<!-- Visual identity and personality at a glance. Edit these fields freely. -->\n")).toBe(true);
  });

  it("preserves an unknown key's exact spelling, spacing and position", () => {
    const content = "---\nvibe: warm\nname:   deannatroi  \nx-custom: kept\n---\nbody\n";
    const parsed = parseIdentityFrontMatter(content);
    const output = serializeIdentityFrontMatter(parsed, {
      ...readIdentityFields(parsed),
      vibe: "cool",
    });
    expect(output).toBe("---\nvibe: cool\nname:   deannatroi  \nx-custom: kept\n---\nbody\n");
  });

  it("preserves the CRLF line ending", () => {
    const content = "---\r\nvibe: warm\r\n---\r\n# Identity\r\n";
    const parsed = parseIdentityFrontMatter(content);
    expect(parsed.kind === "ok" && parsed.newline).toBe("\r\n");

    const output = serializeIdentityFrontMatter(parsed, {
      ...readIdentityFields(parsed),
      vibe: "cool",
    });
    expect(output).toBe("---\r\nvibe: cool\r\n---\r\n# Identity\r\n");
    expect(output).not.toMatch(/(?<!\r)\n/);
  });

  // Windows Notepad writes a BOM by default, and this is exactly the kind of
  // file an operator opens in Notepad. Left in place, the BOM makes the opening
  // delimiter stop matching, the file reads as "no block at all", the form
  // shows three empty fields over a file full of values, and saving prepends a
  // second block that demotes the original to body text.
  it("sees the block through a byte-order mark and keeps the mark at byte 0", () => {
    const content = "\uFEFF---\nvibe: warm\nname: x\n---\n# Identity\n";
    const parsed = parseIdentityFrontMatter(content);
    expect(parsed.kind).toBe("ok");
    expect(readIdentityFields(parsed)).toEqual({
      archetype: "",
      vibe: "warm",
      greeting_style: "",
    });
    expect(roundTrip(content)).toBe(content);

    const output = serializeIdentityFrontMatter(parsed, {
      ...readIdentityFields(parsed),
      vibe: "cool",
    });
    expect(output).toBe(content.replace("vibe: warm", "vibe: cool"));
    expect(output.startsWith("\uFEFF")).toBe(true);
  });

  it("treats a file that is only a byte-order mark as empty", () => {
    const parsed = parseIdentityFrontMatter("\uFEFF");
    expect(parsed.kind).toBe("absent");
    expect(parsed.kind === "absent" && parsed.content).toBe("");
  });

  it("keeps CR-only line endings when it appends a key", () => {
    const content = "---\rname: deannatroi\r---\r# Identity\r";
    const parsed = parseIdentityFrontMatter(content);
    expect(parsed.kind === "ok" && parsed.newline).toBe("\r");
    expect(roundTrip(content)).toBe(content);

    const output = serializeIdentityFrontMatter(parsed, {
      ...readIdentityFields(parsed),
      archetype: "assistant",
    });
    expect(output).toBe(
      "---\rname: deannatroi\rarchetype: assistant\r---\r# Identity\r",
    );
  });

  it("preserves a file that ends at the closing delimiter with no newline", () => {
    const content = "---\nvibe: warm\n---";
    const parsed = parseIdentityFrontMatter(content);
    const output = serializeIdentityFrontMatter(parsed, {
      ...readIdentityFields(parsed),
      vibe: "cool",
    });
    expect(output).toBe("---\nvibe: cool\n---");
  });

  it("preserves an empty body and a body that starts with a blank line", () => {
    expect(roundTrip("---\nvibe: warm\n---")).toBe("---\nvibe: warm\n---");
    expect(roundTrip("---\nvibe: warm\n---\n\n# Identity\n")).toBe(
      "---\nvibe: warm\n---\n\n# Identity\n",
    );
  });

  it("preserves blank lines, comments and list items inside the block", () => {
    const content = "---\n# a comment\nvibe: warm\n\ntags:\n  - one\n  - two\n---\nbody\n";
    expect(roundTrip(content)).toBe(content);
  });

  it("preserves a value that carries a trailing YAML comment, because nothing parses it as YAML", () => {
    const content = "---\nvibe: warm # not stripped\n---\n";
    expect(roundTrip(content)).toBe(content);
    expect(readIdentityField(parseIdentityFrontMatter(content), "vibe")).toBe("warm # not stripped");
  });

  it("keeps quoting literal rather than interpreting it", () => {
    const content = '---\nvibe: "warm"\n---\n';
    expect(roundTrip(content)).toBe(content);
    expect(readIdentityField(parseIdentityFrontMatter(content), "vibe")).toBe('"warm"');
  });
});

describe("serializeIdentityFrontMatter — the three keys", () => {
  it("appends a key that has no line, at the end of the block", () => {
    const parsed = parseIdentityFrontMatter("---\nvibe: warm\n---\n# Identity\n");
    const output = serializeIdentityFrontMatter(parsed, {
      ...readIdentityFields(parsed),
      archetype: "assistant",
    });
    expect(output).toBe("---\nvibe: warm\narchetype: assistant\n---\n# Identity\n");
  });

  it("does not add a line for a key left empty", () => {
    const parsed = parseIdentityFrontMatter("---\nvibe: warm\n---\n");
    const output = serializeIdentityFrontMatter(parsed, {
      archetype: "",
      vibe: "warm",
      greeting_style: "",
    });
    expect(output).toBe("---\nvibe: warm\n---\n");
  });

  it("clears a key by leaving its line in place, empty", () => {
    const parsed = parseIdentityFrontMatter(DEPLOYED);
    const output = serializeIdentityFrontMatter(parsed, {
      ...readIdentityFields(parsed),
      greeting_style: "",
    });
    expect(output).toBe(DEPLOYED.replace("greeting_style: warm", "greeting_style:"));
  });

  it("gives both lines of a duplicated key the same value", () => {
    const parsed = parseIdentityFrontMatter("---\nvibe: first\nvibe: second\n---\n");
    const output = serializeIdentityFrontMatter(parsed, {
      ...readIdentityFields(parsed),
      vibe: "settled",
    });
    expect(output).toBe("---\nvibe: settled\nvibe: settled\n---\n");
  });

  it("normalises spacing around an owned key but leaves unknown keys alone", () => {
    const content = "---\nvibe:   warm  \nname:   spaced  \n---\n";
    const parsed = parseIdentityFrontMatter(content);
    const output = serializeIdentityFrontMatter(parsed, readIdentityFields(parsed));
    expect(output).toBe("---\nvibe: warm\nname:   spaced  \n---\n");
  });

  it("rewrites every owned key that is present, even when its value is unchanged", () => {
    // A field the operator never typed into is still re-emitted canonically;
    // the contract is that only these three lines can move, not that they do.
    const parsed = parseIdentityFrontMatter("---\narchetype : assistant\n---\n");
    const output = serializeIdentityFrontMatter(parsed, readIdentityFields(parsed));
    expect(output).toBe("---\narchetype: assistant\n---\n");
  });
});

describe("serializeIdentityFrontMatter — no block yet", () => {
  it("prepends a block and keeps the existing body", () => {
    const parsed = parseIdentityFrontMatter("# Identity\n\nSome prose.\n");
    const output = serializeIdentityFrontMatter(parsed, {
      archetype: "assistant",
      vibe: "",
      greeting_style: "warm",
    });
    expect(output).toBe("---\narchetype: assistant\ngreeting_style: warm\n---\n# Identity\n\nSome prose.\n");
  });

  it("returns the file untouched when nothing was entered", () => {
    const content = "# Identity\n";
    const parsed = parseIdentityFrontMatter(content);
    expect(
      serializeIdentityFrontMatter(parsed, { archetype: "", vibe: "", greeting_style: "" }),
    ).toBe(content);
  });

  it("creates a block for an empty file, with a trailing newline", () => {
    const parsed = parseIdentityFrontMatter("");
    expect(
      serializeIdentityFrontMatter(parsed, { archetype: "assistant", vibe: "", greeting_style: "" }),
    ).toBe("---\narchetype: assistant\n---\n");
  });

  it("prepends the block after a byte-order mark, never before it", () => {
    const parsed = parseIdentityFrontMatter("\uFEFF# Identity\n");
    expect(parsed.kind === "absent" && parsed.content).toBe("# Identity\n");

    expect(
      serializeIdentityFrontMatter(parsed, {
        archetype: "assistant",
        vibe: "",
        greeting_style: "",
      }),
    ).toBe("\uFEFF---\narchetype: assistant\n---\n# Identity\n");

    // Nothing entered means nothing written, mark included.
    expect(
      serializeIdentityFrontMatter(parsed, { archetype: "", vibe: "", greeting_style: "" }),
    ).toBe("\uFEFF# Identity\n");
  });

  it("follows the body's CRLF when creating a block", () => {
    const parsed = parseIdentityFrontMatter("# Identity\r\n");
    expect(
      serializeIdentityFrontMatter(parsed, { archetype: "assistant", vibe: "", greeting_style: "" }),
    ).toBe("---\r\narchetype: assistant\r\n---\r\n# Identity\r\n");
  });

  it("refuses to serialize a malformed block instead of guessing", () => {
    const parsed = parseIdentityFrontMatter("---\nvibe: warm\n# Identity\n");
    expect(() =>
      serializeIdentityFrontMatter(parsed, { archetype: "", vibe: "", greeting_style: "" }),
    ).toThrow(/malformed/i);
  });
});

describe("identityFieldsDiffer", () => {
  it("is false when the form still holds what the file holds", () => {
    const parsed = parseIdentityFrontMatter(DEPLOYED);
    expect(identityFieldsDiffer(parsed, readIdentityFields(parsed))).toBe(false);
  });

  it("is true after a field changes, including clearing one", () => {
    const parsed = parseIdentityFrontMatter(DEPLOYED);
    expect(identityFieldsDiffer(parsed, { ...readIdentityFields(parsed), vibe: "other" })).toBe(true);
    expect(identityFieldsDiffer(parsed, { ...readIdentityFields(parsed), vibe: "" })).toBe(true);
  });

  it("ignores whitespace-only differences, which the save normalises away", () => {
    const parsed = parseIdentityFrontMatter(DEPLOYED);
    expect(identityFieldsDiffer(parsed, { ...readIdentityFields(parsed), vibe: "  helpful  " })).toBe(false);
  });

  it("exposes exactly three owned keys", () => {
    expect([...IDENTITY_FIELD_KEYS]).toEqual(["archetype", "vibe", "greeting_style"]);
  });
});
