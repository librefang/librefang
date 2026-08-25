// Renders URL-safety-checked workflow images and authenticates protected same-origin API assets before exposing them to `<img>`.
// Embedded `data:` images render bare: browsers block top-level navigation to a data URI, so wrapping them in a link only produces a dead click target.

import type { ImageRef } from "../lib/workflowOutputImages";
import { useTranslation } from "react-i18next";
import { AuthenticatedImage } from "./AuthenticatedImage";

interface Props {
  refs: ImageRef[];
  /** Optional label rendered above the gallery (i18n string from caller). */
  label?: string;
}

export function WorkflowStepImageGallery({ refs, label }: Props) {
  const { t } = useTranslation();
  if (refs.length === 0) return null;

  return (
    <div data-testid="workflow-step-image-gallery" className="space-y-1.5">
      {label && (
        <p className="text-[9px] font-bold text-text-dim/50">{label}</p>
      )}
      <div className="flex flex-wrap gap-2">
        {refs.map((ref, index) => {
          const isEmbedded = ref.kind === "data-uri";
          return (
            <AuthenticatedImage
              // The same image can legitimately appear twice in one step's output, so the source alone is not a unique key.
              key={`${ref.src}-${index}`}
              src={ref.src}
              alt={ref.alt || t("workflows.generated_image_alt", {
                defaultValue: "generated image",
              })}
              loading="lazy"
              className={isEmbedded
                ? "block max-h-[200px] max-w-[200px] w-auto rounded-lg border border-border-subtle object-contain bg-main/30"
                : "block max-h-[200px] w-auto object-contain bg-main/30"}
              linkProps={isEmbedded ? undefined : {
                target: "_blank",
                rel: "noopener noreferrer",
                className: "block rounded-lg overflow-hidden border border-border-subtle hover:border-brand/40 transition-colors max-w-[200px]",
                title: ref.alt,
              }}
            />
          );
        })}
      </div>
    </div>
  );
}
