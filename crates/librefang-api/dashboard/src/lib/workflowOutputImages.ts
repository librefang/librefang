// Extracts image references from a workflow step's free-form output string.
//
// Step outputs are plain strings (see WorkflowStepResult.output in src/api.ts
// and StepResult in crates/librefang-kernel/src/workflow.rs). When a step
// calls the `image_generate` tool the agent typically includes the tool's
// JSON result (or its `image_urls` array) verbatim in its reply. We scan the
// text for:
//
//   * Embedded JSON objects/arrays containing image shapes:
//       { url: "https://.../foo.png" }
//       { url: "data:image/png;base64,..." }
//       { image_urls: ["/api/uploads/<id>", ...] }
//       { images: [ { url, data_base64 } ] }
//       { artifact: "/api/media/artifacts/<id>" } | { artifact_id: "<id>" }
//
//   * Bare URLs in prose: `https://example.com/foo.png`,
//     `/api/uploads/<id>`, `/api/media/artifacts/<id>`, `data:image/...`.
//
// Only safe URL schemes are returned. `javascript:`, `file:`, `about:`,
// `vbscript:`, `data:` with a non-image MIME, and protocol-relative
// (`//evil.com/foo.png`) refs are rejected — those would otherwise render
// as `<img src>` and either no-op or, worse, leak credentials. The fallback
// is to display the raw string; nothing is dropped silently.

export type ImageRefKind = "url" | "data-uri" | "artifact";

export interface ImageRef {
  kind: ImageRefKind;
  src: string;
  alt?: string;
}

