import { useId } from "react";

interface SliderInputProps {
  label: string;
  value: number;
  onChange: (v: number) => void;
  min: number;
  max: number;
  step?: number;
  enabled?: boolean;
  onToggle?: (enabled: boolean) => void;
  /** Format function for display ticks */
  formatTick?: (v: number) => string;
  /** Tick positions to display below the slider */
  ticks?: number[];
}

export function SliderInput({
  label,
  value,
  onChange,
  min,
  max,
  step = 1,
  enabled = true,
  onToggle,
  formatTick,
  ticks,
}: SliderInputProps) {
  const id = useId();
  const lowerBound = Math.min(min, max);
  const upperBound = Math.max(min, max);
  const clamp = (nextValue: number) =>
    Math.min(upperBound, Math.max(lowerBound, nextValue));
  const boundedValue = Number.isFinite(value) ? clamp(value) : lowerBound;
  const pct =
    upperBound === lowerBound
      ? 0
      : ((boundedValue - lowerBound) / (upperBound - lowerBound)) * 100;
  const emitValue = (rawValue: string) => {
    const nextValue = Number.parseFloat(rawValue);
    if (Number.isFinite(nextValue)) onChange(clamp(nextValue));
  };
  // A row that is inheriting the catalog default is dimmed, but the dimming belongs on the values, never on the switch that overrides them (refs #7782).
  // While it sat on the row container it faded the toggle too, leaving the default state of every parameter with no visible way out of it.
  const dimmed = enabled ? "" : " opacity-40";

  return (
    <div className="space-y-1.5">
      <div className="flex items-center justify-between gap-2">
        <label htmlFor={id} className={`text-xs font-bold text-text-dim${dimmed}`}>
          {label}
        </label>
        <div className="flex items-center gap-2">
          <input
            type="number"
            value={boundedValue}
            onChange={(e) => emitValue(e.target.value)}
            min={lowerBound}
            max={upperBound}
            step={step}
            disabled={!enabled}
            className={`w-20 rounded-lg border border-border-subtle bg-main px-2 py-1 text-xs text-right font-mono outline-none focus:border-brand disabled:cursor-not-allowed${dimmed}`}
          />
          {onToggle ? (
            <button
              type="button"
              role="switch"
              aria-checked={enabled}
              aria-label={label}
              onClick={() => onToggle(!enabled)}
              className={`relative w-8 h-[18px] rounded-full transition-colors outline-none focus-visible:ring-2 focus-visible:ring-brand ${
                enabled ? "bg-brand" : "bg-text-dim"
              }`}
            >
              <span
                className={`absolute top-0.5 w-3.5 h-3.5 rounded-full bg-white shadow transition-transform ${
                  enabled ? "translate-x-4" : "translate-x-0.5"
                }`}
              />
            </button>
          ) : null}
        </div>
      </div>
      <input
        id={id}
        type="range"
        min={lowerBound}
        max={upperBound}
        step={step}
        value={boundedValue}
        onChange={(e) => emitValue(e.target.value)}
        disabled={!enabled}
        className={`w-full h-1.5 rounded-full appearance-none cursor-pointer disabled:cursor-not-allowed accent-brand${dimmed}`}
        style={{
          background: enabled
            ? `linear-gradient(to right, var(--color-brand) ${pct}%, var(--color-border-subtle) ${pct}%)`
            : undefined,
        }}
      />
      {ticks ? (
        // Each label sits at the position its own value maps to, from the same
        // expression the filled track above uses, so the legend and the thumb
        // agree about where a value lives.
        //
        // This was `flex justify-between`, which spaces labels evenly whatever
        // they say. On a range whose ticks are not evenly spaced that is
        // actively misleading: the context-window row runs 1024..2097152 with
        // ticks at 32K/128K/512K/1M, so "1M" was drawn hard right when 1M is
        // the midpoint, and "128K" a third of the way across when its true
        // position is 6%. Reading a value off the legend was wrong by an order
        // of magnitude.
        <div className={`relative h-3 text-[9px] text-text-dim/50 font-mono${dimmed}`}>
          {ticks.map((t, index) => {
            const position =
              upperBound === lowerBound
                ? 0
                : ((t - lowerBound) / (upperBound - lowerBound)) * 100;
            const clamped = Math.min(100, Math.max(0, position));
            // Centre each label on its mark, except at the ends, where centring
            // would push half the text outside the track.
            const align =
              clamped <= 0
                ? "translate-x-0"
                : clamped >= 100
                  ? "-translate-x-full"
                  : "-translate-x-1/2";
            return (
              <span
                key={`${t}-${index}`}
                className={`absolute whitespace-nowrap ${align}`}
                style={{ left: `${clamped}%` }}
              >
                {formatTick ? formatTick(t) : t}
              </span>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}
