import { useEffect, useRef, type ReactNode } from "react";
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
}

export function CollapsibleSection({
  title,
  children,
  defaultOpen,
  invalid,
}: CollapsibleSectionProps) {
  const ref = useRef<HTMLDetailsElement>(null);
  const wasInvalid = useRef(false);

  useEffect(() => {
    if (invalid && !wasInvalid.current && ref.current) {
      ref.current.open = true;
    }
    wasInvalid.current = Boolean(invalid);
  }, [invalid]);

  return (
    <details
      ref={ref}
      // `overflow-hidden` trims the body to the rounded corners while folded,
      // but it also clips an absolutely-positioned popover rendered by a child
      // (the model pickers in the manifest editor). Release the clip once the
      // section is open — the `open:` variant matches this element itself
      // (`group-open:` would target descendants, per Tailwind) — and keep it
      // for the folded state.
      className="group overflow-hidden rounded-xl border border-border-subtle/60 bg-surface/40 open:overflow-visible"
      open={defaultOpen || undefined}
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
