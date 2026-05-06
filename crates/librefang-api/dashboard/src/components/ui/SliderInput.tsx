import { useId, useState, useCallback, useEffect } from "react";

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
  const safeMin = Math.min(min, max);
  const safeMax = Math.max(min, max);
  const pct = safeMax === safeMin ? 0 : ((value - safeMin) / (safeMax - safeMin)) * 100;

  const [textValue, setTextValue] = useState<string>(String(value));
  useEffect(() => {
    const parsed = parseFloat(textValue);
    if (isNaN(parsed) || parsed !== value) {
      setTextValue(String(value));
    }
  }, [value]);
  const commitTextValue = useCallback(
    (raw: string) => {
      const v = parseFloat(raw);
      if (!isNaN(v)) {
        const clamped = Math.min(safeMax, Math.max(safeMin, v));
        onChange(clamped);
        setTextValue(String(clamped));
      } else {
        setTextValue(String(value));
      }
    },
    [safeMin, safeMax, onChange, value],
  );

  return (
    <div className={`space-y-1.5 ${!enabled ? "opacity-40" : ""}`}>
      <div className="flex items-center justify-between gap-2">
        <label htmlFor={id} className="text-xs font-bold text-text-dim">
          {label}
        </label>
        <div className="flex items-center gap-2">
          <input
            type="text"
            inputMode="decimal"
            value={textValue}
            onChange={(e) => setTextValue(e.target.value)}
            onBlur={() => commitTextValue(textValue)}
            onKeyDown={(e) => {
              if (e.key === "Enter") commitTextValue(textValue);
            }}
            disabled={!enabled}
            className="w-20 rounded-lg border border-border-subtle bg-main px-2 py-1 text-xs text-right font-mono outline-none focus:border-brand disabled:cursor-not-allowed"
          />
          {onToggle ? (
            <button
              type="button"
              role="switch"
              aria-checked={enabled}
              aria-label={label}
              onClick={() => onToggle(!enabled)}
              className={`relative w-8 h-[18px] rounded-full transition-colors ${
                enabled ? "bg-brand" : "bg-border-subtle"
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
            ? `linear-gradient(to right, var(--color-brand, #6366f1) ${pct}%, var(--color-border-subtle, #d1d5db) ${pct}%)`
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
