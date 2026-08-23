// Renders URL-safety-checked workflow images and authenticates protected same-origin API assets before exposing them to `<img>`.

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
        {refs.map((ref) => (
          <AuthenticatedImage
            key={ref.src}
            src={ref.src}
            alt={ref.alt || t("workflows.generated_image_alt", {
              defaultValue: "generated image",
            })}
            loading="lazy"
            className="block max-h-[200px] w-auto object-contain bg-main/30"
            linkProps={{
              target: "_blank",
              rel: "noopener noreferrer",
              className: "block rounded-lg overflow-hidden border border-border-subtle hover:border-brand/40 transition-colors max-w-[200px]",
              title: ref.alt,
            }}
          />
        ))}
      </div>
    </div>
  );
}
