import { useState, useEffect, useMemo, useRef } from 'react';
import { flushSync } from 'react-dom';
import { animate } from 'motion/react';
import Markdown, { type Components } from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { useMathPlugins } from '../lib/hooks/useMathPlugins';

const mdComponents: Components = {
  p: ({ children }) => <p className="mb-2 last:mb-0">{children}</p>,
  h1: ({ children }) => <h1 className="text-lg font-bold mb-2">{children}</h1>,
  h2: ({ children }) => <h2 className="text-base font-bold mb-1.5">{children}</h2>,
  h3: ({ children }) => <h3 className="text-sm font-bold mb-1">{children}</h3>,
  ul: ({ children }) => <ul className="list-disc pl-4 mb-2 space-y-0.5">{children}</ul>,
  ol: ({ children }) => <ol className="list-decimal pl-4 mb-2 space-y-0.5">{children}</ol>,
  li: ({ children }) => <li className="text-sm">{children}</li>,
  code: ({ node, children, ...props }) => {
    const isBlock = node?.position?.start?.line !== node?.position?.end?.line || String(children).includes("\n");
    return isBlock
      ? <pre className="p-2 rounded-lg bg-main font-mono text-[11px] overflow-x-auto mb-2"><code>{children}</code></pre>
      : <code className="px-1 py-0.5 rounded bg-main font-mono text-[11px]" {...props}>{children}</code>;
  },
  pre: ({ children }) => <>{children}</>,
  table: ({ children }) => <table className="w-full text-xs border-collapse mb-2">{children}</table>,
  th: ({ children }) => <th className="border border-border-subtle px-2 py-1 bg-main font-bold text-left">{children}</th>,
  td: ({ children }) => <td className="border border-border-subtle px-2 py-1">{children}</td>,
  blockquote: ({ children }) => <blockquote className="border-l-2 border-brand pl-3 italic text-text-dim mb-2">{children}</blockquote>,
  strong: ({ children }) => <strong className="font-bold">{children}</strong>,
  a: ({ href, children }) => <a href={href} className="text-brand underline" target="_blank" rel="noopener noreferrer">{children}</a>,
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
  const lastIdxRef = useRef(0);
  const { remarkPlugins: mathRemark, rehypePlugins: mathRehype } = useMathPlugins(text);

  useEffect(() => {
    const needsReset = lastIdxRef.current > text.length;
    if (needsReset) {
      flushSync(() => setDisplayed(""));
      lastIdxRef.current = 0;
    }

    const start = lastIdxRef.current;
    const remaining = text.length - start;
    if (remaining <= 0) return;
    const controls = animate(start, text.length, {
      duration: (remaining * speed) / 1000,
      ease: "linear",
      onUpdate: (latest) => {
        const idx = Math.min(Math.floor(latest), text.length);
        if (idx !== lastIdxRef.current) {
          lastIdxRef.current = idx;
          setDisplayed(text.slice(0, idx));
        }
      },
    });
    return () => controls.stop();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [text, speed]);

  const remarkPlugins = useMemo(
    () => [remarkGfm, ...mathRemark],
    [mathRemark],
  );

  const markdown = useMemo(() => (
    <Markdown
      remarkPlugins={remarkPlugins}
      rehypePlugins={mathRehype}
      components={mdComponents}
    >
      {displayed}
    </Markdown>
  ), [displayed, remarkPlugins, mathRehype]);

  return (
    <div aria-live="polite" aria-atomic="false">
      {markdown}
    </div>
  );
}
