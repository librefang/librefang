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
  const pct = max === min ? 0 : ((value - min) / (max - min)) * 100;

  return (
    <div className={`space-y-1.5 ${!enabled ? "opacity-40" : ""}`}>
      <div className="flex items-center justify-between gap-2">
        <label htmlFor={id} className="text-xs font-bold text-text-dim">
          {label}
        </label>
        <div className="flex items-center gap-2">
          <input
            type="number"
            value={value}
            onChange={(e) => {
              const v = parseFloat(e.target.value);
              if (!isNaN(v)) onChange(Math.min(max, Math.max(min, v)));
            }}
            min={min}
            max={max}
            step={step}
            disabled={!enabled}
            className="w-20 rounded-lg border border-border-subtle bg-main px-2 py-1 text-xs text-right font-mono outline-none focus:border-primary disabled:cursor-not-allowed"
          />
          {onToggle ? (
            <button
              type="button"
              role="switch"
              aria-checked={enabled}
              aria-label={label}
              onClick={() => onToggle(!enabled)}
              className={`relative w-8 h-[18px] rounded-full transition-colors ${
                enabled ? "bg-primary" : "bg-border-subtle"
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
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(e) => onChange(parseFloat(e.target.value))}
        disabled={!enabled}
        className="w-full h-1.5 rounded-full appearance-none cursor-pointer disabled:cursor-not-allowed accent-brand"
        style={{
          background: enabled
            ? `linear-gradient(to right, var(--color-brand) ${pct}%, var(--color-border-subtle) ${pct}%)`
            : undefined,
        }}
      />
      {ticks ? (
        <div className="flex justify-between text-[9px] text-text-dim/50 font-mono px-0.5">
          {ticks.map((t) => (
            <span key={t}>{formatTick ? formatTick(t) : t}</span>
          ))}
        </div>
      ) : null}
    </div>
  );
}
