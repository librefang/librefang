import type { ReactNode, ComponentType } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
type PluggableList = any[];

const defaultComponents: Record<string, ComponentType<any>> = {
  p: ({ children }: { children: ReactNode }) => <p className="mb-1.5 last:mb-0">{children}</p>,
  h1: ({ children }: { children: ReactNode }) => <h1 className="text-sm font-bold mb-1.5">{children}</h1>,
  h2: ({ children }: { children: ReactNode }) => <h2 className="text-xs font-bold mb-1">{children}</h2>,
  h3: ({ children }: { children: ReactNode }) => <h3 className="text-xs font-bold mb-1">{children}</h3>,
  ul: ({ children }: { children: ReactNode }) => <ul className="list-disc pl-4 mb-1.5 space-y-0.5">{children}</ul>,
  ol: ({ children }: { children: ReactNode }) => <ol className="list-decimal pl-4 mb-1.5 space-y-0.5">{children}</ol>,
  li: ({ children }: { children: ReactNode }) => <li className="text-xs">{children}</li>,
  code: ({ node, children, ...props }: any) => {
    const isBlock = node?.position?.start?.line !== node?.position?.end?.line || String(children).includes("\n");
    return isBlock
      ? <pre className="p-2 rounded-lg bg-main font-mono text-[11px] overflow-x-auto mb-1.5"><code>{children}</code></pre>
      : <code className="px-1 py-0.5 rounded bg-main font-mono text-[11px]" {...props}>{children}</code>;
  },
  pre: ({ children }: { children: ReactNode }) => <>{children}</>,
  table: ({ children }: { children: ReactNode }) => <table className="w-full text-xs border-collapse mb-1.5">{children}</table>,
  th: ({ children }: { children: ReactNode }) => <th className="border border-border-subtle px-2 py-1 bg-main font-bold text-left">{children}</th>,
  td: ({ children }: { children: ReactNode }) => <td className="border border-border-subtle px-2 py-1">{children}</td>,
  blockquote: ({ children }: { children: ReactNode }) => <blockquote className="border-l-2 border-brand pl-3 italic text-text-dim mb-1.5">{children}</blockquote>,
  strong: ({ children }: { children: ReactNode }) => <strong className="font-bold">{children}</strong>,
  a: ({ href, children }: { href?: string; children: ReactNode }) => <a href={href} className="text-brand underline" target="_blank" rel="noopener noreferrer">{children}</a>,
};

interface MarkdownContentProps {
  children: string;
  className?: string;
  remarkPlugins?: PluggableList;
  rehypePlugins?: PluggableList;
  components?: Record<string, ComponentType<any>>;
}

export function MarkdownContent({
  children,
  className,
  remarkPlugins,
  rehypePlugins,
  components,
}: MarkdownContentProps) {
  const merged = components ? { ...defaultComponents, ...components } : defaultComponents;
  const plugins: PluggableList = remarkPlugins ? [remarkGfm, ...remarkPlugins] : [remarkGfm];

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
}
