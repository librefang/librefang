import { memo, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Maximize2 } from "lucide-react";
import { useUIStore } from "../../lib/store";
import { Modal } from "./Modal";

/**
 * Renders a ```mermaid fenced block as a diagram.
 *
 * Three things shape this component:
 *
 * - **Mermaid is loaded on demand.** It is the largest single dependency in the
 *   dashboard, and most sessions never see a diagram, so it is behind a dynamic
 *   `import()` that Vite splits into its own chunk. The promise is cached at
 *   module scope so a conversation with twenty diagrams parses the library once.
 * - **A diagram that does not parse falls back to its source.** Agents emit
 *   Mermaid that is wrong, truncated, or a dialect this version does not know;
 *   none of that is a reason to show the operator an error where a code block
 *   would do. The streaming view never reaches here at all — `Typewriter_v2`
 *   renders its own `<pre>` — so a half-written diagram is shown as text while
 *   it arrives and becomes a diagram once the turn settles.
 * - **`securityLevel: "strict"`** keeps Mermaid from emitting raw HTML in node
 *   labels or wiring `click` directives to scripts. Chat content is model
 *   output, which is untrusted input by definition.
 */

type MermaidApi = typeof import("mermaid")["default"];

let mermaidPromise: Promise<MermaidApi> | null = null;

function loadMermaid(): Promise<MermaidApi> {
  mermaidPromise ??= import("mermaid").then((mod) => mod.default);
  return mermaidPromise;
}

// `mermaid.render` needs an id that is unique per document, not per component:
// it parks a measuring element in the DOM under that id while it lays the
// diagram out, and two concurrent renders sharing an id clobber each other.
// A module-global counter is what makes it document-wide; a per-instance one
// would collide across components by construction.
let seq = 0;

/**
 * Apply the same link policy to a diagram's anchors as to markdown links.
 *
 * Mermaid emits a `click X "url" _blank` directive as
 * `<a xlink:href="…" target="_blank">` and never sets `rel`, so a diagram node
 * would be the one link on the page opting out of the `noopener noreferrer`
 * that `MarkdownContent` gives every other link — and it looks like a diagram
 * node, not like a link.
 *
 * Anchors mermaid did not emit cannot be here: `DOMPurify` has already run over
 * the whole SVG inside `mermaid.render`.
 */
function withLinkPolicy(svg: string): string {
  return svg.replace(/<a\b(?![^>]*\brel=)/gi, '<a rel="noopener noreferrer"');
}

interface MermaidDiagramProps {
  /** The fenced block's contents, verbatim. */
  source: string;
}

