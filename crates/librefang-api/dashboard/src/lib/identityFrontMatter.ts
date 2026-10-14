/**
 * Lossless front-matter editing for a workspace identity file (`IDENTITY.md`).
 *
 * Nothing parses this file as YAML. `read_identity_file` returns the bytes and
 * the prompt builder injects them verbatim under `## Identity`, capped at 500
 * chars (`crates/librefang-runtime/src/prompt_builder.rs:1182`), so
 * `archetype:`, `vibe:` and `greeting_style:` are a convention the model reads,
 * not a schema the daemon enforces. In particular a trailing `# comment` or a
 * quoted value is text, not syntax — this module never strips either.
 *
 * That asymmetry sets the whole design. The file belongs to the operator and
 * the dashboard is a guest in it: parse-to-object then re-stringify would
 * rewrite the body, reorder keys, drop every key this module does not know
 * about, and normalise CRLF to LF — each one silently destroying work someone
 * did by hand. So the file is never modelled as data. It is split into the
 * lines this editor owns (the three keys) and the spans it must not touch, and
 * `serializeIdentityFrontMatter` reassembles the untouched spans byte for byte.
 *
 * Guarantees, in both directions:
 *   - the body (everything after the closing `---`) is copied verbatim;
 *   - key order is positional, so it cannot change;
 *   - unknown keys, comments, blank lines and nested/indented lines are `raw`
 *     and re-emitted with their original terminator, so CRLF survives;
 *   - a leading byte-order mark stays at byte 0 and no longer hides the block;
 *   - only a line whose key is one of `IDENTITY_FIELD_KEYS` at column 0 can be
 *     rewritten, and it is re-emitted canonically as `key: value`.
 */

/** The front-matter keys this editor owns. Every other key in the block is preserved untouched. */
export const IDENTITY_FIELD_KEYS = ["archetype", "vibe", "greeting_style"] as const;

export type IdentityFieldKey = (typeof IDENTITY_FIELD_KEYS)[number];

/** Keys the file carries but that a different control owns, shown read-only. */
export const IDENTITY_FOREIGN_KEYS = ["emoji", "avatar_url", "color"] as const;

export type IdentityForeignKey = (typeof IDENTITY_FOREIGN_KEYS)[number];

/** One line with the terminator that followed it — the unit of lossless reassembly. */
type SourcedLine = {
  /** The line's text, terminator excluded. */
  text: string;
  /** `\n`, `\r\n`, `\r`, or `""` for the last line of a file with no trailing newline. */
  term: string;
};

export type IdentityFrontMatter =
  | {
      /** A `---` block was found and closed. */
      kind: "ok";
      /** `"\uFEFF"` when the file opened with a BOM, `""` otherwise. Re-emitted first on write. */
      bom: string;
      open: SourcedLine;
      /** Every line between the delimiters, in file order. */
      inner: SourcedLine[];
      close: SourcedLine;
      /** Everything after the closing delimiter, verbatim, terminators included. */
      body: string;
      /** Terminator to use for lines this module appends. */
      newline: string;
    }
  | {
      /** No leading `---` block at all. `content` is the file after any BOM, verbatim. */
      kind: "absent";
      bom: string;
      content: string;
      newline: string;
    }
  | {
      /**
       * Opens with `---` and never closes it. Left strictly alone: the opening
       * line may be a markdown horizontal rule rather than truncated
       * front-matter, and there is no way to tell which — so the editor goes
       * read-only instead of guessing at a rewrite.
       */
      kind: "malformed";
      bom: string;
      content: string;
      newline: string;
    };

/**
 * A byte-order mark.
 *
 * Split off before the block is located and put back on write. Without that,
 * a BOM-prefixed file — which is what Windows Notepad writes by default, and
 * this is exactly the kind of file an operator opens in Notepad — reads as
 * "no front-matter block at all", because a delimiter with U+FEFF in front of
 * it is not a delimiter. The editor would then show three empty fields over a
 * file that has values in it, and saving would prepend a *second* block,
 * demoting the original to body text that the prompt builder still injects but
 * nothing parses.
 */
const BOM = "\uFEFF";

/**
 * A `---` delimiter line. Trailing spaces and tabs are tolerated.
 *
 * No BOM arm: `parseIdentityFrontMatter` strips a leading U+FEFF before any
 * line is examined, so the delimiter is never seen with one attached.
 */
const DELIMITER_RE = /^---[ \t]*$/;

/** Splits on any terminator while keeping each one attached to the line it ended. */
function splitSourced(content: string): SourcedLine[] {
  // Odd indices are the captured terminators, even indices the line text. The
  // array always ends on a (possibly empty) line, so a file ending in a
  // newline is represented as a final terminator-less empty line.
  const parts = content.split(/(\r\n|\r|\n)/);
  const lines: SourcedLine[] = [];
  for (let index = 0; index < parts.length; index += 2) {
    lines.push({ text: parts[index] ?? "", term: parts[index + 1] ?? "" });
  }
  return lines;
}

/**
 * The file's dominant terminator, so a line this module appends matches its
 * neighbours. Ties go to CRLF; a file with no terminator at all gets LF.
 */
function detectNewline(content: string): string {
  const crlf = (content.match(/\r\n/g) ?? []).length;
  const lf = (content.match(/\n/g) ?? []).length - crlf;
  const cr = (content.match(/\r/g) ?? []).length - crlf;
  if (cr > crlf && cr > lf) return "\r";
  if (crlf === 0) return "\n";
  return lf > crlf ? "\n" : "\r\n";
}

/**
 * Parses a `key: value` line.
 *
 * Returns `null` for anything that is not a top-level scalar entry: indented
 * lines (a nested mapping's keys look exactly like ours and must not be
 * hijacked), list items, comments, and lines without a colon. Only the first
 * colon separates — `/api/v1` and `calm: relaxed` are values, not keys.
 */
