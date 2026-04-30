import { memo, useMemo, type ComponentProps } from "react";
import Markdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";

// react-markdown's plugin lists alias to `PluggableList` from `unified`,
// but `unified` is not a direct dependency here (only pulled transitively
// via react-markdown). Re-derive the type from the public component
// props instead so we don't have to add a top-level import.
type PluggableList = NonNullable<ComponentProps<typeof Markdown>["remarkPlugins"]>;

// `Components` provides contextual typing for each entry — props (incl.
// `children`, which is optional in react-markdown's HTML element types)
// are inferred per HTML tag. Don't add explicit param annotations here:
// they re-narrow `children` to required and break the assignability.
const defaultComponents: Components = {
  p: ({ children }) => <p className="mb-1.5 last:mb-0">{children}</p>,
  h1: ({ children }) => <h1 className="text-sm font-bold mb-1.5">{children}</h1>,
  h2: ({ children }) => <h2 className="text-xs font-bold mb-1">{children}</h2>,
  h3: ({ children }) => <h3 className="text-xs font-bold mb-1">{children}</h3>,
  ul: ({ children }) => <ul className="list-disc pl-4 mb-1.5 space-y-0.5">{children}</ul>,
  ol: ({ children }) => <ol className="list-decimal pl-4 mb-1.5 space-y-0.5">{children}</ol>,
  li: ({ children }) => <li className="text-xs">{children}</li>,
  code: ({ node, children, ...props }) => {
    const isBlock = node?.position?.start?.line !== node?.position?.end?.line || String(children).includes("\n");
    return isBlock
      ? <pre className="p-2 rounded-lg bg-main font-mono text-[11px] overflow-x-auto mb-1.5"><code>{children}</code></pre>
      : <code className="px-1 py-0.5 rounded bg-main font-mono text-[11px]" {...props}>{children}</code>;
  },
  pre: ({ children }) => <>{children}</>,
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
};

// Stable default plugin array — never changes between renders
const defaultPlugins: PluggableList = [remarkGfm];

interface MarkdownContentProps {
  children: string;
  className?: string;
  remarkPlugins?: PluggableList;
  rehypePlugins?: PluggableList;
  components?: Components;
}

export const MarkdownContent = memo(function MarkdownContent({
  children,
  className,
  remarkPlugins,
  rehypePlugins,
  components,
}: MarkdownContentProps) {
  const merged = useMemo(
    () => components ? { ...defaultComponents, ...components } : defaultComponents,
    [components],
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
      >
        {children}
      </Markdown>
    </div>
  );
});
