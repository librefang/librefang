import { memo, useMemo, type ComponentProps } from "react";
import Markdown, { defaultUrlTransform, type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { MermaidDiagram } from "./MermaidDiagram";

// remark tags a fenced block's language as `language-<id>` on the <code>.
// Matched as a whole class token so `language-mermaidish` is not mistaken for
// it, and case-insensitively because agents write ```Mermaid.
const MERMAID_FENCE = /(?:^|\s)language-mermaid(?:\s|$)/i;

function isMermaidFence(className?: string): boolean {
  return !!className && MERMAID_FENCE.test(className);
}

// Extend react-markdown's URL allowlist with deep-link schemes for native
// apps the agent commonly references. Schemes outside the default allowlist
// (http/https/mailto/tel/...) are stripped by `defaultUrlTransform`, which
// would silently break `[note](obsidian://...)` links agents emit when
// pointing at vault entries.
const OBSIDIAN_URL =
  /^obsidian(?:-advanced-uri)?:[A-Za-z0-9._~%!$&'()*+,;=:@/?#-]*$/i;

// Exported so other markdown renderers (e.g. Typewriter_v2, which renders
// the streaming view) share one URL policy — otherwise react-markdown's
// `defaultUrlTransform` runs implicitly there and strips the obsidian
// schemes this allows, so a link would render differently while streaming
// than in the settled MarkdownContent view.
export const urlTransform = (url: string): string => {
  if (OBSIDIAN_URL.test(url)) {
    return url;
  }
  return defaultUrlTransform(url);
};

// react-markdown's plugin lists alias to `PluggableList` from `unified`,
// but `unified` is not a direct dependency here (only pulled transitively
// via react-markdown). Re-derive the type from the public component
// props instead so we don't have to add a top-level import.
type PluggableList = NonNullable<ComponentProps<typeof Markdown>["remarkPlugins"]>;

function buildComponents(overrides?: Components, diagrams = false): Components {
  const { pre: customPre, ...restOverrides } = overrides ?? {};
  const passThroughPre: Components["pre"] = ({ children }) => <>{children}</>;

  return {
    p: ({ children }) => <p className="mb-1.5 last:mb-0">{children}</p>,
    h1: ({ children }) => <h1 className="text-sm font-bold mb-1.5">{children}</h1>,
    h2: ({ children }) => <h2 className="text-xs font-bold mb-1">{children}</h2>,
    h3: ({ children }) => <h3 className="text-xs font-bold mb-1">{children}</h3>,
    ul: ({ children }) => <ul className="list-disc pl-4 mb-1.5 space-y-0.5">{children}</ul>,
    ol: ({ children }) => <ol className="list-decimal pl-4 mb-1.5 space-y-0.5">{children}</ol>,
    li: ({ children }) => <li className="text-xs">{children}</li>,
    code: ({ node, children, className: codeClassName, ...props }) => {
      const isBlock =
        node?.position
          ? node.position.start.line !== node.position.end.line
          : typeof children === "string" && children.includes("\n");
      // A ```mermaid fence renders as a diagram. Checked before the generic
      // block branch so a caller's `pre` override cannot swallow it, and
      // before the inline branch because `language-*` only ever appears on a
      // fenced block. `MermaidDiagram` falls back to this same `<pre>` shape
      // when the source does not parse.
      if (diagrams && isMermaidFence(codeClassName) && typeof children === "string") {
        return <MermaidDiagram source={children.replace(/\n$/, "")} />;
      }
      if (isBlock) {
        if (customPre) {
          const Pre = customPre;
          return <Pre><code className={codeClassName} {...props}>{children}</code></Pre>;
        }
        return (
          <pre className="p-2 rounded-lg bg-main font-mono text-[11px] overflow-x-auto mb-1.5">
            <code>{children}</code>
          </pre>
        );
      }
      return (
        <code className={`px-1 py-0.5 rounded bg-main font-mono text-[11px] ${codeClassName ?? ""}`} {...props}>
          {children}
        </code>
      );
    },
    pre: passThroughPre,
    table: ({ children }) => (
      <div className="overflow-x-auto mb-1.5">
        <table className="w-full text-xs border-collapse">{children}</table>
      </div>
    ),
    th: ({ children }) => <th className="border border-border-subtle px-2 py-1 bg-main font-bold text-left">{children}</th>,
    td: ({ children }) => <td className="border border-border-subtle px-2 py-1">{children}</td>,
    blockquote: ({ children }) => <blockquote className="border-l-2 border-brand pl-3 italic text-text-dim mb-1.5">{children}</blockquote>,
    strong: ({ children }) => <strong className="font-bold">{children}</strong>,
    a: ({ href, children }) => <a href={href} className="text-brand underline" target="_blank" rel="noopener noreferrer">{children}</a>,
    ...restOverrides,
  };
}

// Stable default plugin array — never changes between renders
const defaultPlugins: PluggableList = [remarkGfm];

interface MarkdownContentProps {
  children: string;
  className?: string;
  remarkPlugins?: PluggableList;
  rehypePlugins?: PluggableList;
  components?: Components;
  /**
   * Render a ```mermaid fence as a diagram instead of a code block.
   *
   * Opt-in, because this renderer is shared by surfaces a diagram has no
   * business in: a memory record's 4rem preview box, an agent card, and the
   * chat's thinking panel — which grows token by token while a turn streams,
   * so a fence there would re-parse and re-layout on every frame.
   */
  diagrams?: boolean;
}

export const MarkdownContent = memo(function MarkdownContent({
  children,
  className,
  remarkPlugins,
  rehypePlugins,
  components,
  diagrams,
}: MarkdownContentProps) {
  const merged = useMemo(
    () => buildComponents(components, diagrams),
    [components, diagrams],
  );
  const plugins = useMemo(
    () => remarkPlugins ? [remarkGfm, ...remarkPlugins] : defaultPlugins,
    [remarkPlugins],
  );

  return (
    <div className={className}>
      <Markdown
        remarkPlugins={plugins}
        rehypePlugins={rehypePlugins}
        components={merged}
        urlTransform={urlTransform}
      >
        {children}
      </Markdown>
    </div>
  );
});