function parseField(text: string): { key: string; value: string } | null {
  if (text.length === 0) return null;
  if (/^[ \t]/.test(text)) return null;
  const colon = text.indexOf(":");
  if (colon <= 0) return null;
  const key = text.slice(0, colon).trim();
  if (key.length === 0 || /\s/.test(key)) return null;
  return { key, value: text.slice(colon + 1).trim() };
}

function isIdentityFieldKey(key: string): key is IdentityFieldKey {
  return (IDENTITY_FIELD_KEYS as readonly string[]).includes(key);
}

/** Parses an identity file's bytes into the pieces the editor may and may not touch. */
export function parseIdentityFrontMatter(content: string): IdentityFrontMatter {
  // The BOM is split off rather than matched around: it belongs at byte 0, and
  // `absent.content` is what a new block gets prepended to, so leaving it in
  // would push it into the middle of the file on the first save.
  const bom = content.startsWith(BOM) ? BOM : "";
  const rest = bom ? content.slice(BOM.length) : content;

  const newline = detectNewline(rest);
  const lines = splitSourced(rest);

  const first = lines[0];
  if (!first || !DELIMITER_RE.test(first.text)) {
    return { kind: "absent", bom, content: rest, newline };
  }

  const closeIndex = lines.findIndex(
    (line, index) => index > 0 && DELIMITER_RE.test(line.text),
  );
  if (closeIndex === -1) {
    return { kind: "malformed", bom, content: rest, newline };
  }

  const close = lines[closeIndex];

  return {
    kind: "ok",
    bom,
    open: first,
    inner: lines.slice(1, closeIndex),
    close,
    body: lines
      .slice(closeIndex + 1)
      .map((line) => line.text + line.term)
      .join(""),
    newline,
  };
}

/**
 * Reads one key's value from the block, or `null` when the key has no line.
 *
 * The last occurrence wins, matching how a reader scanning top-to-bottom would
 * end up interpreting the block.
 */
export function readIdentityField(
  source: IdentityFrontMatter,
  key: string,
): string | null {
  if (source.kind !== "ok") return null;
  let found: string | null = null;
  for (const line of source.inner) {
    const field = parseField(line.text);
    if (field?.key === key) found = field.value;
  }
  return found;
}

/** The three owned fields, with absent keys read as the empty string. */
export function readIdentityFields(
  source: IdentityFrontMatter,
): Record<IdentityFieldKey, string> {
  return {
    archetype: readIdentityField(source, "archetype") ?? "",
    vibe: readIdentityField(source, "vibe") ?? "",
    greeting_style: readIdentityField(source, "greeting_style") ?? "",
  };
}

/** `key: value`, or `key:` for an empty value — the shape the generated template already uses. */
function formatField(key: string, value: string): string {
  const trimmed = value.trim();
  return trimmed.length === 0 ? `${key}:` : `${key}: ${trimmed}`;
}

/**
 * Reassembles the file from an unedited parse plus new values for the three
 * owned keys.
 *
 * Only lines whose key is one of `IDENTITY_FIELD_KEYS` at column 0 are
 * rewritten; they are re-emitted canonically as `key: value`. Every other
 * byte — body, comments, key order, unknown keys, terminators — is copied
 * through. A key with no line is appended at the end of the block, and only
 * when it has a value: an empty new field must not add noise to a file the
 * operator never opted into.
 *
 * Duplicate lines for the same owned key all receive the same value, since the
 * form offers exactly one control per key and leaving a contradictory twin
 * behind would make the next read disagree with what was just saved.
 *
 * A byte-order mark comes back first, exactly where it was.
 *
 * @throws when `source` is `malformed` — there is no block to write into, and
 *   silently returning the input would let a caller believe a save happened.
 */
export function serializeIdentityFrontMatter(
  source: IdentityFrontMatter,
  values: Record<IdentityFieldKey, string>,
): string {
  if (source.kind === "malformed") {
    throw new Error(
      "Cannot serialize a malformed identity front-matter block; the editor must stay read-only.",
    );
  }

  if (source.kind === "absent") {
    const added = IDENTITY_FIELD_KEYS.filter((key) => values[key].trim().length > 0);
    if (added.length === 0) return source.bom + source.content;
    const block = [
      "---",
      ...added.map((key) => formatField(key, values[key])),
      "---",
    ].join(source.newline);
    return source.bom + block + source.newline + source.content;
  }

  const appendedTerm = source.close.term || source.newline;
  const rewritten: string[] = [];
  const seen = new Set<IdentityFieldKey>();

  for (const line of source.inner) {
    const field = parseField(line.text);
    if (!field || !isIdentityFieldKey(field.key)) {
      rewritten.push(line.text + line.term);
      continue;
    }
    rewritten.push(formatField(field.key, values[field.key]) + line.term);
    seen.add(field.key);
  }

  for (const key of IDENTITY_FIELD_KEYS) {
    if (seen.has(key) || values[key].trim().length === 0) continue;
    rewritten.push(formatField(key, values[key]) + appendedTerm);
  }

  return (
    source.bom +
    source.open.text +
    source.open.term +
    rewritten.join("") +
    source.close.text +
    source.close.term +
    source.body
  );
}

/** True when any owned field differs from what the file currently holds. */
export function identityFieldsDiffer(
  source: IdentityFrontMatter,
  values: Record<IdentityFieldKey, string>,
): boolean {
  const current = readIdentityFields(source);
  return IDENTITY_FIELD_KEYS.some(
    (key) => current[key].trim() !== values[key].trim(),
  );
}
