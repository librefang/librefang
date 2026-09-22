import type { ReactNode } from "react";
import { ChevronDown } from "lucide-react";

/**
 * A titled section that folds away, for grouping a long form.
 *
 * Native `<details>`/`<summary>` rather than `useState` + `aria-expanded`, which
 * is what the app's other collapsing panels use. The native element brings the
 * keyboard behaviour, the expanded/collapsed state and the click target with it,
 * and the chevron animates from CSS (`group-open:rotate-180`) so there is no
 * per-panel state to keep in sync.
 *
 * `invalid` forces the section open: a validation error the operator cannot see
 * is indistinguishable from no error at all.
 */
export interface CollapsibleSectionProps {
  title: string;
  children: ReactNode;
  defaultOpen?: boolean;
  invalid?: boolean;
  /**
   * Identity of the section as a whole, emitted as `data-section`.
   *
   * `title` is display text and therefore neither stable nor unique — two
   * sections may legitimately share a title with a field inside them — so
   * callers that need to address a section (the manifest editor's tab
   * routing, and the tests that guard it) carry the id here instead.
   */
  sectionId?: string;
}

export function CollapsibleSection({
  title,
  children,
  defaultOpen,
  invalid,
  sectionId,
}: CollapsibleSectionProps) {
  return (
    <details
      data-section={sectionId}
      className="group overflow-hidden rounded-xl border border-border-subtle/60 bg-surface/40"
      open={defaultOpen || invalid}
    >
      <summary
        aria-invalid={invalid || undefined}
        className="flex cursor-pointer list-none items-center justify-between p-3 select-none"
      >
        <span
          className={`text-[10px] font-bold uppercase tracking-widest ${
            invalid ? "text-error" : "text-text-dim"
          }`}
        >
          {title}
        </span>
        <ChevronDown className="h-4 w-4 text-text-dim transition-transform group-open:rotate-180" />
      </summary>
      <div className="space-y-2.5 px-3 pb-3">{children}</div>
    </details>
  );
}
