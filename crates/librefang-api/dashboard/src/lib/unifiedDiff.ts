// Minimal line-by-line unified diff for the skill-workshop pending review
// UI (#5819). The dashboard has no diff dependency, and pulling one in for a
// single inline preview isn't worth the bundle cost, so this implements the
// classic Myers-style LCS over *lines* and emits unified-diff rows.
//
// Scope: this is a display helper, not a patch format. It produces enough to
// render an inline old-vs-new preview (added / removed / unchanged lines); it
// does not emit `@@` hunk headers or attempt intra-line word diffing.

export type DiffLineKind = "context" | "add" | "remove";

export interface DiffLine {
  kind: DiffLineKind;
  /** Line text without the trailing newline. */
  text: string;
}

/**
 * Compute a line-level unified diff between `oldText` and `newText`.
 *
 * Returns an ordered list of diff lines: removed lines (from `oldText`) and
 * added lines (from `newText`) around the unchanged context. The longest
 * common subsequence of lines is treated as context.
 */
export function unifiedLineDiff(oldText: string, newText: string): DiffLine[] {
  const a = splitLines(oldText);
  const b = splitLines(newText);

  // LCS length table over lines. n/m are bounded by the skill body size,
  // which the daemon caps (MAX_PROMPT_CONTEXT_CHARS) — fine for an O(n*m)
  // table here.
  const n = a.length;
  const m = b.length;
  const lcs: number[][] = Array.from({ length: n + 1 }, () =>
    new Array<number>(m + 1).fill(0),
  );
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      lcs[i][j] =
        a[i] === b[j]
          ? lcs[i + 1][j + 1] + 1
          : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
    }
  }

  const out: DiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      out.push({ kind: "context", text: a[i] });
      i++;
      j++;
    } else if (lcs[i + 1][j] >= lcs[i][j + 1]) {
      out.push({ kind: "remove", text: a[i] });
      i++;
    } else {
      out.push({ kind: "add", text: b[j] });
      j++;
    }
  }
  while (i < n) {
    out.push({ kind: "remove", text: a[i] });
    i++;
  }
  while (j < m) {
    out.push({ kind: "add", text: b[j] });
    j++;
  }
  return out;
}

/**
 * Whether a diff has any actual changes (at least one add/remove line). Used
 * to fall back to a "no changes" hint instead of rendering an all-context
 * block.
 */
export function hasChanges(lines: DiffLine[]): boolean {
  return lines.some((l) => l.kind !== "context");
}

// Split into lines, dropping a single trailing empty line produced by a
// terminal newline so the diff doesn't show a spurious blank tail row. An
// empty string is zero lines (not one empty line), so an empty old body
// diffs as a pure insertion rather than a phantom remove/add of "".
function splitLines(text: string): string[] {
  if (text === "") {
    return [];
  }
  const lines = text.split("\n");
  if (lines.length > 1 && lines[lines.length - 1] === "") {
    lines.pop();
  }
  return lines;
}
