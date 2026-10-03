import { useCallback, useEffect, useRef, type ReactNode, type Ref } from "react";
import { useTranslation } from "react-i18next";
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
 * `invalid` opens the section when it *becomes* invalid: a validation error the
 * operator cannot see is indistinguishable from no error at all. It deliberately
 * does not close on recovery — clearing one of two errors while resubmitting must
 * not snap shut the section the operator is still working in (#8403 review). The
 * opening is imperative so user-driven toggling stays native and uncontrolled;
 * React never rewrites `open` for a manual toggle, so a hand-expanded section is
 * not force-collapsed either.
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
  /**
   * How many fields the folded section holds, shown as a badge next to the
   * title.
   *
   * The caller does the counting: "a field" is a question about the form
   * inside, and only the caller knows what it rendered there. Absent and zero
   * are the same answer — no badge at all, because "0 fields" is noise that
   * reads as a section which failed to load.
   */
  count?: number;
  /**
   * The section's own `<details>`, for a caller that has to measure the DOM
   * (the manifest editor, for `count` above). Merged with the ref the
   * invalid-open effect keeps, so lending one out does not cost that effect
   * its handle.
   */
  rootRef?: Ref<HTMLDetailsElement>;
}

export function CollapsibleSection({
  title,
  children,
  defaultOpen,
  invalid,
  sectionId,
  count,
  rootRef,
}: CollapsibleSectionProps) {
  const ref = useRef<HTMLDetailsElement>(null);
  const wasInvalid = useRef(false);

  useEffect(() => {
    if (invalid && !wasInvalid.current && ref.current) {
      ref.current.open = true;
    }
    wasInvalid.current = Boolean(invalid);
  }, [invalid]);

  // Stable identity, or React would detach and reattach the element every
  // render — and a callback `rootRef` would fire with it.
  const attachRef = useCallback(
    (node: HTMLDetailsElement | null) => {
      ref.current = node;
      if (typeof rootRef === "function") rootRef(node);
      else if (rootRef) rootRef.current = node;
    },
    [rootRef],
  );

  return (
    <details
      ref={attachRef}
      data-section={sectionId}
      className="group overflow-hidden rounded-xl border border-border-subtle/60 bg-surface/40"
      open={defaultOpen || undefined}
    >
      <summary
        aria-invalid={invalid || undefined}
        className="flex cursor-pointer list-none items-center justify-between gap-2 p-3 select-none"
      >
        <span className="flex items-center gap-2">
          <span
            className={`text-[10px] font-bold uppercase tracking-widest ${
              invalid ? "text-error" : "text-text-dim"
            }`}
          >
            {title}
          </span>
          <FieldCountBadge count={count ?? 0} />
        </span>
        <ChevronDown className="h-4 w-4 text-text-dim transition-transform group-open:rotate-180" />
      </summary>
      <div className="space-y-2.5 px-3 pb-3">{children}</div>
    </details>
  );
}

/**
 * The badge that advertises how many fields a section holds: the digit the eye
 * reads, the plural unit a screen reader needs.
 *
 * Exported because three surfaces show it — the folded sections, the
 * always-open `Section` and the form-level total — and one markup is one place
 * for the accessible label to be right. `labelKey` lets the total say
 * "fields total" while a section says "fields".
 *
 * Absent and zero are the same answer — no badge at all, because "0 fields" is
 * noise that reads as a section which failed to load — so the guard lives
 * here rather than at each call site.
 */
export function FieldCountBadge({
  count,
  labelKey = "agents.form.field_count",
}: {
  count: number;
  labelKey?: string;
}) {
  const { t } = useTranslation();
  if (count <= 0) return null;
  return (
    <span className="inline-flex shrink-0 items-center rounded-full bg-main/60 px-1.5 py-0.5 text-[10px] font-medium text-text-dim">
      {/* The digit is what the eye reads; the unit is what a screen reader
          needs, so the visible half is hidden from the accessible name and
          the label replaces it. */}
      <span aria-hidden="true">{count}</span>
      <span className="sr-only">{t(labelKey, { count })}</span>
    </span>
  );
}
