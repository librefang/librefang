// Lazy KaTeX loader for chat / typewriter markdown rendering.
//
// Issue #3381: pulling `rehype-katex`, `remark-math`, and
// `katex/dist/katex.min.css` at module level forced the entire ~280KB
// KaTeX payload (JS + CSS + fonts) into the dashboard's cold-start
// bundle even though the vast majority of LLM messages contain no math.
// This module exposes:
//
//   - `containsMathDelimiters(s)` — fast string check matching the
//     same delimiter set as the `remark-math` defaults: `$...$`,
//     `$$...$$`, and `\(...\)` / `\[...\]`.
//   - `useKatexPlugins(text)` — React hook that returns plugin arrays
//     suitable for `react-markdown`'s `remarkPlugins` / `rehypePlugins`
//     props, dynamically importing the KaTeX-related modules (and CSS)
//     ONLY after the first message body containing math is observed.
//     Until then the hook returns `undefined` so callers fall back to
//     plain-markdown rendering with no extra cost.
//
// Once loaded, the plugin arrays are cached module-level so subsequent
// math-bearing messages render synchronously without re-importing.

import { useEffect, useMemo, useState, type ComponentProps } from "react";
import type Markdown from "react-markdown";

type PluggableList = NonNullable<ComponentProps<typeof Markdown>["remarkPlugins"]>;

// Match `$...$`, `$$...$$`, `\(...\)`, `\[...\]` — the delimiter set
// `remark-math` recognizes by default. Liberal on purpose: we'd rather
// load KaTeX one extra time than silently skip a math block.
const MATH_DELIMITER_RE = /\$\$[\s\S]+?\$\$|\$[^\s$][^$]*\$|\\\([\s\S]+?\\\)|\\\[[\s\S]+?\\\]/;

/** True if the given text plausibly contains math the markdown
 * pipeline would otherwise drop on the floor. */
export function containsMathDelimiters(text: string | null | undefined): boolean {
  if (!text) return false;
  return MATH_DELIMITER_RE.test(text);
}

let katexBundle: { remark: PluggableList; rehype: PluggableList } | null = null;
let katexPromise: Promise<{ remark: PluggableList; rehype: PluggableList }> | null = null;

function loadKatexPlugins(): Promise<{ remark: PluggableList; rehype: PluggableList }> {
  if (katexPromise) return katexPromise;
  katexPromise = Promise.all([
    import("remark-math"),
    import("rehype-katex"),
    // CSS side-effect import — Vite folds this into the `katex` chunk
    // (see vite.config.ts manualChunks) so the JS + CSS arrive together.
    import("katex/dist/katex.min.css"),
  ]).then(([remarkMathMod, rehypeKatexMod]) => {
    const bundle = {
      remark: [remarkMathMod.default] as PluggableList,
      rehype: [rehypeKatexMod.default] as PluggableList,
    };
    katexBundle = bundle;
    return bundle;
  }).catch((err) => {
    // Reset on failure so the next math-bearing message can retry
    // (e.g. transient chunk-load error after a deploy).
    katexPromise = null;
    throw err;
  });
  return katexPromise;
}

interface KatexPlugins {
  remarkPlugins?: PluggableList;
  rehypePlugins?: PluggableList;
}

/** React hook: returns KaTeX plugin arrays once `text` first contains
 * math delimiters. Returns an empty object until then so callers can
 * spread it directly into a `<Markdown {...plugins}>` element. */
export function useKatexPlugins(text: string | null | undefined): KatexPlugins {
  const hasMath = useMemo(() => containsMathDelimiters(text), [text]);
  // `katexBundle` may already be populated from a previous message;
  // mirror its presence into render state so we re-render once the
  // import resolves.
  const [loaded, setLoaded] = useState<typeof katexBundle>(katexBundle);

  useEffect(() => {
    if (!hasMath) return;
    if (katexBundle) {
      if (loaded !== katexBundle) setLoaded(katexBundle);
      return;
    }
    let cancelled = false;
    loadKatexPlugins().then((bundle) => {
      if (!cancelled) setLoaded(bundle);
    }).catch(() => {
      // Already logged by the loader; fall back to plain markdown.
    });
    return () => { cancelled = true; };
  }, [hasMath, loaded]);

  if (!hasMath || !loaded) return {};
  return { remarkPlugins: loaded.remark, rehypePlugins: loaded.rehype };
}
