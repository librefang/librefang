import { memo } from "react";
import { canvasSrcDoc, type ChatCanvas as ChatCanvasData } from "../lib/chat";

/**
 * Renders a canvas an agent presented with `canvas_present` inside the chat transcript.
 *
 * The HTML is untrusted model output, so it goes into an iframe with an empty `sandbox` (scripts, same-origin access, forms, and popups all disabled) and a restrictive CSP from `canvasSrcDoc`.
 */
export const ChatCanvas = memo(function ChatCanvas({ canvas }: { canvas: ChatCanvasData }) {
  return (
    <figure className="w-full rounded-lg border border-border-subtle bg-surface overflow-hidden">
      <figcaption className="px-3 py-1.5 text-[11px] font-medium text-text-dim border-b border-border-subtle truncate">
        {canvas.title}
      </figcaption>
      <iframe
        title={canvas.title}
        sandbox=""
        referrerPolicy="no-referrer"
        srcDoc={canvasSrcDoc(canvas.html)}
        className="block w-full h-[360px] bg-white"
      />
    </figure>
  );
});
