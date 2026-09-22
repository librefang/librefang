import type { ReactNode } from "react";
import { cn } from "../../lib/cn";

/**
 * Label + control + hint/error, the shape every form in the dashboard draws.
 *
 * There were four hand-rolled versions of this (57 / 19 / 9 / 3 uses) with three
 * different type scales and two different wrappers. This is the one the manifest
 * editor already used, which is the only one carrying `required`, `invalid` and
 * the `role="alert"` error slot — so promoting it propagates the accessibility
 * work rather than losing it.
 *
 * The wrapper is a `<div>`, not a `<label>`, and that is load-bearing (#5246): a
 * `<label>` forwards every click inside its bounds to its first labelable
 * control, which silently ate clicks meant for composite widgets like
 * `MultiSelectCmdk` — picking a skill from the catalog never reached the
 * option's handler and the chip was never added.
 *
 * The cost of a bare `<div>` is that clicking the label text no longer focuses
 * the control. Pass `htmlFor` with the matching `id` on the control to get it
 * back: a `<label htmlFor>` that sits *beside* its control rather than wrapping
 * it has none of the #5246 behaviour, so both properties hold at once.
 */
export interface FieldProps {
  /**
   * The visible label. Optional only for the case where the surrounding card
   * already names the field — a section whose title *is* the field's name,
   * where drawing both reads as the same word twice, one line apart. Those
   * callers pass `ariaLabel` instead, so the control keeps a name to announce.
   */
  label?: string;
  hint?: string;
  /** Marks the label with an asterisk. Validation is the caller's job. */
  required?: boolean;
  /** Paints the label as error text and renders `error` below the control. */
  invalid?: boolean;
  error?: string;
  /** Id for the error node, so the control can point at it with `aria-describedby`. */
  errorId?: string;
  /**
   * Id of the control this labels. When set, the label becomes a real
   * `<label>` and clicking it focuses that control. The caller must put the
   * same id on the control.
   */
  htmlFor?: string;
  /**
   * The accessible name for a field with no visible label.
   *
   * Rendered as a labelled `role="group"` around the control, which is what a
   * composite widget (the skills and MCP finders) needs: their input carries no
   * accessible name of its own — a placeholder is a hint, not a name, and the
   * finder's only `aria-label` sits on the listbox it opens — so the group is
   * what tells the operator which field they are in.
   */
  ariaLabel?: string;
  children: ReactNode;
}

export function Field({
  label,
  hint,
  required,
  invalid,
  error,
  errorId,
  htmlFor,
  ariaLabel,
  children,
}: FieldProps) {
  const labelClass = cn(
    "block text-[10px] font-bold uppercase",
    invalid ? "text-error" : "text-text-dim",
  );
  const labelNode = (
    <>
      {label}
      {required && <span className="ml-0.5 text-error">*</span>}
    </>
  );

  // The group is only needed when there is no visible label; with one, the
  // label already names what is inside it.
  const groupLabel = !label && ariaLabel ? ariaLabel : undefined;

  return (
    <div
      className="block"
      role={groupLabel ? "group" : undefined}
      aria-label={groupLabel}
    >
      {label &&
        (htmlFor ? (
          <label className={labelClass} htmlFor={htmlFor}>
            {labelNode}
          </label>
        ) : (
          <span className={labelClass}>{labelNode}</span>
        ))}
      <span className={label ? "mt-1 block" : "block"}>{children}</span>
      {invalid && error && (
        <span id={errorId} className="mt-1 block text-[10px] text-error" role="alert">
          {error}
        </span>
      )}
      {hint && <span className="mt-1 block text-[10px] text-text-dim/70">{hint}</span>}
    </div>
  );
}
