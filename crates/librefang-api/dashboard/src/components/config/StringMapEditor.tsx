import { useCallback, useMemo } from "react";
import { Plus, X } from "lucide-react";
import { useTranslation } from "react-i18next";

// Editor for `BTreeMap<String, String | Number>` config fields. The whole
// section (e.g. `provider_urls`, `tool_timeouts`) is a single value posted
// to /api/config/set as one object.
//
// Tracks entries as an ordered array internally so a row in the middle
// can have its key edited without losing the row when the user types
// through a duplicate or empty intermediate state. The committed value
// only contains rows whose key trims to non-empty.

type Entry = readonly [string, string | number];

type Props = {
  value: Record<string, string | number> | null | undefined;
  onChange: (next: Record<string, string | number>) => void;
  valueType?: "string" | "number";
  keyPlaceholder?: string;
  valuePlaceholder?: string;
  // Step / min / max apply to numeric values only. Ignored for string maps.
  min?: number;
  max?: number;
  step?: number;
};

function entriesFromValue(value: Props["value"]): Entry[] {
  if (!value || typeof value !== "object") return [];
  return Object.entries(value);
}

function commit(entries: Entry[], onChange: Props["onChange"]) {
  const obj: Record<string, string | number> = {};
  for (const [k, v] of entries) {
    const trimmedKey = k.trim();
    if (trimmedKey) obj[trimmedKey] = v;
  }
  onChange(obj);
}

export function StringMapEditor({
  value, onChange, valueType = "string",
  keyPlaceholder, valuePlaceholder,
  min, max, step,
}: Props) {
  const { t } = useTranslation();
  const entries = useMemo(() => entriesFromValue(value), [value]);

  const updateKey = useCallback((idx: number, key: string) => {
    const next = entries.map((e, i): Entry => i === idx ? [key, e[1]] : e);
    commit(next, onChange);
  }, [entries, onChange]);

  const updateValue = useCallback((idx: number, raw: string) => {
    const parsed: string | number = valueType === "number"
      ? (raw === "" ? 0 : Number(raw))
      : raw;
    const next = entries.map((e, i): Entry => i === idx ? [e[0], parsed] : e);
    commit(next, onChange);
  }, [entries, onChange, valueType]);

  const removeRow = useCallback((idx: number) => {
    commit(entries.filter((_, i) => i !== idx), onChange);
  }, [entries, onChange]);

  const addRow = useCallback(() => {
    commit([...entries, ["", valueType === "number" ? 0 : ""]], onChange);
  }, [entries, onChange, valueType]);

  const inputClass =
    "px-2.5 py-1.5 rounded-lg border border-border-subtle bg-main text-xs font-mono outline-none focus:border-brand transition-colors";

  return (
    <div className="flex flex-col gap-1.5">
      {entries.length === 0 && (
        <p className="text-[10px] text-text-dim italic">
          {t("config.map_empty", "No entries — click Add to create one")}
        </p>
      )}
      {entries.map(([k, v], idx) => (
        <div key={idx} className="flex items-center gap-1.5">
          <input
            type="text"
            value={k}
            onChange={(e) => updateKey(idx, e.target.value)}
            placeholder={keyPlaceholder ?? t("config.map_key", "key")}
            className={`${inputClass} flex-1 min-w-0`}
            autoComplete="off"
            spellCheck={false}
          />
          <span className="text-[10px] text-text-dim shrink-0">=</span>
          <input
            type={valueType === "number" ? "number" : "text"}
            value={String(v ?? "")}
            onChange={(e) => updateValue(idx, e.target.value)}
            placeholder={valuePlaceholder ?? t("config.map_value", "value")}
            min={valueType === "number" ? min : undefined}
            max={valueType === "number" ? max : undefined}
            step={valueType === "number" ? step : undefined}
            className={`${inputClass} flex-1 min-w-0`}
            autoComplete="off"
            spellCheck={false}
          />
          <button
            type="button"
            onClick={() => removeRow(idx)}
            className="p-1 rounded-md text-text-dim hover:text-danger hover:bg-surface-hover transition-colors shrink-0"
            title={t("config.remove_row", "Remove")}
            aria-label={t("config.remove_row", "Remove")}
          >
            <X className="w-3 h-3" />
          </button>
        </div>
      ))}
      <button
        type="button"
        onClick={addRow}
        className="flex items-center gap-1 self-start px-2 py-1 rounded-md text-[10px] text-text-dim hover:text-brand hover:bg-surface-hover transition-colors"
      >
        <Plus className="w-3 h-3" />
        {t("config.add_row", "Add")}
      </button>
    </div>
  );
}
