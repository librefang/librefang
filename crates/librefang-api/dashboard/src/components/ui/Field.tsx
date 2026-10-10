import { useId, type ReactNode } from "react";
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
   * composite widget (the skills and MCP finders) needs: its own trigger is
   * named generically, so without this the operator hears "Select options" and
   * not which field they are in.
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
      {/* Decoration only: `aria-hidden` keeps it out of the control's
          accessible name, so an `htmlFor` label is announced as "Name",
          not "Name*" (#8403 review). */}
      {required && (
        <span aria-hidden="true" className="ml-0.5 text-error">
          *
        </span>
      )}
    </>
  );

  // The wrapper is a `<div>` and, without `htmlFor`, the visible label is a
  // `<span>` — and a `<span>` names nothing. So a control inside a labelled
  // Field used to have no accessible name at all: it could not come from the
  // label (a span), and it could not come from the wrapper, which had no
  // `role` unless the field was *unlabelled* with an `ariaLabel`.
  //
  // Naming the group is the `fieldset`/`legend` pattern and the same mechanism
  // the finders already use through `ariaLabel`; the difference was only that
  // a labelled Field never got one. Measured before this: 31 controls across
  // the agent form had neither a name of their own nor a group's, and every
  // one of them sat inside a labelled Field.
  //
  // `aria-labelledby` rather than `aria-label`: the name is the visible text,
  // so pointing at it keeps the two in step and lets the required asterisk
  // come along. `htmlFor` fields are left alone — there the label is a real
  // `<label>` and already names the control.
  const labelId = useId();
  const groupFromLabel = Boolean(label) && !htmlFor;
  const groupFromAria = !label && Boolean(ariaLabel);
  const isGroup = groupFromLabel || groupFromAria;

  return (
    <div
      className="block"
      role={isGroup ? "group" : undefined}
      aria-label={groupFromAria ? ariaLabel : undefined}
      aria-labelledby={groupFromLabel ? labelId : undefined}
    >
      {label &&
        (htmlFor ? (
          <label className={labelClass} htmlFor={htmlFor}>
            {labelNode}
          </label>
        ) : (
          <span id={labelId} className={labelClass}>
            {labelNode}
          </span>
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
