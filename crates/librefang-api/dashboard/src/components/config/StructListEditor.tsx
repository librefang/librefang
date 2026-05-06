import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ChevronDown, ChevronRight, Plus, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { type JsonSchema, resolveRef, type ConfigSchemaRoot } from "../../api";

// Editor for `Vec<Struct>` config fields. Each item is rendered as a
// collapsible card with a JSON editor for its fields. The collapsed header
// surfaces a "headline" derived from a heuristic key (provider / name / id /
// agent) so users can identify items at a glance.
//
// v1 deliberately uses a JSON editor per item rather than recursively
// rendering ConfigFieldInput, to avoid pulling that component out of
// ConfigPage. Replace with a structured per-field editor in a follow-up
// once the field renderer is extracted.

const HEADLINE_KEYS = ["name", "id", "provider", "agent", "channel_type", "url"] as const;

function headlineFor(item: unknown, fallbackIndex: number): string {
  if (item && typeof item === "object" && !Array.isArray(item)) {
    const obj = item as Record<string, unknown>;
    for (const key of HEADLINE_KEYS) {
      const v = obj[key];
      if (typeof v === "string" && v.trim()) return v;
    }
  }
  return `#${fallbackIndex + 1}`;
}

function defaultItemFor(itemSchema: JsonSchema | undefined, root: ConfigSchemaRoot | undefined): Record<string, unknown> {
  // Resolve the item schema if it's a $ref so we can read its `properties`.
  let target: JsonSchema | undefined = itemSchema;
  if (target?.$ref && root) target = resolveRef(root, target.$ref);
  const out: Record<string, unknown> = {};
  if (target?.properties) {
    for (const [k, node] of Object.entries(target.properties)) {
      if (node.default !== undefined) {
        out[k] = node.default;
      } else if (Array.isArray(node.type) ? node.type.includes("null") : false) {
        out[k] = null;
      } else if (node.type === "boolean") {
        out[k] = false;
      } else if (node.type === "integer" || node.type === "number") {
        out[k] = 0;
      } else if (node.type === "array") {
        out[k] = [];
      } else if (node.type === "object") {
        out[k] = {};
      } else {
        out[k] = "";
      }
    }
  }
  return out;
}

type Props = {
  value: unknown[] | null | undefined;
  onChange: (next: unknown[]) => void;
  itemSchema?: JsonSchema;
  schemaRoot?: ConfigSchemaRoot;
};

export function StructListEditor({ value, onChange, itemSchema, schemaRoot }: Props) {
  const { t } = useTranslation();
  const items = useMemo(() => Array.isArray(value) ? value : [], [value]);

  const removeItem = useCallback((idx: number) => {
    onChange(items.filter((_, i) => i !== idx));
  }, [items, onChange]);

  const updateItem = useCallback((idx: number, next: unknown) => {
    onChange(items.map((item, i) => i === idx ? next : item));
  }, [items, onChange]);

  const addItem = useCallback(() => {
    onChange([...items, defaultItemFor(itemSchema, schemaRoot)]);
  }, [items, onChange, itemSchema, schemaRoot]);

  return (
    <div className="flex flex-col gap-1.5">
      {items.length === 0 && (
        <p className="text-[10px] text-text-dim italic">
          {t("config.list_empty", "No items — click Add to create one")}
        </p>
      )}
      {items.map((item, idx) => (
        <StructListRow
          key={idx}
          headline={headlineFor(item, idx)}
          item={item}
          onChange={(next) => updateItem(idx, next)}
          onRemove={() => removeItem(idx)}
        />
      ))}
      <button
        type="button"
        onClick={addItem}
        className="flex items-center gap-1 self-start px-2 py-1 rounded-md text-[10px] text-text-dim hover:text-brand hover:bg-surface-hover transition-colors"
      >
        <Plus className="w-3 h-3" />
        {t("config.add_item", "Add")}
      </button>
    </div>
  );
}

function StructListRow({
  headline, item, onChange, onRemove,
}: {
  headline: string;
  item: unknown;
  onChange: (next: unknown) => void;
  onRemove: () => void;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [text, setText] = useState(() => JSON.stringify(item, null, 2));
  const [error, setError] = useState<string | null>(null);
  const lastEmittedRef = useRef<string>(text);

  // Keep the textarea in sync with external value updates (e.g. when a
  // sibling row is removed and `item` shifts) without clobbering the
  // user's in-progress edits.
  useEffect(() => {
    const incoming = JSON.stringify(item, null, 2);
    if (incoming === lastEmittedRef.current) return;
    setText(incoming);
    lastEmittedRef.current = incoming;
    setError(null);
  }, [item]);

  const handleChange = useCallback((raw: string) => {
    setText(raw);
    if (raw.trim() === "") {
      setError(null);
      onChange({});
      lastEmittedRef.current = raw;
      return;
    }
    try {
      const parsed = JSON.parse(raw);
      setError(null);
      onChange(parsed);
      lastEmittedRef.current = raw;
    } catch (e) {
      setError((e as Error).message);
    }
  }, [onChange]);

  return (
    <div className="rounded-lg border border-border-subtle bg-main">
      <div className="flex items-center gap-2 px-2.5 py-1.5">
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          className="text-text-dim hover:text-text shrink-0"
          aria-label={open ? t("config.collapse", "Collapse") : t("config.expand", "Expand")}
        >
          {open ? <ChevronDown className="w-3 h-3" /> : <ChevronRight className="w-3 h-3" />}
        </button>
        <span className="text-xs font-mono text-text truncate flex-1">{headline}</span>
        <button
          type="button"
          onClick={onRemove}
          className="p-1 rounded-md text-text-dim hover:text-danger hover:bg-surface-hover transition-colors shrink-0"
          title={t("config.remove_item", "Remove")}
          aria-label={t("config.remove_item", "Remove")}
        >
          <Trash2 className="w-3 h-3" />
        </button>
      </div>
      {open && (
        <div className="px-2.5 pb-2 pt-0 flex flex-col gap-1">
          <textarea
            value={text}
            onChange={(e) => handleChange(e.target.value)}
            rows={Math.min(Math.max(text.split("\n").length, 4), 20)}
            spellCheck={false}
            className={`w-full px-2.5 py-1.5 rounded-lg border bg-main text-[11px] font-mono outline-none resize-y transition-colors ${
              error ? "border-danger" : "border-border-subtle focus:border-brand"
            }`}
          />
          {error && <p className="text-[10px] text-danger">{error}</p>}
        </div>
      )}
    </div>
  );
}