const IMAGE_EXT_RE = /\.(png|jpe?g|gif|webp|svg|avif|bmp|ico)(\?[^\s"'<>)]*)?$/i;
const ARTIFACT_PATH_RE = /^\/api\/media\/artifacts\/[A-Za-z0-9_-]+/;
const UPLOAD_PATH_RE = /^\/api\/uploads\/[A-Za-z0-9_.-]+/;
const DATA_IMAGE_RE = /^data:image\/[a-z0-9.+-]+;[^,]*,/i;

const HTTP_URL_RE = /https?:\/\/[^\s"'<>)]+/gi;
const DATA_URI_RE = /data:image\/[a-z0-9.+-]+;[^,]*,[A-Za-z0-9+/=_-]+/gi;
const APIPATH_RE = /\/api\/(?:uploads|media\/artifacts)\/[A-Za-z0-9_.-]+/gi;

/**
 * Validate a string is a safe image source we'll render as `<img src>`.
 * Returns the matching ImageRef or `null` if the value is not a safe image.
 */
export function classifyImageRef(raw: string, alt?: string): ImageRef | null {
  if (typeof raw !== "string") return null;
  const s = raw.trim();
  if (!s) return null;

  // Reject protocol-relative URLs — even if the path ends in .png, the
  // browser resolves the host from the current origin, which is a vector
  // for hijacking once the dashboard is served from a custom domain.
  if (s.startsWith("//")) return null;

  if (DATA_IMAGE_RE.test(s)) {
    return { kind: "data-uri", src: s, alt };
  }

  // Reject any other data:/javascript:/file:/about:/vbscript: scheme.
  const lower = s.toLowerCase();
  if (
    lower.startsWith("javascript:") ||
    lower.startsWith("file:") ||
    lower.startsWith("about:") ||
    lower.startsWith("vbscript:") ||
    lower.startsWith("data:")
  ) {
    return null;
  }

  if (s.startsWith("http://") || s.startsWith("https://")) {
    // Strip query/fragment when checking extension.
    const path = s.split(/[?#]/, 1)[0];
    if (IMAGE_EXT_RE.test(path)) return { kind: "url", src: s, alt };
    // http(s) without image extension is not assumed to be an image —
    // could be an HTML page link.
    return null;
  }

  if (ARTIFACT_PATH_RE.test(s)) return { kind: "artifact", src: s, alt };
  if (UPLOAD_PATH_RE.test(s)) {
    // Uploads dir holds generated images (and other media); without an
    // extension we can't be 100% sure, but the image_generate tool
    // writes here and we treat it as image. Caller renders <img>;
    // browser falls back to broken-image icon if MIME is wrong, which
    // is acceptable.
    return { kind: "url", src: s, alt };
  }

  return null;
}

interface CandidateNode {
  value: unknown;
  alt?: string;
}

/**
 * Walk a parsed JSON value collecting image-shaped fields. Recognizes:
 *   - { url: "..." }
 *   - { artifact: "..." } / { artifact_id: "..." }
 *   - { data_base64: "...", mime?: "image/png" }
 *   - { image_urls: [...] }
 *   - { images: [ ... ] }
 *   - Arrays of any of the above.
 */
function harvestFromJson(root: unknown, out: ImageRef[], seen: Set<string>): void {
  // FIFO queue so iteration order matches document order — galleries
  // should render images in the same sequence the agent produced them.
  const queue: CandidateNode[] = [{ value: root }];
  let head = 0;

  while (head < queue.length) {
    const { value, alt } = queue[head++];

    if (value == null) continue;

    if (typeof value === "string") {
      const ref = classifyImageRef(value, alt);
      if (ref && !seen.has(ref.src)) {
        seen.add(ref.src);
        out.push(ref);
      }
      continue;
    }

    if (Array.isArray(value)) {
      for (const item of value) queue.push({ value: item, alt });
      continue;
    }

    if (typeof value === "object") {
      const obj = value as Record<string, unknown>;

      // Common alt-text fields produced by image tools.
      const altText =
        (typeof obj.alt === "string" && obj.alt) ||
        (typeof obj.revised_prompt === "string" && obj.revised_prompt) ||
        (typeof obj.prompt === "string" && obj.prompt) ||
        alt;

      // Image-shaped scalar fields, checked in priority order.
      for (const key of ["url", "image_url", "src", "file_url"]) {
        const v = obj[key];
        if (typeof v === "string") {
          const ref = classifyImageRef(v, altText);
          if (ref && !seen.has(ref.src)) {
            seen.add(ref.src);
            out.push(ref);
          }
        }
      }

      // Artifact references.
      for (const key of ["artifact", "artifact_path"]) {
        const v = obj[key];
        if (typeof v === "string") {
          const ref = classifyImageRef(v, altText);
          if (ref && !seen.has(ref.src)) {
            seen.add(ref.src);
            out.push(ref);
          }
        }
      }
      if (typeof obj.artifact_id === "string" && obj.artifact_id) {
        const src = `/api/media/artifacts/${obj.artifact_id}`;
        const ref = classifyImageRef(src, altText);
        if (ref && !seen.has(ref.src)) {
          seen.add(ref.src);
          out.push(ref);
        }
      }

      // base64 blob directly on the object — synthesize a data URI.
      if (typeof obj.data_base64 === "string" && obj.data_base64) {
        const mime = typeof obj.mime === "string" && obj.mime.startsWith("image/")
          ? obj.mime
          : "image/png";
        const src = `data:${mime};base64,${obj.data_base64}`;
        if (!seen.has(src)) {
          seen.add(src);
          out.push({ kind: "data-uri", src, alt: altText });
        }
      }

      // Recurse into known container fields and any unknown nested values.
      for (const v of Object.values(obj)) {
        if (v !== null && (typeof v === "object")) {
          queue.push({ value: v, alt: altText });
        }
      }
    }
  }
}

/**
 * Find every parseable JSON object/array embedded in `text` and feed
 * each one through `harvestFromJson`. We scan for balanced `{...}` and
 * `[...]` regions — bracket counting is enough for the well-formed JSON
 * blocks that `serde_json::to_string_pretty` emits in tool results.
 */
function harvestFromText(text: string, out: ImageRef[], seen: Set<string>): void {
  const len = text.length;
  let i = 0;
  while (i < len) {
    const ch = text[i];
    if (ch === "{" || ch === "[") {
      const end = scanBalanced(text, i);
      if (end > i) {
        const slice = text.slice(i, end + 1);
        try {
          const parsed: unknown = JSON.parse(slice);
          harvestFromJson(parsed, out, seen);
        } catch {
          // Not valid JSON — keep going.
        }
        i = end + 1;
        continue;
      }
    }
    i += 1;
  }
}

/** Returns the index of the matching closing bracket, or -1 if unbalanced. */
function scanBalanced(s: string, start: number): number {
  const open = s[start];
  const close = open === "{" ? "}" : open === "[" ? "]" : "";
  if (!close) return -1;
  let depth = 0;
  let inStr = false;
  let escape = false;
  for (let i = start; i < s.length; i += 1) {
    const c = s[i];
    if (inStr) {
      if (escape) {
        escape = false;
      } else if (c === "\\") {
        escape = true;
      } else if (c === '"') {
        inStr = false;
      }
      continue;
    }
    if (c === '"') {
      inStr = true;
      continue;
    }
    if (c === open) depth += 1;
    else if (c === close) {
      depth -= 1;
      if (depth === 0) return i;
    }
  }
  return -1;
}

/** Fallback: pull bare image-shaped URLs out of prose. */
function harvestBareUrls(text: string, out: ImageRef[], seen: Set<string>): void {
  const collect = (re: RegExp) => {
    let m: RegExpExecArray | null;
    // Reset lastIndex to be safe — these regexes are module-scope.
    re.lastIndex = 0;
    while ((m = re.exec(text)) !== null) {
      // Trim trailing punctuation that often glues onto URLs in prose.
      const raw = m[0].replace(/[)\].,;:!?"']+$/, "");
      const ref = classifyImageRef(raw);
      if (ref && !seen.has(ref.src)) {
        seen.add(ref.src);
        out.push(ref);
      }
    }
  };
  collect(DATA_URI_RE);
  collect(HTTP_URL_RE);
  collect(APIPATH_RE);
}

/**
 * Extract every image reference from a workflow step's output. Returns
 * an empty array when nothing image-shaped is found — callers fall back
 * to plain-text rendering.
 *
 * Accepts `unknown` so callers don't need to narrow first; non-strings
 * are treated as empty.
 */
export function extractImageRefs(output: unknown): ImageRef[] {
  if (output == null) return [];

  // If the caller hands us a parsed value directly, walk it.
  if (typeof output !== "string") {
    const out: ImageRef[] = [];
    const seen = new Set<string>();
    harvestFromJson(output, out, seen);
    return out;
  }

  const text = output;
  if (!text) return [];

  const out: ImageRef[] = [];
  const seen = new Set<string>();

  // 1. Try parsing the whole string as JSON first — the cleanest case
  //    (a step that returns nothing but the tool's JSON result).
  const trimmed = text.trim();
  if (trimmed.startsWith("{") || trimmed.startsWith("[")) {
    try {
      const parsed: unknown = JSON.parse(trimmed);
      harvestFromJson(parsed, out, seen);
    } catch {
      // Fall through to embedded-block scan.
    }
  }

  // 2. Scan for embedded JSON blocks (agent prose + a tool result blob).
  harvestFromText(text, out, seen);

  // 3. Bare URLs in prose (no surrounding JSON).
  harvestBareUrls(text, out, seen);

  return out;
}
