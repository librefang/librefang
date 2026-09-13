// Client mirror of the server's name rule for knowledge bases and documents.
//
// The authority is `is_safe_name` in `crates/librefang-api/src/routes/knowledge.rs`.
// This copy exists only so an operator gets a specific message before the round
// trip; anything it is unsure about is left to the server's 400.
//
// It is a denylist, not an alphabet. `Manual de operaciones`, `Informe Q3
// (final).pdf` and `運用マニュアル.md` are ordinary things to want, and an
// `[A-Za-z0-9._-]` allowlist declares most of the world's writing systems
// invalid. Only the classes that are dangerous or that no filesystem stores
// faithfully are refused.
//
// One server check is deliberately NOT mirrored: `is_safe_name` also asks the
// platform's own path parser for exactly one `Component::Normal`, which catches
// the Windows drive-relative form `C:evil.md` (whose `Prefix::Disk` makes
// `Path::join` replace the base rather than extend it). That check is
// platform-dependent by design — on Unix `:` is an ordinary filename character
// and `C:evil.md` is one normal component — and the browser cannot know which
// OS the daemon runs on. Implementing the Windows reading here would refuse
// names a Linux daemon stores happily, which is the client-stricter-than-server
// bug this whole file exists to avoid. So it is left to the server, which knows.
//
// What this file deliberately does NOT do is scan for prompt injection. Every
// name in a base is replayed into agent context — `file_list` returns the
// filenames and the base name reaches the system prompt as `- **@name** → …` —
// so the server runs the runtime's `injection_guard::scan_message` over both.
// A second, weaker copy in TypeScript would drift from the phrase table it is
// meant to mirror, and the weaker copy is the way in. That refusal therefore
// arrives as a 400 whose message is written to be shown verbatim.

/** Longest base name, in code points. Mirrors `MAX_BASE_NAME_CHARS`. */
export const MAX_BASE_NAME_CHARS = 64;

/** Longest document filename, in code points. Mirrors `MAX_FILENAME_CHARS`. */
export const MAX_FILENAME_CHARS = 128;

/**
 * Mirrors `librefang_types::text::INVISIBLE_FORMAT_CHARS`.
 *
 * `is_control` does not cover these: a bidi override or a zero-width space is
 * category Cf, not Cc, and they are exactly what makes one name render as
 * another — to the operator reading the list and to the model reading it.
 *
 * Grouped as the Rust const is, so the two can be diffed by eye;
 * `knowledgeNames.test.ts` pins this set to that file so they cannot drift.
 */
const INVISIBLE_FORMAT_CHARS = new Set<string>([
  // Zero-width & joiner code points
  "\u00AD", // soft hyphen
  "\u034F", // combining grapheme joiner
  "\u115F", // hangul choseong filler
  "\u1160", // hangul jungseong filler
  "\u17B4", // khmer vowel inherent aq
  "\u17B5", // khmer vowel inherent aa
  "\u180E", // mongolian vowel separator
  "\u200B", // zero-width space
  "\u200C", // zero-width non-joiner
  "\u200D", // zero-width joiner
  "\u2060", // word joiner
  "\u2061", // function application
  "\u2062", // invisible times
  "\u2063", // invisible separator
  "\u2064", // invisible plus
  "\u3164", // hangul filler
  "\uFEFF", // zero-width no-break space / BOM
  "\uFFA0", // halfwidth hangul filler
  // Bidi marks / embeddings / overrides / isolates
  "\u061C", // arabic letter mark
  "\u200E", // left-to-right mark
  "\u200F", // right-to-left mark
  "\u202A", // left-to-right embedding
  "\u202B", // right-to-left embedding
  "\u202C", // pop directional formatting
  "\u202D", // left-to-right override
  "\u202E", // right-to-left override
  "\u2066", // left-to-right isolate
  "\u2067", // right-to-left isolate
  "\u2068", // first strong isolate
  "\u2069", // pop directional isolate
  // Variation selectors (text-injection hiding)
  "\uFE00", // variation selector-1
  "\uFE01", // variation selector-2
  "\uFE02", // variation selector-3
  "\uFE03", // variation selector-4
  "\uFE04", // variation selector-5
  "\uFE05", // variation selector-6
  "\uFE06", // variation selector-7
  "\uFE07", // variation selector-8
  "\uFE08", // variation selector-9
  "\uFE09", // variation selector-10
  "\uFE0A", // variation selector-11
  "\uFE0B", // variation selector-12
  "\uFE0C", // variation selector-13
  "\uFE0D", // variation selector-14
  "\uFE0E", // variation selector-15
  "\uFE0F", // variation selector-16
]);

/**
 * Windows reserved device names, which cannot be used as a filename there even
 * with an extension. Mirrors `WINDOWS_RESERVED_STEMS`; checked against the stem,
 * case-insensitively.
 */
const WINDOWS_RESERVED_STEMS = new Set<string>([
  "con",
  "prn",
  "aux",
  "nul",
  "com1",
  "com2",
  "com3",
  "com4",
  "com5",
  "com6",
  "com7",
  "com8",
  "com9",
  "lpt1",
  "lpt2",
  "lpt3",
  "lpt4",
  "lpt5",
  "lpt6",
  "lpt7",
  "lpt8",
  "lpt9",
]);

function isSafeName(name: string, maxChars: number): boolean {
  // Code points, not UTF-16 units. The server counts `chars()`, so `.length`
  // would charge two against the limit for every astral character and refuse
  // names the server accepts.
  const points = [...name];
  if (points.length === 0 || points.length > maxChars) return false;

  // The leading-dot rule kills `.` and `..` and dotfiles at once. A trailing dot
  // or surrounding whitespace is stripped silently by Windows, so the name
  // stored would not be the name asked for.
  //
  // JS `trim()` and Rust `str::trim` differ on two code points, and neither
  // changes the verdict: U+0085 is trimmed only by Rust but is category Cc, and
  // U+FEFF is trimmed only by JS but is in the invisible set — both are refused
  // below either way.
  if (name.startsWith(".") || name.endsWith(".") || name.trim() !== name) return false;

  for (const point of points) {
    const code = point.codePointAt(0) ?? 0;
    // Rust's `char::is_control()` is exactly the Cc category.
    const isControl = code <= 0x1f || (code >= 0x7f && code <= 0x9f);
    // The tag block, checked as a range rather than through the invisible
    // table, which stops at U+FE0F. U+E0020–U+E007F mirror printable ASCII one
    // for one, render as nothing, and are read by a model as the ASCII they
    // mirror — the smuggling channel a name-based injection would actually use.
    const isTag = code >= 0xe0000 && code <= 0xe007f;
    if (
      isControl ||
      isTag ||
      point === "/" ||
      point === "\\" ||
      INVISIBLE_FORMAT_CHARS.has(point)
    ) {
      return false;
    }
  }

  // `to_ascii_lowercase` on the server, so only A-Z folds: a non-ASCII letter
  // never lowercases into a reserved stem here either.
  const stem = (name.split(".")[0] ?? "").replace(/[A-Z]/g, (c) => c.toLowerCase());
  return !WINDOWS_RESERVED_STEMS.has(stem);
}

/** A base name: a directory, a TOML key, and the `@alias` an agent is told about. */
export function isValidBaseName(name: string): boolean {
  return isSafeName(name, MAX_BASE_NAME_CHARS);
}

/** A document filename: the operator's own file, under the name they gave it. */
export function isValidDocumentName(name: string): boolean {
  return isSafeName(name, MAX_FILENAME_CHARS);
}
