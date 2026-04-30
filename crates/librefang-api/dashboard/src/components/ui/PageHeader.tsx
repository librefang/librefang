import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { RefreshCw, HelpCircle } from "lucide-react";
import type { Components } from "react-markdown";
import { Modal } from "./Modal";
import { MarkdownContent } from "./MarkdownContent";

/// The help text in i18n is plain text using `\n\n` as paragraph separators,
/// `· ` as bullet markers, `1. 2. 3.` for numbered steps, and a trailing
/// colon on a stand-alone line as a section heading. Translate that
/// convention into actual Markdown so the renderer below can produce
/// proper `<h2>` / `<ul>` / `<ol>` instead of one wall of `<p>`.
function toHelpMarkdown(s: string): string {
  return (
    s
      // `· Foo` (any indent) → standard markdown unordered list item.
      .replace(/^(\s*)·\s+/gm, "$1- ")
      // `Section heading:` on its own line (no inner colon, no leading
      // bullet / number / hash) → `## Section heading`. Recognises both
      // ASCII `:` and the Chinese full-width colon `：`. The trailing
      // colon is dropped — the heading visual replaces it.
      .replace(/^([^\s:：0-9·\-#][^:：\n]*)[:：]\s*$/gm, "## $1")
      // Bare http(s) URL → `[url](url)` so it renders as a clickable
      // link. Skips URLs already wrapped in markdown link syntax (`[..](..)`)
      // by checking the preceding char isn't `[` or `(`. Stops at
      // whitespace, ASCII `)` / `(`, Chinese `）` / `（`, quotes, `<`, `>`,
      // and trailing punctuation like `,` / `.` / `;` / `:` so a sentence-
      // ending dot or full-width close-paren after the URL doesn't get
      // swallowed into the link.
      .replace(
        /(?<![[(])\bhttps?:\/\/[^\s()（）<>"'`,。，；：]+/g,
        (m) => `[${m}](${m})`,
      )
  );
}

// See MarkdownContent.tsx for why explicit `{ children: ReactNode }`
// annotations are dropped — `Components` provides contextual typing
// where `children` is optional, matching react-markdown's contract.
const HELP_MARKDOWN_COMPONENTS: Components = {
  p: ({ children }) => (
    <p className="text-sm text-text-dim leading-relaxed mb-4 last:mb-0">{children}</p>
  ),
  h1: ({ children }) => (
    <h1 className="text-base font-black tracking-tight text-text-main mb-3 mt-6 first:mt-0">
      {children}
    </h1>
  ),
  h2: ({ children }) => (
    <h2 className="text-[11px] font-black uppercase tracking-widest text-brand mb-3 mt-6 first:mt-0">
      {children}
    </h2>
  ),
  h3: ({ children }) => (
    <h3 className="text-sm font-bold text-text-main mb-2 mt-4">{children}</h3>
  ),
  ul: ({ children }) => (
    <ul className="list-disc pl-5 marker:text-brand/60 space-y-1.5 mb-4 text-sm text-text-dim leading-relaxed">
      {children}
    </ul>
  ),
  ol: ({ children }) => (
    <ol className="list-decimal pl-5 marker:text-brand marker:font-bold space-y-2 mb-4 text-sm text-text-dim leading-relaxed">
      {children}
    </ol>
  ),
  li: ({ children }) => (
    <li className="pl-1">{children}</li>
  ),
  code: ({ node, children, ...props }) => {
    const isBlock =
      node?.position?.start?.line !== node?.position?.end?.line ||
      String(children).includes("\n");
    return isBlock ? (
      <pre className="p-3 rounded-xl bg-main/60 border border-border-subtle/50 font-mono text-[12px] text-text-main overflow-x-auto mb-4">
        <code>{children}</code>
      </pre>
    ) : (
      <code
        className="px-1.5 py-0.5 rounded bg-main/60 border border-border-subtle/50 font-mono text-[12px] text-text-main"
        {...props}
      >
        {children}
      </code>
    );
  },
  strong: ({ children }) => (
    <strong className="font-bold text-text-main">{children}</strong>
  ),
  a: ({ href, children }) => (
    <a
      href={href}
      className="text-brand underline decoration-brand/40 hover:decoration-brand transition-colors"
      target="_blank"
      rel="noopener noreferrer"
    >
      {children}
    </a>
  ),
};

interface PageHeaderProps {
  icon: ReactNode;
  title: string;
  subtitle?: string;
  /** Optional small label rendered next to the title (e.g. "Beta", count, or a <Badge/> element). */
  badge?: ReactNode;
  actions?: ReactNode;
  isFetching?: boolean;
  onRefresh?: () => void;
  helpText?: string;
}

export function PageHeader({ icon, title, subtitle, badge, actions, isFetching, onRefresh, helpText }: PageHeaderProps) {
  const { t } = useTranslation();
  const [showHelp, setShowHelp] = useState(false);

  return (
    <>
      <header className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2 min-w-0">
          <div className="p-1.5 rounded-lg bg-primary/10 text-primary shrink-0">{icon}</div>
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <h1 className="text-base font-extrabold tracking-tight">{title}</h1>
              {badge && (
                <span className="inline-flex items-center rounded-md border border-border-subtle bg-main/40 px-1.5 py-0.5 text-[10px] font-semibold text-text-dim">
                  {badge}
                </span>
              )}
            </div>
            {subtitle && <p className="text-[11px] text-text-dim hidden sm:block">{subtitle}</p>}
          </div>
        </div>
        <div className="flex items-center gap-2 shrink-0 flex-wrap justify-end">
          {actions}
          {helpText && (
            <button
              onClick={() => setShowHelp(true)}
              className="flex h-8 w-8 items-center justify-center rounded-xl border border-border-subtle bg-surface text-text-dim hover:text-primary hover:border-primary/30 transition-colors duration-200"
              title={t("common.help", { defaultValue: "Help" })}
              aria-label={t("common.help", { defaultValue: "Help" })}
            >
              <HelpCircle className="h-4 w-4" />
            </button>
          )}
          {onRefresh && (
            <button
              className="flex h-8 items-center gap-1.5 rounded-xl border border-border-subtle bg-surface px-3 text-xs font-bold text-text-dim hover:text-primary hover:border-primary/30 hover:shadow-sm transition-colors duration-200"
              onClick={onRefresh}
              aria-label={t("common.refresh")}
              aria-busy={isFetching}
            >
              <RefreshCw className={`h-3.5 w-3.5 ${isFetching ? "animate-spin motion-reduce:animate-none" : ""}`} />
              <span className="hidden sm:inline">{t("common.refresh")}</span>
            </button>
          )}
        </div>
      </header>

      <Modal isOpen={showHelp && Boolean(helpText)} onClose={() => setShowHelp(false)} title={title} size="6xl">
        <div className="p-5 sm:p-8 lg:p-10">
          <MarkdownContent components={HELP_MARKDOWN_COMPONENTS}>
            {helpText ? toHelpMarkdown(helpText) : ""}
          </MarkdownContent>
        </div>
      </Modal>
    </>
  );
}
