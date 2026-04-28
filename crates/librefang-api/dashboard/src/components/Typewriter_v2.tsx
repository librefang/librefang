import { useState, useEffect, useMemo } from 'react';
import { animate } from 'motion/react';
import Markdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import remarkMath from 'remark-math';
import rehypeKatex from 'rehype-katex';
import 'katex/dist/katex.min.css';

const mdComponents = {
  p: ({ children }: any) => <p className="mb-2 last:mb-0">{children}</p>,
  h1: ({ children }: any) => <h1 className="text-lg font-bold mb-2">{children}</h1>,
  h2: ({ children }: any) => <h2 className="text-base font-bold mb-1.5">{children}</h2>,
  h3: ({ children }: any) => <h3 className="text-sm font-bold mb-1">{children}</h3>,
  ul: ({ children }: any) => <ul className="list-disc pl-4 mb-2 space-y-0.5">{children}</ul>,
  ol: ({ children }: any) => <ol className="list-decimal pl-4 mb-2 space-y-0.5">{children}</ol>,
  li: ({ children }: any) => <li className="text-sm">{children}</li>,
  code: ({ node, children, ...props }: any) => {
    const isBlock = node?.position?.start?.line !== node?.position?.end?.line || String(children).includes("\n");
    return isBlock
      ? <pre className="p-2 rounded-lg bg-main font-mono text-[11px] overflow-x-auto mb-2"><code>{children}</code></pre>
      : <code className="px-1 py-0.5 rounded bg-main font-mono text-[11px]" {...props}>{children}</code>;
  },
  pre: ({ children }: any) => <>{children}</>,
  table: ({ children }: any) => <table className="w-full text-xs border-collapse mb-2">{children}</table>,
  th: ({ children }: any) => <th className="border border-border-subtle px-2 py-1 bg-main font-bold text-left">{children}</th>,
  td: ({ children }: any) => <td className="border border-border-subtle px-2 py-1">{children}</td>,
  blockquote: ({ children }: any) => <blockquote className="border-l-2 border-primary pl-3 italic text-text-dim mb-2">{children}</blockquote>,
  strong: ({ children }: any) => <strong className="font-bold">{children}</strong>,
  a: ({ href, children }: any) => <a href={href} className="text-primary underline" target="_blank" rel="noopener noreferrer">{children}</a>,
};

/// Streams `text` character-by-character into the markdown output to
/// give an LLM-style "typing" effect. The reveal is driven by motion's
/// `animate()` (instead of a hand-rolled RAF) so it joins the same
/// animation scheduler as the rest of the dashboard and respects
/// `prefers-reduced-motion`.
///
/// `speed` is milliseconds-per-character (kept from the legacy API).
/// When the source text shrinks below the already-displayed length
/// (e.g. the upstream message restarted), the typewriter rewinds to 0.
export function Typewriter_v2({ text, speed = 20 }: { text: string; speed?: number }) {
  const [displayed, setDisplayed] = useState("");

  useEffect(() => {
    // If upstream restarted the stream (new text shorter than what we already
    // typed out), rewind. Otherwise resume from where we are.
    const start = displayed.length > text.length ? 0 : displayed.length;
    if (start === 0 && displayed !== "") setDisplayed("");
    const remaining = text.length - start;
    if (remaining <= 0) return;
    const controls = animate(start, text.length, {
      duration: (remaining * speed) / 1000,
      ease: "linear",
      onUpdate: (latest) => {
        const idx = Math.min(Math.floor(latest), text.length);
        setDisplayed(text.slice(0, idx));
      },
    });
    return () => controls.stop();
    // displayed intentionally excluded — re-running on every char update
    // would restart the animation each frame. Only react to source changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [text, speed]);

  return useMemo(() => (
    <Markdown
      remarkPlugins={[remarkGfm, remarkMath]}
      rehypePlugins={[rehypeKatex]}
      components={mdComponents}
    >
      {displayed}
    </Markdown>
  ), [displayed]);
}
