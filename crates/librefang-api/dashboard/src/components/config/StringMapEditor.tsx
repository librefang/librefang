import { useCallback, useEffect, useRef, useState } from "react";
import { Plus, X } from "lucide-react";
import { useTranslation } from "react-i18next";

// Editor for `BTreeMap<String, String | Number>` config fields. The whole
// section (e.g. `provider_urls`, `tool_timeouts`) is a single value posted
// to /api/config/set as one object.
//
// Tracks rows as a local-state array with stable per-row ids so a row in
// the middle keeps its identity even when the user types through a
// duplicate / empty intermediate key state. The committed object posted
// upward only contains rows whose key trims to non-empty; duplicates
// resolve last-write-wins (matches Object.fromEntries semantics).

type Row = { id: string; key: string; value: string | number };

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

let rowIdCounter = 0;
function newRowId(): string {
  rowIdCounter += 1;
  return `r${rowIdCounter}`;
}

function rowsFromValue(value: Props["value"]): Row[] {
  if (!value || typeof value !== "object") return [];
  return Object.entries(value).map(([k, v]) => ({ id: newRowId(), key: k, value: v }));
}

function commitRows(rows: Row[]): Record<string, string | number> {
  const out: Record<string, string | number> = {};
  for (const r of rows) {
    const trimmed = r.key.trim();
    if (trimmed) out[trimmed] = r.value;
  }
  return out;
}

function rowsMatchValue(rows: Row[], value: Props["value"]): boolean {
  const obj = value && typeof value === "object" ? value : {};
  const committed = commitRows(rows);
  const a = Object.keys(committed);
  const b = Object.keys(obj);
  if (a.length !== b.length) return false;
  for (const k of a) {
    if (committed[k] !== (obj as Record<string, string | number>)[k]) return false;
  }
  return true;
}

export function StringMapEditor({
  value, onChange, valueType = "string",
  keyPlaceholder, valuePlaceholder,
  min, max, step,
}: Props) {
  const { t } = useTranslation();

  // Initial seed from props; further updates come from local mutation.
  const [rows, setRows] = useState<Row[]>(() => rowsFromValue(value));
  // Track what we last emitted so we can ignore "incoming" prop updates
  // that are just our own commit echoing back through the parent.
  const lastEmittedRef = useRef<Record<string, string | number> | null>(null);

  // Re-seed from incoming value ONLY when the parent state diverged from
  // our last emitted commit (e.g. another tab edited config, or the user
  // hit Reset). Without this we'd clobber the user's mid-edit row state
  // every render. Without the value-equality check we'd re-seed on our
  // own echo and drop the focused input's row identity.
  useEffect(() => {
    if (lastEmittedRef.current && rowsMatchValue(rows, value)) return;
    if (!lastEmittedRef.current && rows.length === 0) {
      setRows(rowsFromValue(value));
      return;
    }
    if (!rowsMatchValue(rows, value)) {
      setRows(rowsFromValue(value));
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value]);

  const emit = useCallback((next: Row[]) => {
    setRows(next);
    const obj = commitRows(next);
    lastEmittedRef.current = obj;
    onChange(obj);
  }, [onChange]);

  const updateKey = useCallback((id: string, key: string) => {
    emit(rows.map((r) => r.id === id ? { ...r, key } : r));
  }, [rows, emit]);

  const updateValue = useCallback((id: string, raw: string) => {
    const parsed: string | number = valueType === "number"
      ? (raw === "" ? 0 : Number(raw))
      : raw;
    emit(rows.map((r) => r.id === id ? { ...r, value: parsed } : r));
  }, [rows, emit, valueType]);

  const removeRow = useCallback((id: string) => {
    emit(rows.filter((r) => r.id !== id));
  }, [rows, emit]);

  const addRow = useCallback(() => {
    emit([...rows, { id: newRowId(), key: "", value: valueType === "number" ? 0 : "" }]);
  }, [rows, emit, valueType]);

  const inputClass =
    "px-2.5 py-1.5 rounded-lg border border-border-subtle bg-main text-xs font-mono outline-none focus:border-brand transition-colors";

  return (
    <div className="flex flex-col gap-1.5">
      {rows.length === 0 && (
        <p className="text-[10px] text-text-dim italic">
          {t("config.map_empty", "No entries — click Add to create one")}
        </p>
      )}
      {rows.map((r) => (
        <div key={r.id} className="flex items-center gap-1.5">
          <input
            type="text"
            value={r.key}
            onChange={(e) => updateKey(r.id, e.target.value)}
            placeholder={keyPlaceholder ?? t("config.map_key", "key")}
            className={`${inputClass} flex-1 min-w-0`}
            autoComplete="off"
            spellCheck={false}
          />
          <span className="text-[10px] text-text-dim shrink-0">=</span>
          <input
            type={valueType === "number" ? "number" : "text"}
            value={String(r.value ?? "")}
            onChange={(e) => updateValue(r.id, e.target.value)}
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
            onClick={() => removeRow(r.id)}
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