export const MermaidDiagram = memo(function MermaidDiagram({ source }: MermaidDiagramProps) {
  const { t } = useTranslation();
  const theme = useUIStore((s) => s.theme);
  const [svg, setSvg] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const [zoomed, setZoomed] = useState(false);
  // Guards against a late render landing after the component unmounted or the
  // source changed — chat re-renders on every streamed token upstream of here.
  const renderToken = useRef(0);

  useEffect(() => {
    const token = ++renderToken.current;
    let cancelled = false;
    setFailed(false);

    loadMermaid()
      .then(async (mermaid) => {
        mermaid.initialize({
          startOnLoad: false,
          securityLevel: "strict",
          theme: theme === "light" ? "default" : "dark",
          fontFamily: "inherit",
          // Pinned, and pinned *through* `secure`, because `layout` is not one
          // of the keys mermaid protects from an `%%{init: …}%%` directive. A
          // single line of diagram source — which may have come from a web page
          // the agent read — would otherwise pull the 1.4 MB ELK layout engine.
          layout: "dagre",
          secure: [
            "secure",
            "securityLevel",
            "startOnLoad",
            "maxTextSize",
            "suppressErrorRendering",
            "maxEdges",
            "layout",
          ],
        });
        const { svg: rendered } = await mermaid.render(`mermaid-${seq++}`, source);
        if (cancelled || token !== renderToken.current) return;
        // An empty or absent SVG is a failure, not a diagram: mounting it would
        // give an empty box announced as a diagram, with the source gone.
        if (rendered) setSvg(withLinkPolicy(rendered));
        else setFailed(true);
      })
      .catch(() => {
        if (!cancelled && token === renderToken.current) {
          setSvg(null);
          setFailed(true);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [source, theme]);

  if (failed || !svg) {
    return (
      <pre
        className="p-2 rounded-lg bg-main font-mono text-[11px] overflow-x-auto mb-1.5"
        data-testid="mermaid-source"
        aria-label={failed ? t("chat.mermaid_unrenderable", { defaultValue: "Diagram source (could not be rendered)" }) : undefined}
      >
        <code>{source}</code>
      </pre>
    );
  }

  return (
    <figure className="mb-1.5 min-w-0">
      <div className="relative min-w-0 group">
        {/*
          The enlarge affordance is a sibling button, not a wrapper around the
          diagram: mermaid emits `<a>` for a `click` directive, and an anchor
          inside a <button> is nested interactive content — invalid HTML, and a
          link a keyboard user cannot reach. The container carries the pointer
          affordance; this button carries the accessible name.
        */}
        <button
          type="button"
          onClick={() => setZoomed(true)}
          aria-label={t("chat.mermaid_enlarge", { defaultValue: "Enlarge diagram" })}
          className="absolute top-1 right-1 z-10 rounded-md border border-border-subtle bg-surface/90 p-1 text-text-dim opacity-0 transition-opacity hover:text-text-main focus-visible:opacity-100 group-hover:opacity-100"
        >
          <Maximize2 className="h-3 w-3" />
        </button>
        <div
          // A `click X "url"` directive makes a node an anchor, and a click on
          // one would otherwise both follow the link and open the modal. The
          // link wins: it is the more specific intent, and the enlarge control
          // is a keystroke away.
          onClick={(event) => {
            if ((event.target as HTMLElement).closest("a")) return;
            setZoomed(true);
          }}
          className="min-w-0 overflow-x-auto rounded-lg bg-main p-2 cursor-zoom-in [&>svg]:!max-w-full [&>svg]:!max-h-64 [&>svg]:!w-auto [&>svg]:!h-auto"
          data-testid="mermaid-diagram"
        // Mermaid returns an SVG string; there is no React tree to hand back.
        // It is generated by Mermaid itself under `securityLevel: "strict"`,
        // which strips raw HTML from labels, drops `javascript:` hrefs and
        // refuses `click` callbacks — verified against mermaid 12 with hostile
        // input, including an `%%{init:{"securityLevel":"loose"}}%%` directive,
        // which mermaid's own `secure` list rejects.
        //
        // `!max-w-full`, with the important modifier, because mermaid writes
        // `style="max-width: …px"` onto the `<svg>` itself and an inline style
        // beats a class. A diagram wider than the bubble therefore ignored the
        // cap and pushed out of the chat column. The `!` also caps a source
        // that asked for a fixed pixel width — `%%{init:{"flowchart":
        // {"useMaxWidth":false}}}%%` emits `width="1234"`, and `useMaxWidth` is
        // not one of the keys mermaid's `secure` list protects, so model output
        // could otherwise choose how wide the chat is.
        //
        // `!w-auto` with `!max-h-64` is what makes it scale rather than clip.
        // Mermaid sets `width="100%"` as a presentation attribute, and a
        // replaced element pinned to a width takes a height cap by squashing;
        // released to `auto`, the intrinsic ratio from its `viewBox` governs
        // and both caps behave as "contain". A diagram is a thumbnail in the
        // transcript and full size in the enlarged view — reading a dense one
        // inline was never going to work in a chat column.
          dangerouslySetInnerHTML={{ __html: svg }}
        />
      </div>
      {/*
        Fitting the diagram to the column makes a dense one unreadable, so the
        full-size view is one click away rather than a horizontal scrollbar the
        column is too narrow to make usable.
      */}
      <Modal
        isOpen={zoomed}
        onClose={() => setZoomed(false)}
        size="7xl"
        title={t("chat.mermaid_diagram", { defaultValue: "Diagram" })}
      >
        <div
          className="overflow-auto max-h-[80vh] rounded-lg bg-main p-4 [&>svg]:h-auto"
          data-testid="mermaid-diagram-zoomed"
          // Deliberately uncapped here: this view exists to show the diagram at
          // its natural size, and the container scrolls in both directions.
          //
          // The same markup, ids and all. Re-scoping them to avoid the
          // duplicate ids this second mount creates is worse than the problem:
          // mermaid renders `#${id} .node { … }` rules into a <style> it makes
          // the SVG's first child (`createUserStyles(…, idSelector)`), so a
          // renamed root id leaves every one of those selectors matching
          // nothing and the enlarged copy loses its styling. Duplicate ids are
          // invalid markup; a diagram drawn without its own stylesheet is a
          // visible regression.
          dangerouslySetInnerHTML={{ __html: svg }}
        />
      </Modal>
      {/*
        The source stays reachable. `role="img"` on the container would collapse
        the whole diagram to one node for a screen reader — and every diagram on
        the page would announce the same word — so the SVG keeps mermaid's own
        <title>/<desc>, and this is the way to the text it was drawn from.
      */}
      <details className="mt-1">
        <summary className="cursor-pointer text-[10px] text-text-dim hover:text-text-main">
          {t("chat.mermaid_show_source", { defaultValue: "Diagram source" })}
        </summary>
        <pre className="mt-1 p-2 rounded-lg bg-main font-mono text-[11px] overflow-x-auto">
          <code>{source}</code>
        </pre>
      </details>
    </figure>
  );
});
