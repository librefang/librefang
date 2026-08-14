// CSV parser for the user bulk-import wizard.
//
// Hardened against two real-world failure modes the previous hand-rolled
// `split('\n').split(',')` parser hit:
//
//   1. UTF-8 BOM prefix on files exported from Excel — without stripping it,
//      the first header column contains a hidden marker before "name" and the
//      `header.includes("name")` check fails, rejecting the whole import.
//
//   2. Quoted fields with embedded newlines (RFC-4180 §2.6: "Fields
//      containing line breaks (CRLF), double quotes, and commas should be
//      enclosed in double-quotes."). Splitting on `\n` first chops the
//      record into garbage rows.
//
// This parser is a single-pass tokenizer over the full input. It does not
// pre-split into lines, so embedded newlines inside `"…"` survive intact.

export type CsvParseResult = {
  /** One entry per record (= one row of cells), header row included. */
  records: string[][];
  /** Soft syntax errors. Records are retained so callers can preview them. */
  errors: string[];
};

/**
 * Tokenize CSV text into records. Handles:
 * - Leading UTF-8 BOM (stripped before parsing).
 * - Quoted fields with embedded `,`, `\r`, `\n`, and `""` (escaped quote).
 * - CRLF, LF, and CR line endings (CR-only is rare but legal in old Mac files).
 * - Trailing newline (does not emit a phantom empty record).
 */
export function parseCsvText(input: string): CsvParseResult {
  const errors: string[] = [];
  // Strip BOM. Only the first code unit can be the BOM marker.
  let raw = input;
  if (raw.charCodeAt(0) === 0xfeff) {
    raw = raw.slice(1);
  }

  const records: string[][] = [];
  let cur = "";
  let row: string[] = [];
  let quoted = false;
  let closedQuote = false;
  let sawAnyChar = false;

  for (let i = 0; i < raw.length; i++) {
    const c = raw[i];
    if (quoted) {
      if (c === '"') {
        if (raw[i + 1] === '"') {
          cur += '"';
          i++;
        } else {
          quoted = false;
          closedQuote = true;
        }
      } else {
        // Inside quotes, newlines and commas are literal.
        cur += c;
      }
      sawAnyChar = true;
      continue;
    }

    if (c === '"') {
      if (cur.length > 0) {
        errors.push(
          `Record ${records.length + 1}: unexpected quote in an unquoted field; treating it literally.`,
        );
        cur += c;
        closedQuote = false;
      } else {
        quoted = true;
      }
      sawAnyChar = true;
    } else if (c === ",") {
      row.push(cur);
      cur = "";
      closedQuote = false;
      sawAnyChar = true;
    } else if (c === "\r" || c === "\n") {
      // End of record. Consume CRLF as a single delimiter.
      row.push(cur);
      records.push(row);
      row = [];
      cur = "";
      closedQuote = false;
      sawAnyChar = false;
      if (c === "\r" && raw[i + 1] === "\n") {
        i++;
      }
    } else {
      if (closedQuote) {
        errors.push(
          `Record ${records.length + 1}: unexpected character after a closing quote; treating it literally.`,
        );
        closedQuote = false;
      }
      cur += c;
      sawAnyChar = true;
    }
  }

  // Flush the final record if the file didn't end with a newline, OR if it
  // did but we have buffered content (impossible after the branch above, but
  // covered for safety).
  if (sawAnyChar || cur.length > 0 || row.length > 0) {
    if (quoted) {
      errors.push(`Record ${records.length + 1}: unterminated quoted field.`);
    }
    row.push(cur);
    records.push(row);
  }

  return { records, errors };
}

// ---------------------------------------------------------------------------
// User-import semantic layer
// ---------------------------------------------------------------------------

export type UserCsvRow = {
  name: string;
  role: string;
  channel_bindings: Record<string, string>;
};

export type ParseUsersCsvResult = {
  rows: UserCsvRow[];
  errors: string[];
};

/**
 * Parse and validate user-import CSV content.
 *
 * @param raw  Raw file or pasted text (BOM tolerated).
 * @param validRoles  List of permitted role names. Rows with other values are
 *                    pushed through verbatim and an error is appended; the
 *                    caller decides whether to commit (matches the previous
 *                    behaviour — the wizard surfaces errors in the preview).
 */
export function parseUsersCsv(
  raw: string,
  validRoles: readonly string[],
): ParseUsersCsvResult {
  const { records, errors: syntaxErrors } = parseCsvText(raw);

  // Drop blank records (entirely empty cells, e.g. trailing newlines or
  // blank lines between rows). A record like `[""]` is one empty cell.
  const nonBlank = records
    .map((cells, index) => ({ cells, sourceRow: index + 1 }))
    .filter(({ cells }) => cells.some(cell => cell.trim() !== ""));
  if (nonBlank.length === 0) {
    return { rows: [], errors: syntaxErrors };
  }

  const errors = [...syntaxErrors];
  const header = nonBlank[0].cells.map(h => h.trim().toLowerCase());
  const seenHeaders = new Set<string>();
  for (const column of header) {
    if (seenHeaders.has(column)) {
      errors.push(`Header: duplicate column '${column}'.`);
    }
    seenHeaders.add(column);
  }

  if (!header.includes("name")) {
    errors.push("Header must include a `name` column.");
    return { rows: [], errors };
  }
  if (!header.includes("role")) {
    errors.push("Header must include a `role` column.");
    return { rows: [], errors };
  }

  const rows: UserCsvRow[] = [];
  for (let i = 1; i < nonBlank.length; i++) {
    const { cells, sourceRow } = nonBlank[i];
    const get = (key: string): string => {
      const idx = header.indexOf(key);
      return idx >= 0 ? cells[idx] ?? "" : "";
    };
    const name = get("name").trim();
    if (!name) {
      errors.push(`Row ${sourceRow}: missing name`);
      continue;
    }
    const role = (get("role") || "user").trim().toLowerCase();
    if (!validRoles.includes(role)) {
      errors.push(
        `Row ${sourceRow}: invalid role '${role}' (expected one of ${validRoles.join(
          ", ",
        )})`,
      );
    }
    const channel_bindings: Record<string, string> = {};
    for (const col of header) {
      if (col === "name" || col === "role") continue;
      const val = get(col).trim();
      if (val) channel_bindings[col] = val;
    }
    rows.push({ name, role, channel_bindings });
  }
  return { rows, errors };
}
