// Renders the images detected in a workflow step's output as a gallery.
// Pure presentation — no fetch, no data access; the helper that produces
// `refs` is in src/lib/workflowOutputImages.ts and is already URL-safety
// checked, so we render `<img src={ref.src}>` directly.

import type { ImageRef } from "../lib/workflowOutputImages";

interface Props {
  refs: ImageRef[];
  /** Optional label rendered above the gallery (i18n string from caller). */
  label?: string;
}

export function WorkflowStepImageGallery({ refs, label }: Props) {
  if (refs.length === 0) return null;

  return (
    <div data-testid="workflow-step-image-gallery" className="space-y-1.5">
      {label && (
        <p className="text-[9px] font-bold text-text-dim/50">{label}</p>
      )}
      <div className="flex flex-wrap gap-2">
        {refs.map((ref) => (
          <a
            key={ref.src}
            href={ref.src}
            target="_blank"
            rel="noreferrer noopener"
            className="block rounded-lg overflow-hidden border border-border-subtle hover:border-brand/40 transition-colors max-w-[200px]"
            title={ref.alt}
          >
            <img
              src={ref.src}
              alt={ref.alt || "generated image"}
              loading="lazy"
              className="block max-h-[200px] w-auto object-contain bg-main/30"
            />
          </a>
        ))}
      </div>
    </div>
  );
}
