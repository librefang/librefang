import { useEffect, useState, type ComponentProps } from "react";
import type Markdown from "react-markdown";

// react-markdown's plugin lists alias to `PluggableList` from `unified`,
// but `unified` is not a direct dependency. Re-derive the type from the
// public component props so we don't have to add a top-level import.
type PluggableList = NonNullable<ComponentProps<typeof Markdown>["remarkPlugins"]>;

export interface MathPlugins {
  remarkPlugins: PluggableList;
  rehypePlugins: PluggableList;
}

const EMPTY: MathPlugins = { remarkPlugins: [], rehypePlugins: [] };

// Detect math delimiters: `$...$`, `$$...$$`, `\(...\)`, `\[...\]`.
// Cheap regex; false positives (e.g. price text "$5") only cost a one-time
// dynamic import of katex on a chat that wouldn't have rendered math anyway.
const MATH_DELIMITER_RE =
  /\$\$[\s\S]+?\$\$|\$[^\s$][^$\n]*?[^\s$]\$|\\\([\s\S]+?\\\)|\\\[[\s\S]+?\\\]/;

export function containsMathDelimiters(s: string): boolean {
  if (!s) return false;
  return MATH_DELIMITER_RE.test(s);
}

// Cache the resolved plugin pair across components so the second consumer
// doesn't re-trigger dynamic imports / CSS injection.
let cachedPromise: Promise<MathPlugins> | null = null;

async function loadMathPlugins(): Promise<MathPlugins> {
  if (cachedPromise) return cachedPromise;
  cachedPromise = (async () => {
    const [{ default: remarkMath }, { default: rehypeKatex }] = await Promise.all([
      import("remark-math"),
      import("rehype-katex"),
    ]);
    // Side-effect CSS import — bundler emits a separate stylesheet that's
    // injected on demand the first time a math block renders.
    await import("katex/dist/katex.min.css");
    return {
      remarkPlugins: [remarkMath],
      rehypePlugins: [rehypeKatex],
    } satisfies MathPlugins;
  })();
  return cachedPromise;
}

/**
 * Lazily load `remark-math` + `rehype-katex` (+ KaTeX CSS) only when the
 * given content actually contains math delimiters. Returns empty plugin
 * arrays until the dynamic import resolves; consumers can pass the result
 * straight into `<MarkdownContent remarkPlugins={...} rehypePlugins={...}>`
 * without conditional logic.
 *
 * Pulling KaTeX (~280 KB) out of the eager bundle was the main motivation
 * — see #3381.
 */
export function useMathPlugins(content: string): MathPlugins {
  const [plugins, setPlugins] = useState<MathPlugins>(EMPTY);

  useEffect(() => {
    if (!containsMathDelimiters(content)) return;
    let cancelled = false;
    loadMathPlugins().then((p) => {
      if (!cancelled) setPlugins(p);
    });
    return () => {
      cancelled = true;
    };
  }, [content]);

  return plugins;
}
