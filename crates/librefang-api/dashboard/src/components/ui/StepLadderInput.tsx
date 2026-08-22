import { useId, useState } from "react";
import { formatTokens, isOnLadder, ladderUpTo } from "../../lib/modelParamLadders";

interface StepLadderInputProps {
  label: string;
  /** Raw form value. `""` is the inherit state, not zero. */
  value: string;
  onChange: (next: string) => void;
  /** Preset rungs, smallest first. */
  ladder: readonly number[];
  /**
   * The model's declared maximum, when some source vouched for it.
   * Trims the ladder so no rung is offered that the endpoint cannot honour.
   * Leave `undefined` for a limit that was never measured — an unknown limit is not a ceiling
   * (#7780), and capping against a placeholder would hide rungs that may well work.
   */
  cap?: number;
  /** Label for the "let the model / system decide" rung. */
  inheritLabel: string;
  /** Label for the rung that opens the free-entry field. */
  customLabel: string;
  /** Placeholder for the custom field. */
  customPlaceholder?: string;
  /** Optional advisory shown under the control, e.g. an over-limit warning. */
  warning?: string;
}

/**
 * A row of preset token counts plus `inherit` and `custom`.
 *
 * Replaces the slider these fields used to have.
 * The rungs are discrete because the choices are discrete, and `inherit` is a rung rather than an
 * empty box, so "this agent has no opinion, let the model's setting apply" is something you can
 * point at instead of something you infer from a blank field.
 *
 * `custom` stays selected while the typed value is off-ladder, so typing 50000 does not make the
 * field flicker back to a preset on the next render.
 */
export function StepLadderInput({
  label,
  value,
  onChange,
  ladder,
  cap,
  inheritLabel,
  customLabel,
  customPlaceholder,
  warning,
}: StepLadderInputProps) {
  const id = useId();
  const rungs = ladderUpTo(ladder, cap);
  const parsed = value.trim() === "" ? null : Number(value);
  const numeric = parsed !== null && Number.isFinite(parsed) ? parsed : null;

  // Custom is an explicit mode, not something derived purely from the value.
  // Deriving it meant pressing "custom" while a preset was selected seeded the
  // field with that preset — which is on the ladder — so the control decided it
  // was not in custom mode and the field never appeared. The operator pressed
  // the button and nothing happened.
  //
  // The value still forces the mode on: a stored number that is not a rung has
  // nowhere else to be edited.
  const [customMode, setCustomMode] = useState(false);
  const offLadder = value.trim() !== "" && !isOnLadder(rungs, numeric);
  const isCustom = customMode || offLadder;

  const pick = (next: string): void => {
    setCustomMode(false);
    onChange(next);
  };

  const rungClass = (selected: boolean): string =>
    `px-2 py-1 rounded-lg border text-xs font-mono transition-colors ${
      selected
        ? "border-brand bg-brand/10 text-brand"
        : "border-border-subtle bg-main text-text-dim hover:border-brand/50"
    }`;

  return (
    <div className="space-y-1.5">
      <label htmlFor={id} className="text-xs font-bold text-text-dim">
        {label}
      </label>
      <div id={id} className="flex flex-wrap gap-1.5">
        <button
          type="button"
          aria-pressed={value.trim() === ""}
          className={rungClass(value.trim() === "")}
          onClick={() => pick("")}
        >
          {inheritLabel}
        </button>
        {rungs.map((rung) => (
          <button
            key={rung}
            type="button"
            aria-pressed={!isCustom && numeric === rung}
            className={rungClass(!isCustom && numeric === rung)}
            onClick={() => pick(String(rung))}
          >
            {formatTokens(rung)}
          </button>
        ))}
        <button
          type="button"
          aria-pressed={isCustom}
          className={rungClass(isCustom)}
          // Seed the field with the current preset so the operator edits a number
          // rather than starting from an empty box.
          onClick={() => {
            setCustomMode(true);
            onChange(numeric !== null ? String(numeric) : String(rungs[0] ?? 1));
          }}
        >
          {customLabel}
        </button>
      </div>
      {isCustom ? (
        <input
          type="number"
          min="1"
          value={value}
          aria-label={`${label} — ${customLabel}`}
          onChange={(e) => onChange(e.target.value)}
          placeholder={customPlaceholder}
          className="w-full rounded-lg border border-border-subtle bg-main px-2 py-1 text-xs font-mono outline-none focus:border-brand"
        />
      ) : null}
      {warning ? (
        <p className="text-[11px] text-red-400 flex items-start gap-1">
          <span aria-hidden="true">⚠</span>
          <span>{warning}</span>
        </p>
      ) : null}
    </div>
  );
}
