import { useTranslation } from "react-i18next";
import { useState, useCallback, useEffect, useMemo, useRef } from "react";
import { Link, useBlocker } from "@tanstack/react-router";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import {
  RefreshCw, Save, Zap, Settings, Search, RotateCcw,
  AlertTriangle, X, Copy, Check, FileText,
} from "lucide-react";
import {
  type ConfigSchemaRoot,
  type ConfigSectionDescriptor,
  type JsonSchema,
  type UiFieldOptions,
  resolveRef,
} from "../api";
import {
  useConfigSchema,
  useFullConfig,
  useRawConfigToml,
} from "../lib/queries/config";
import {
  useBatchSetConfigValues,
  useSetConfigValue,
  useReloadConfig,
} from "../lib/mutations/config";
import { TomlViewer } from "../components/TomlViewer";
import { StringMapEditor } from "../components/config/StringMapEditor";
import { StructListEditor } from "../components/config/StructListEditor";

/* ------------------------------------------------------------------ */
/*  Category → sections mapping                                        */
/* ------------------------------------------------------------------ */

const CATEGORY_SECTIONS: Record<string, string[]> = {
  general: [
    "general", "default_model", "thinking", "budget", "reload", "llm",
    "fallback_providers", "provider_urls", "provider_regions",
    "provider_proxy_urls", "provider_request_timeout_secs",
    "vertex_ai", "azure_openai", "proxy",
  ],
  memory: ["memory", "proactive_memory", "auto_dream"],
  tools: [
    "web", "browser", "links", "media", "tts", "canvas",
    "tool_invoke", "parallel_tools", "tool_results", "tool_policy",
    "tool_timeouts", "skills", "plugins",
  ],
  channels: [
    "channels", "broadcast", "auto_reply",
    "sidecar_channels", "inbox", "webhook_triggers", "triggers",
  ],
  security: [
    "approval", "exec_policy", "vault", "oauth", "external_auth", "terminal",
    "sanitize", "privacy", "taint_rules", "audit",
  ],
  network: ["network", "a2a", "pairing"],
  infra: [
    "docker", "extensions", "session", "queue",
    "compaction", "registry", "context_engine", "prompt_intelligence",
    "task_board", "rate_limit", "health_check", "heartbeat",
    "telemetry", "notification",
  ],
};

// Top-level category navigation order. Drives the tab strip at the top of
// ConfigPage so the user can move between categories without leaving the
// page. Closes #4678 — sidebar previously had a single Config link landing
// on /config/general with no way to reach Tools / Security / Memory / etc.
const CATEGORIES_ORDER: ReadonlyArray<string> = [
  "general", "memory", "tools", "channels", "security", "network", "infra",
];

// Explicit field ordering for sections where the server-side JSON schema
// ordering is wrong for the user. `KernelConfig`'s `default_model` sub-struct
// declares fields alphabetically via serde, so the user sees `model` before
// `provider` even though model cascades from provider. The rendering code
// falls back to the declared order for any section not listed here, so
// forgetting an entry is a no-op rather than a regression. Closes #2746.
const SECTION_FIELD_ORDER: Record<string, string[]> = {
  default_model: ["provider", "model", "api_key_env", "base_url"],
};

function sectionLabelFallback(key: string): string {
  return key.split("_").map((w) => w.charAt(0).toUpperCase() + w.slice(1)).join(" ");
}

const ACRONYM_MAP: Record<string, string> = {
  Api: "API", Url: "URL", Sql: "SQL", Ssl: "SSL", Tls: "TLS",
  Ttl: "TTL", Env: "Env Var", Id: "ID", Usd: "USD", Llm: "LLM",
  Mdns: "mDNS", Totp: "TOTP",
};

function fieldLabelFallback(key: string): string {
  return key
    .replace(/_/g, " ")
    .replace(/\b\w/g, (c) => c.toUpperCase())
    .replace(/\b\w+\b/g, (w) => ACRONYM_MAP[w] ?? w);
}

/* ------------------------------------------------------------------ */
/*  Draft-07 → UI type/options resolution                              */
/* ------------------------------------------------------------------ */

/** What the UI needs to render one field. Derived fresh from the schema each
 *  render — no persistent view-model. */
type FieldRender = {
  type: string;
  options?: UiFieldOptions["select"] | UiFieldOptions["select_objects"] | UiFieldOptions["number_select"];
  min?: number;
  max?: number;
  step?: number;
  // For "struct_list" fields — the JSON Schema of one item, used to seed
  // sensible defaults when the user clicks "Add". Only set when type is
  // "struct_list" (Vec<Struct>) and the items shape is resolvable.
  itemSchema?: JsonSchema;
};

function pickType(node: JsonSchema): string {
  // Arrayed `type: [..., "null"]` — skip "null" variant.
  const raw = Array.isArray(node.type) ? node.type.find((t) => t !== "null") ?? node.type[0] : node.type;
  if (typeof raw === "string") return raw;
  // No concrete type AND no union shape → unknown schema construct.
  // Warn once so unexpected shapes surface during dev rather than silently
  // rendering as a text input.
  if (!node.anyOf && !node.oneOf && !node.$ref && !Array.isArray(node.enum)) {
    // eslint-disable-next-line no-console
    console.warn("[ConfigPage] schema node missing 'type'; defaulting to string", node);
  }
  return "string";
}

/** Unwrap schemars wrapper shapes and return the concrete schema branch:
 *
 *  - `Option<T>` → `{anyOf|oneOf: [{$ref|…}, {type: "null"}]}` — pick non-null.
 *  - Struct with default + description → `{description, default, allOf: [{$ref}]}`
 *    — pick the single allOf branch (this is what schemars emits for required
 *    struct fields carrying metadata the ref target doesn't have).
 *
 *  Called before reading `type` / `properties` / `items` on a field node. */
function unwrapNullable(node: JsonSchema): JsonSchema {
  // allOf pattern: metadata-wrapped single ref. No null branch to pick;
  // just unwrap the first entry.
  if (Array.isArray(node.allOf) && node.allOf.length > 0) {
    return node.allOf[0];
  }
  const branches = node.anyOf ?? node.oneOf;
  if (!Array.isArray(branches)) return node;
  const nonNull = branches.find((b) => b.type !== "null" && b.$ref !== undefined) ??
    branches.find((b) => b.type !== "null");
  return nonNull ?? node;
}

function resolveFieldRender(node: JsonSchema, ui?: UiFieldOptions): FieldRender {
  // 1. UI overlay wins when it specifies a select-family type.
  if (ui?.select) return { type: "select", options: ui.select };
  if (ui?.select_objects) return { type: "select", options: ui.select_objects };
  if (ui?.number_select) return { type: "number_select", options: ui.number_select };

  // 2. Native Rust enum → schema `enum` array → select.
  if (Array.isArray(node.enum) && node.enum.length > 0) {
    return { type: "select", options: node.enum.map(String) };
  }

  // 3. Unwrap Option<T> / nullable shapes so we see the real type below.
  const effective = unwrapNullable(node);
  // Bare `{$ref: ...}` with no type sibling means a direct reference to
  // another struct. Today every config field wraps such refs in
  // Option<>/OneOrMany<> so this branch is latent, but a future bare
  // struct field should render as a JSON editor, not a text input.
  if (effective.$ref && !effective.type) return { type: "object" };
  // `serde_json::Value` fields (hook input/output schemas, tool
  // input_schemas) emit as `{description, default}` with no type because
  // Value can be any JSON. Render as JsonEditor so users can author the
  // arbitrary JSON payload, not a text input that'd corrupt the shape.
  if (!effective.type && !Array.isArray(effective.enum) && !effective.anyOf
      && !effective.oneOf && !effective.allOf && !effective.$ref) {
    return { type: "object" };
  }
  const primary = pickType(effective);
  if (primary === "boolean") return { type: "boolean" };
  if (primary === "integer" || primary === "number") {
    return {
      type: "number",
      min: ui?.min ?? effective.minimum ?? node.minimum,
      max: ui?.max ?? effective.maximum ?? node.maximum,
      step: ui?.step ?? effective.multipleOf ?? node.multipleOf,
    };
  }
  if (primary === "array") {
    const itemType = effective.items?.type;
    if (itemType === "string") return { type: "string[]" };
    // Array of structs (items has $ref or object type, e.g. OneOrMany<TelegramConfig>)
    // must render as a JSON editor, not a comma-separated string input.
    if (itemType === "object" || effective.items?.$ref) return { type: "object" };
    return { type: "array" };
  }
  if (primary === "object") return { type: "object" };
  return { type: "string" };
}

/** Resolve a section descriptor to the concrete property map the UI renders.
 *  Returns the ordered list of `[fieldKey, FieldRender]` plus the per-field
 *  JSON-pointer path for `x-ui-options` lookup. */
function resolveSectionFields(
  root: ConfigSchemaRoot,
  desc: ConfigSectionDescriptor,
): Array<[string, FieldRender]> {
  const uiOptions = root["x-ui-options"] ?? {};
  const entries: Array<[string, FieldRender]> = [];

  if (desc.root_level && desc.fields) {
    // Root-level fields: read declared order from the descriptor so the UI
    // follows intent, not the struct's serde ordering.
    for (const fieldKey of desc.fields) {
      const node = root.properties?.[fieldKey];
      if (!node) continue;
      const ui = uiOptions[`/${fieldKey}`];
      entries.push([fieldKey, resolveFieldRender(node, ui)]);
    }
    return entries;
  }

  if (!desc.struct_field) return [];
  let target: JsonSchema | undefined = root.properties?.[desc.struct_field];
  // schemars wraps struct fields in several shapes depending on whether
  // they're optional and whether they carry metadata (default/description):
  //   Option<T>          → {anyOf|oneOf: [{$ref}, {type: "null"}]}
  //   T with metadata    → {description, default, allOf: [{$ref}]}
  //   T without metadata → bare {$ref}
  // Peel any of these wrappers so the real sub-struct's properties are
  // visible to the section enumerator. Without this, ~52 of the ~60
  // top-level struct fields (those schemars emits as allOf-wrapped) would
  // render as empty sections in the UI.
  if (target && !target.$ref) {
    if (Array.isArray(target.allOf) && target.allOf.length > 0) {
      target = target.allOf[0];
    } else if (Array.isArray(target.anyOf)) {
      const nonNull = target.anyOf.find((a) => a.$ref || (a.type && a.type !== "null"));
      if (nonNull) target = nonNull;
    } else if (Array.isArray(target.oneOf)) {
      const nonNull = target.oneOf.find((a) => a.$ref || (a.type && a.type !== "null"));
      if (nonNull) target = nonNull;
    }
  }
  if (target?.$ref) target = resolveRef(root, target.$ref);

  // Collection-typed sections (BTreeMap / HashMap / Vec<Struct>) — emit a
  // single synthetic "field" whose key matches the section key so the
  // section renders as one editor instead of a property grid. Path
  // computation in the renderer treats this as root-level so the value is
  // posted to /api/config/set under the section key directly.
  if (target?.type === "object" && target.additionalProperties &&
      typeof target.additionalProperties === "object") {
    const apType = target.additionalProperties.type;
    const valueType: "number_map" | "string_map" =
      apType === "integer" || apType === "number" ? "number_map" : "string_map";
    return [[desc.key, { type: valueType }]];
  }
  if (target?.type === "array" && target.items) {
    const items = target.items;
    if (items.$ref || items.type === "object") {
      return [[desc.key, { type: "struct_list", itemSchema: items }]];
    }
    // Vec<String> as a whole-section field (no shipping cases today; falls
    // through to the existing string[] treatment if the section happens to
    // wrap a Vec<String>).
  }

  if (!target?.properties) return [];

  const declared = Object.entries(target.properties);
  const order = SECTION_FIELD_ORDER[desc.key];
  const ordered = order
    ? [
        ...order.map((k) => declared.find(([fk]) => fk === k)).filter((e): e is [string, JsonSchema] => !!e),
        ...declared.filter(([fk]) => !order.includes(fk)),
      ]
    : declared;

  for (const [fieldKey, node] of ordered) {
    const ui = uiOptions[`/${desc.struct_field}/${fieldKey}`];
    entries.push([fieldKey, resolveFieldRender(node, ui)]);
  }
  return entries;
}

function getNestedValue(
  obj: Record<string, unknown>,
  section: string,
  field: string,
  rootLevel?: boolean,
): unknown {
  if (rootLevel) return obj[field];
  const sec = obj[section] as Record<string, unknown> | undefined;
  return sec?.[field];
}

/* ------------------------------------------------------------------ */
/*  Highlight matching text in search results                          */
/* ------------------------------------------------------------------ */

function Highlight({ text, query }: { text: string; query: string }) {
  if (!query) return <>{text}</>;
  const idx = text.toLowerCase().indexOf(query.toLowerCase());
  if (idx === -1) return <>{text}</>;
  return (
    <>
      {text.slice(0, idx)}
      <mark className="bg-primary/20 text-primary rounded-sm not-italic">{text.slice(idx, idx + query.length)}</mark>
      {text.slice(idx + query.length)}
    </>
  );
}

/* ------------------------------------------------------------------ */
/*  Field type badge                                                   */
/* ------------------------------------------------------------------ */

const TYPE_COLORS: Record<string, string> = {
  boolean: "text-blue-500 bg-blue-500/10",
  number:  "text-purple-500 bg-purple-500/10",
  select:  "text-amber-500 bg-amber-500/10",
  number_select: "text-amber-500 bg-amber-500/10",
  array:   "text-teal-500 bg-teal-500/10",
  "string[]": "text-teal-500 bg-teal-500/10",
  object:  "text-orange-500 bg-orange-500/10",
  string_map: "text-pink-500 bg-pink-500/10",
  number_map: "text-pink-500 bg-pink-500/10",
  struct_list: "text-indigo-500 bg-indigo-500/10",
  string:  "text-text-dim bg-border-subtle/50",
};

function FieldTypeBadge({ type }: { type: string }) {
  const cls = TYPE_COLORS[type] ?? TYPE_COLORS.string;
  return (
    <span className={`inline-block text-[9px] font-mono px-1 rounded leading-4 ${cls}`}>
      {type}
    </span>
  );
}

/* ------------------------------------------------------------------ */
/*  Copy path button                                                   */
/* ------------------------------------------------------------------ */

function CopyPathButton({ path }: { path: string }) {
  const [copied, setCopied] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const handleCopy = useCallback(() => {
    navigator.clipboard.writeText(path).then(() => {
      setCopied(true);
      if (timerRef.current) clearTimeout(timerRef.current);
      timerRef.current = setTimeout(() => setCopied(false), 1500);
    });
  }, [path]);

  useEffect(() => () => { if (timerRef.current) clearTimeout(timerRef.current); }, []);

  return (
    <button
      onClick={handleCopy}
      className="p-1 rounded-md text-text-dim/50 hover:text-text-dim hover:bg-surface-hover transition-colors"
      title={`Copy path: ${path}`}
    >
      {copied ? <Check className="w-2.5 h-2.5 text-success" /> : <Copy className="w-2.5 h-2.5" />}
    </button>
  );
}

/* ------------------------------------------------------------------ */
/*  Field input                                                        */
/* ------------------------------------------------------------------ */

function JsonEditor({ value, onChange }: { value: unknown; onChange: (v: unknown) => void }) {
  const [text, setText] = useState(() => value != null ? JSON.stringify(value, null, 2) : "");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const incoming = value != null ? JSON.stringify(value, null, 2) : "";
    setText((prev) => {
      try { if (JSON.stringify(JSON.parse(prev), null, 2) === incoming) return prev; } catch { /* empty */ }
      return incoming;
    });
  }, [value]);

  const handleChange = useCallback((e: React.ChangeEvent<HTMLTextAreaElement>) => {
    const raw = e.target.value;
    setText(raw);
    if (raw.trim() === "" || raw.trim() === "{}" || raw.trim() === "[]") {
      setError(null);
      onChange(raw.trim() === "" ? null : JSON.parse(raw.trim()));
      return;
    }
    try {
      const parsed = JSON.parse(raw);
      setError(null);
      onChange(parsed);
    } catch {
      setError("Invalid JSON");
    }
  }, [onChange]);

  return (
    <div className="flex flex-col gap-1">
      <textarea
        value={text}
        onChange={handleChange}
        rows={Math.min(Math.max(text.split("\n").length, 3), 12)}
        spellCheck={false}
        className={`w-full px-3 py-2 rounded-xl border bg-main text-[11px] font-mono outline-none transition-colors resize-y ${
          error ? "border-danger" : "border-border-subtle focus:border-primary"
        }`}
      />
      {error && <p className="text-[10px] text-danger">{error}</p>}
    </div>
  );
}

const SENSITIVE_PATTERNS = /api_key|secret|password|token_env|client_secret|credentials/i;

type SelectOption = string | { id: string; name: string; provider: string } | { value: string; label: string };

function ConfigFieldInput({
  fieldKey, fieldType, options, min, max, step, value, onChange,
  itemSchema, schemaRoot,
}: {
  fieldKey: string;
  fieldType: string;
  options?: SelectOption[];
  min?: number;
  max?: number;
  step?: number;
  value: unknown;
  onChange: (v: unknown) => void;
  // Only used by `struct_list` to seed sensible defaults when the user
  // clicks "Add" on a Vec<Struct> section. Optional otherwise.
  itemSchema?: JsonSchema;
  schemaRoot?: ConfigSchemaRoot;
}) {
  const { t } = useTranslation();
  const inputClass =
    "w-full px-3 py-1.5 rounded-xl border border-border-subtle bg-main text-xs font-mono outline-none focus:border-primary transition-colors";

  if (fieldType === "boolean") {
    return (
      <div className="flex items-center h-[30px]">
        <button
          onClick={() => onChange(!value)}
          className={`relative w-10 h-5 rounded-full transition-colors ${value ? "bg-primary" : "bg-border-subtle"}`}
        >
          <span className={`absolute top-0.5 w-4 h-4 rounded-full bg-white shadow transition-transform ${value ? "left-5" : "left-0.5"}`} />
        </button>
      </div>
    );
  }

  if ((fieldType === "select" || fieldType === "number_select") && options) {
    const normalizedOptions = options.map((o: SelectOption) => {
      if (typeof o === "string") return { value: o, label: t(`config.${fieldKey}_${o}`, o) };
      if ("value" in o && "label" in o) return { value: String(o.value), label: String(o.label) };
      if ("id" in o) return { value: o.id, label: o.name ?? o.id };
      return { value: String(o), label: String(o) };
    });
    const rawValue = String(value ?? "");
    const matched = normalizedOptions.find((o) => o.value.toLowerCase() === rawValue.toLowerCase())?.value ?? rawValue;
    const handleChange = (v: string) => {
      if (fieldType === "number_select") {
        const n = Number(v);
        onChange(Number.isNaN(n) ? v : n);
      } else {
        onChange(v);
      }
    };
    return (
      <select value={matched} onChange={(e) => handleChange(e.target.value)} className={inputClass}>
        {matched && !normalizedOptions.some((o) => o.value === matched) && <option value={matched}>{matched}</option>}
        {normalizedOptions.map((o) => <option key={o.value} value={o.value}>{o.label}</option>)}
      </select>
    );
  }

  if (fieldType === "number") {
    return (
      <input type="number" value={value != null ? String(value) : ""}
        onChange={(e) => {
          const v = e.target.value;
          if (v === "") { onChange(null); return; }
          const n = Number(v);
          if (!Number.isNaN(n)) onChange(n);
        }}
        min={min} max={max} step={step}
        className={inputClass} />
    );
  }

  if (fieldType === "string[]" || fieldType === "array") {
    const arr = Array.isArray(value) ? value : [];
    return (
      <input type="text" value={arr.join(", ")}
        onChange={(e) => onChange(e.target.value.split(",").map((s) => s.trim()).filter(Boolean))}
        placeholder="comma-separated values" className={inputClass} />
    );
  }

  if (fieldType === "object") {
    return <JsonEditor value={value} onChange={onChange} />;
  }

  if (fieldType === "string_map") {
    return (
      <StringMapEditor
        value={value as Record<string, string> | null | undefined}
        onChange={onChange}
        valueType="string"
      />
    );
  }

  if (fieldType === "number_map") {
    return (
      <StringMapEditor
        value={value as Record<string, number> | null | undefined}
        onChange={onChange}
        valueType="number"
        min={min}
        max={max}
        step={step}
      />
    );
  }

  if (fieldType === "struct_list") {
    return (
      <StructListEditor
        value={Array.isArray(value) ? value : []}
        onChange={onChange}
        itemSchema={itemSchema}
        schemaRoot={schemaRoot}
      />
    );
  }

  const isSensitive = fieldType === "string" && SENSITIVE_PATTERNS.test(fieldKey);

  return (
    <input type={isSensitive ? "password" : "text"} value={String(value ?? "")}
      onChange={(e) => onChange(e.target.value || null)} className={inputClass}
      autoComplete={isSensitive ? "off" : undefined} />
  );
}

/* ------------------------------------------------------------------ */
/*  Page component — one per category                                  */
/* ------------------------------------------------------------------ */

export function ConfigPage({ category }: { category: string }) {
  const { t } = useTranslation();

  const schemaQuery = useConfigSchema();
  const configQuery = useFullConfig();

  const [pendingChanges, setPendingChanges] = useState<Record<string, unknown>>({});
  const [saveStatus, setSaveStatus] = useState<Record<string, { ok: boolean; msg: string }>>({});
  const [searchQuery, setSearchQuery] = useState("");
  const [reloadStatus, setReloadStatus] = useState<{ ok: boolean; msg: string } | null>(null);
  const [activeSection, setActiveSection] = useState<string | null>(null);
  const [showRawToml, setShowRawToml] = useState(false);
  const searchRef = useRef<HTMLInputElement>(null);
  const saveStatusTimeoutRefs = useRef<Record<string, ReturnType<typeof setTimeout>>>({});
  const rawTomlQuery = useRawConfigToml(showRawToml);

  const hasPendingChanges = Object.keys(pendingChanges).length > 0;

  // ── Index section descriptors by key and resolve field lists. ──
  const schemaRoot: ConfigSchemaRoot | undefined = schemaQuery.data;
  const sectionsByKey = useMemo<Record<string, ConfigSectionDescriptor>>(() => {
    const out: Record<string, ConfigSectionDescriptor> = {};
    for (const desc of schemaRoot?.["x-sections"] ?? []) {
      out[desc.key] = desc;
    }
    return out;
  }, [schemaRoot]);

  const resolvedFields = useMemo<Record<string, Array<[string, FieldRender]>>>(() => {
    const out: Record<string, Array<[string, FieldRender]>> = {};
    if (!schemaRoot) return out;
    for (const desc of schemaRoot["x-sections"] ?? []) {
      out[desc.key] = resolveSectionFields(schemaRoot, desc);
    }
    return out;
  }, [schemaRoot]);

  // ── Keyboard shortcuts: / to focus search, Esc to clear ────
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement;
      const inInput = target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.tagName === "SELECT";
      if (e.key === "/" && !inInput) {
        e.preventDefault();
        searchRef.current?.focus();
      }
      if (e.key === "Escape" && inInput && target === searchRef.current) {
        setSearchQuery("");
        searchRef.current?.blur();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);

  useEffect(() => {
    if (!hasPendingChanges) return;
    const handler = (e: BeforeUnloadEvent) => { e.preventDefault(); };
    window.addEventListener("beforeunload", handler);
    return () => window.removeEventListener("beforeunload", handler);
  }, [hasPendingChanges]);

  const blocker = useBlocker({
    shouldBlockFn: () => hasPendingChanges,
    withResolver: true,
  });
  const blockerRef = useRef(blocker);
  blockerRef.current = blocker;

  useEffect(() => {
    if (blockerRef.current.status !== "blocked") return;
    if (window.confirm(t("config.unsaved_warning", "You have unsaved changes. Discard them?"))) {
      setPendingChanges({});
      blockerRef.current.proceed();
    } else {
      blockerRef.current.reset();
    }
  }, [blocker.status, t]);

  const handleFieldChange = useCallback(
    (sectionKey: string, fieldKey: string, value: unknown, rootLevel?: boolean) => {
      const path = rootLevel ? fieldKey : `${sectionKey}.${fieldKey}`;
      setPendingChanges((p) => {
        const next = { ...p, [path]: value };
        // Cascading: when provider changes in default_model, clear model if
        // the current selection doesn't belong to the new provider. Pulls
        // the model catalog straight from the x-ui-options overlay.
        //
        // If the catalog hasn't loaded yet (schema query still pending) we
        // can't tell whether the existing model is still valid, so we
        // conservatively clear it — better to ask the user to re-pick than
        // to leave a stale (provider, model) pair that fails at the LLM
        // call site (round-4 review of #4678).
        if (sectionKey === "default_model" && fieldKey === "provider" && value) {
          const modelPath = "default_model.model";
          const currentModel = modelPath in p ? p[modelPath] : getNestedValue(configQuery.data ?? {}, "default_model", "model");
          if (currentModel) {
            const modelOptions = schemaRoot?.["x-ui-options"]?.["/default_model/model"]?.select_objects;
            const modelBelongsToProvider = modelOptions
              ? modelOptions.some(
                  (m) => m.id === String(currentModel) && m.provider === String(value),
                )
              : false;
            if (!modelBelongsToProvider) {
              next[modelPath] = null;
            }
          }
        }
        return next;
      });
    },
    [configQuery.data, schemaRoot]
  );

  const saveMutation = useSetConfigValue({
    onSuccess: (data, variables) => {
      const reloadFailed = data.status === "saved_reload_failed";
      const restartRequired = data.status === "applied_partial" || data.restart_required;
      if (reloadFailed) {
        setSaveStatus((s) => ({ ...s, [variables.path]: { ok: false, msg: t("config.saved_reload_failed", "Saved but reload failed") } }));
      } else {
        const msg = restartRequired ? t("config.saved_restart", "Saved (restart required)") : t("common.saved", "Saved");
        setSaveStatus((s) => ({ ...s, [variables.path]: { ok: true, msg } }));
      }
      setPendingChanges((p) => {
        if (!(variables.path in p) || JSON.stringify(p[variables.path]) === JSON.stringify(variables.value)) {
          const next = { ...p }; delete next[variables.path]; return next;
        }
        return p;
      });
      const existing = saveStatusTimeoutRefs.current[variables.path];
      if (existing) clearTimeout(existing);
      saveStatusTimeoutRefs.current[variables.path] = setTimeout(() => {
        setSaveStatus((s) => { const next = { ...s }; delete next[variables.path]; return next; });
        delete saveStatusTimeoutRefs.current[variables.path];
      }, 3000);
    },
    onError: (err: Error, variables) => {
      setSaveStatus((s) => ({ ...s, [variables.path]: { ok: false, msg: err.message } }));
      const existing = saveStatusTimeoutRefs.current[variables.path];
      if (existing) clearTimeout(existing);
      saveStatusTimeoutRefs.current[variables.path] = setTimeout(() => {
        setSaveStatus((s) => { const next = { ...s }; delete next[variables.path]; return next; });
        delete saveStatusTimeoutRefs.current[variables.path];
      }, 3000);
    },
  });

  const [batchSaving, setBatchSaving] = useState(false);
  const batchStatusTimeoutRefs = useRef<Record<string, ReturnType<typeof setTimeout>>>({});
  const clearBatchStatuses = useCallback((paths: string[], statuses: Record<string, { ok: boolean; msg: string }>, delay: number) => {
    for (const path of paths) {
      const existingTimeout = batchStatusTimeoutRefs.current[path];
      if (existingTimeout) clearTimeout(existingTimeout);
      batchStatusTimeoutRefs.current[path] = setTimeout(() => {
        setSaveStatus((current) => {
          if (current[path]?.msg !== statuses[path]?.msg) return current;
          const next = { ...current };
          delete next[path];
          return next;
        });
        delete batchStatusTimeoutRefs.current[path];
      }, delay);
    }
  }, []);

  useEffect(() => () => {
    for (const timeoutId of Object.values(batchStatusTimeoutRefs.current)) {
      clearTimeout(timeoutId);
    }
    for (const timeoutId of Object.values(saveStatusTimeoutRefs.current)) {
      clearTimeout(timeoutId);
    }
  }, []);

  const batchSaveMutation = useBatchSetConfigValues({
    onSuccess: (results) => {
      let errors = 0;
      const nextStatuses: Record<string, { ok: boolean; msg: string }> = {};
      for (const result of results) {
        if (result.error) {
          nextStatuses[result.path] = {
            ok: false,
            msg: result.error?.message || t("config.save_failed"),
          };
          errors++;
          continue;
        }

        const reloadFailed = result.data?.status === "saved_reload_failed";
        const restartRequired = result.data?.status === "applied_partial" || result.data?.restart_required;
        const reloadErr = reloadFailed && result.data?.reload_error ? result.data.reload_error : null;
        const msg = reloadFailed
          ? reloadErr
            ? `${t("config.saved_reload_failed", "Saved but reload failed")}: ${reloadErr}`
            : t("config.saved_reload_failed", "Saved but reload failed")
          : restartRequired
            ? t("config.saved_restart", "Saved (restart required)")
            : t("common.saved", "Saved");
        nextStatuses[result.path] = { ok: !reloadFailed, msg };
        if (reloadFailed) errors++;
      }

      setSaveStatus((current) => ({ ...current, ...nextStatuses }));

      setPendingChanges((current) => {
        const next = { ...current };
        for (const result of results) {
          if (!result.error && JSON.stringify(current[result.path]) === JSON.stringify(result.value)) {
            delete next[result.path];
          }
        }
        return next;
      });
      setBatchSaving(false);
      clearBatchStatuses(results.map((result) => result.path), nextStatuses, errors > 0 ? 5000 : 3000);
    },
    onError: (err: Error) => {
      setBatchSaving(false);
      setSaveStatus((s) => ({
        ...s,
        __batch__: { ok: false, msg: err.message || t("config.save_failed") },
      }));
      const existing = batchStatusTimeoutRefs.current["__batch__"];
      if (existing) clearTimeout(existing);
      batchStatusTimeoutRefs.current["__batch__"] = setTimeout(() => {
        setSaveStatus((s) => {
          const next = { ...s };
          delete next.__batch__;
          return next;
        });
        delete batchStatusTimeoutRefs.current["__batch__"];
      }, 5000);
    },
  });
  const handleBatchSave = useCallback(async () => {
    const entries = Object.entries(pendingChanges);
    if (entries.length === 0) return;
    setBatchSaving(true);
    try {
      await batchSaveMutation.mutateAsync(entries.map(([path, value]) => ({ path, value })));
    } catch {
      // onError already mapped the batch-level failure into UI state.
    }
  }, [batchSaveMutation, pendingChanges]);

  const handleResetField = useCallback(
    (sectionKey: string, fieldKey: string, rootLevel?: boolean) => {
      const path = rootLevel ? fieldKey : `${sectionKey}.${fieldKey}`;
      setPendingChanges((p) => ({ ...p, [path]: null }));
    },
    []
  );

  const handleResetSection = useCallback(
    (sectionKey: string, fieldKeys: string[], rootLevel?: boolean) => {
      setPendingChanges((p) => {
        const next = { ...p };
        for (const fKey of fieldKeys) {
          const path = rootLevel ? fKey : `${sectionKey}.${fKey}`;
          next[path] = null;
        }
        return next;
      });
    },
    []
  );

  const reloadMutation = useReloadConfig({
    onSuccess: () => {
      setReloadStatus({ ok: true, msg: t("config.reload_success", "Config reloaded") });
    },
    onError: (err: Error) => {
      setReloadStatus({ ok: false, msg: err.message });
    },
  });

  useEffect(() => {
    if (reloadStatus) {
      const id = setTimeout(() => setReloadStatus(null), 3000);
      return () => clearTimeout(id);
    }
  }, [reloadStatus]);

  // ── Derived data ───────────────────────────────────────────────────
  const config = configQuery.data ?? {};
  const sectionKeys = (CATEGORY_SECTIONS[category] ?? []).filter((s) => s in sectionsByKey);
  const categoryTitle = t(`config.cat_${category}`, sectionLabelFallback(category));
  const q = searchQuery.toLowerCase();
  const isSearching = q.length > 0;

  const effectiveTab = isSearching
    ? null
    : (activeSection && sectionKeys.includes(activeSection) ? activeSection : sectionKeys[0] ?? null);

  // Which sections have pending changes (for tab dot indicators)
  const sectionsWithPending = useMemo(() => {
    const set = new Set<string>();
    const rootFieldMap = new Map<string, string>();
    for (const [sKey, desc] of Object.entries(sectionsByKey)) {
      if (desc.root_level) {
        for (const [fKey] of resolvedFields[sKey] ?? []) {
          rootFieldMap.set(fKey, sKey);
        }
      }
    }
    for (const path of Object.keys(pendingChanges)) {
      const dotIdx = path.indexOf(".");
      if (dotIdx !== -1) {
        set.add(path.slice(0, dotIdx));
      } else {
        const sKey = rootFieldMap.get(path);
        if (sKey) set.add(sKey);
      }
    }
    return set;
  }, [pendingChanges, sectionsByKey, resolvedFields]);

  const filteredSections = useMemo(() => {
    const keysToShow = effectiveTab ? [effectiveTab] : sectionKeys;
    if (!q) return keysToShow.map((sKey) => ({ sKey, fields: (resolvedFields[sKey] ?? []).map(([fk]) => fk) }));
    return keysToShow
      .map((sKey) => {
        if (!sectionsByKey[sKey]) return null;
        const sectionMatches = t(`config.sec_${sKey}`, sectionLabelFallback(sKey)).toLowerCase().includes(q) || sKey.includes(q);
        const matchedFields = (resolvedFields[sKey] ?? [])
          .map(([fk]) => fk)
          .filter((fKey) =>
            sectionMatches || fKey.includes(q) || t(`config.fld_${fKey}`, fieldLabelFallback(fKey)).toLowerCase().includes(q)
          );
        return matchedFields.length > 0 ? { sKey, fields: matchedFields } : null;
      })
      .filter((x): x is { sKey: string; fields: string[] } => x !== null);
  }, [sectionKeys, sectionsByKey, resolvedFields, q, effectiveTab, t]);

  // ── Loading / error states ─────────────────────────────────────────
  if (schemaQuery.isLoading || configQuery.isLoading) {
    return (
      <div className="flex flex-col gap-4 p-6 max-w-5xl">
        <div className="flex items-center gap-2.5">
          <Settings className="h-4 w-4 text-text-dim" />
          <span className="text-sm font-semibold">{categoryTitle}</span>
        </div>
        <div className="rounded-2xl border border-border-subtle bg-surface p-8 text-center text-text-dim text-sm">
          {t("common.loading", "Loading...")}
        </div>
      </div>
    );
  }

  if (schemaQuery.isError || configQuery.isError) {
    return (
      <div className="flex flex-col gap-4 p-6 max-w-5xl">
        <div className="flex items-center gap-2.5">
          <Settings className="h-4 w-4 text-text-dim" />
          <span className="text-sm font-semibold">{categoryTitle}</span>
        </div>
        <div className="rounded-2xl border border-danger/30 bg-surface p-8 text-center text-danger text-sm">
          {t("config.load_error", "Failed to load configuration")}
        </div>
      </div>
    );
  }

  // ── Render ─────────────────────────────────────────────────────────
  return (
    <div className="flex flex-col p-6 max-w-5xl gap-4 pb-24">

      {/* Row 1: title + reload */}
      <div className="flex items-center justify-between gap-4">
        <div className="flex items-center gap-2.5">
          <Settings className="h-4 w-4 text-text-dim shrink-0" />
          <div>
            <h1 className="text-sm font-bold leading-tight">{categoryTitle}</h1>
            <p className="text-[11px] text-text-dim leading-tight mt-0.5">{t("config.desc", "System configuration editor")}</p>
          </div>
        </div>
        <div className="flex items-center gap-2 shrink-0">
          {reloadStatus && (
            <span className={`text-xs font-semibold ${reloadStatus.ok ? "text-success" : "text-danger"}`}>
              {reloadStatus.msg}
            </span>
          )}
          <Button variant="secondary" size="sm" onClick={() => setShowRawToml(true)}>
            <FileText className="w-3 h-3 mr-1.5" />
            {t("config.view_raw_toml", "View Raw TOML")}
          </Button>
          <Button variant="secondary" size="sm" onClick={() => reloadMutation.mutate()} isLoading={reloadMutation.isPending}>
            <RefreshCw className="w-3 h-3 mr-1.5" />
            {t("config.reload", "Reload")}
          </Button>
        </div>
      </div>

      {/* Row 1.5: category tabs — all 7 categories reachable from any /config
          page. Closes #4678: previously the sidebar only had a single Config
          link landing on /config/general with no in-page way to reach Tools /
          Security / Memory / etc. */}
      <div className="flex items-center gap-0.5 -mx-6 px-6 overflow-x-auto border-b border-border-subtle">
        {CATEGORIES_ORDER.map((cat) => {
          const isActive = cat === category;
          return (
            <Link
              key={cat}
              // TanStack Router's typed `to` would require listing 7 literal
              // routes; cast keeps the call-site driven by CATEGORIES_ORDER
              // so adding a category is a one-line change.
              to={`/config/${cat}` as never}
              className={`px-3 py-2 text-xs font-semibold border-b-2 -mb-px transition-colors whitespace-nowrap ${
                isActive
                  ? "border-brand text-brand"
                  : "border-transparent text-text-dim hover:text-text hover:border-border-subtle"
              }`}
            >
              {t(`config.cat_${cat}`, sectionLabelFallback(cat))}
            </Link>
          );
        })}
      </div>

      {/* Row 2: section tabs (within current category) — always visible when >1 section; grayed/disabled during search */}
      {sectionKeys.length > 1 && (
        <div className="flex items-center border-b border-border-subtle -mx-6 px-6">
          {sectionKeys.map((sKey) => {
            const isActive = !isSearching && effectiveTab === sKey;
            const hasDot = sectionsWithPending.has(sKey);
            return (
              <button
                key={sKey}
                onClick={() => { setActiveSection(sKey); setSearchQuery(""); }}
                disabled={isSearching}
                className={`relative px-3 py-2 text-xs font-medium border-b-2 -mb-px transition-colors whitespace-nowrap flex items-center gap-1.5 ${
                  isActive
                    ? "border-primary text-primary"
                    : isSearching
                      ? "border-transparent text-text-dim/40 cursor-not-allowed"
                      : "border-transparent text-text-dim hover:text-text hover:border-border-subtle"
                }`}
              >
                {t(`config.sec_${sKey}`, sectionLabelFallback(sKey))}
                {hasDot && (
                  <span className="w-1.5 h-1.5 rounded-full bg-warning shrink-0" />
                )}
              </button>
            );
          })}
          {isSearching && (
            <span className="ml-auto text-[10px] text-text-dim pb-2 pr-1">
              {t("config.searching_all", "searching all sections")}
            </span>
          )}
        </div>
      )}

      {/* Row 3: search */}
      <div className="relative">
        <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-text-dim pointer-events-none" />
        <input
          ref={searchRef}
          type="text"
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
          placeholder={t("config.search_placeholder", "Search fields…  (/)")}
          className="w-full pl-9 pr-8 py-2 rounded-xl border border-border-subtle bg-surface text-xs outline-none focus:border-primary transition-colors"
        />
        {isSearching && (
          <button
            onClick={() => setSearchQuery("")}
            className="absolute right-2.5 top-1/2 -translate-y-1/2 text-text-dim hover:text-text transition-colors"
            aria-label="Clear search"
          >
            <X className="w-3.5 h-3.5" />
          </button>
        )}
      </div>

      {/* Sections */}
      <div className="flex flex-col gap-3">
        {filteredSections.length === 0 && (
          <div className="rounded-2xl border border-border-subtle bg-surface p-8 text-center text-text-dim text-sm">
            {t("config.no_results", "No fields match your search")}
          </div>
        )}
        {filteredSections.map(({ sKey, fields: visibleFields }) => {
          const desc = sectionsByKey[sKey];
          const allFields = resolvedFields[sKey] ?? [];
          const fieldsToShow = q
            ? allFields.filter(([fKey]) => visibleFields.includes(fKey))
            : allFields;

          const hasBadges = desc.hot_reloadable || desc.root_level;
          const showSectionHeader = isSearching || hasBadges;

          return (
            <div key={sKey} className="rounded-2xl border border-border-subtle bg-surface overflow-hidden">
              {showSectionHeader && (
                <div className="flex items-center gap-2 px-5 py-2.5 border-b border-border-subtle/50">
                  {isSearching && (
                    <span className="text-xs font-semibold text-text-dim">
                      {t(`config.sec_${sKey}`, sectionLabelFallback(sKey))}
                    </span>
                  )}
                  {desc.hot_reloadable && (
                    <Badge variant="success"><Zap className="w-2.5 h-2.5 mr-0.5" />{t("config.hot_reload", "Hot Reload")}</Badge>
                  )}
                  {desc.root_level && (
                    <Badge variant="info">{t("config.root_level", "Root Level")}</Badge>
                  )}
                  <div className="ml-auto flex items-center gap-2">
                    {isSearching && (
                      <span className="text-[10px] text-text-dim">
                        {fieldsToShow.length}/{allFields.length} {t("config.fields_unit")}
                      </span>
                    )}
                    {fieldsToShow.some(([fKey]) => {
                      const p = desc.root_level ? fKey : `${sKey}.${fKey}`;
                      return p in pendingChanges;
                    }) && (
                      <button
                        onClick={() => handleResetSection(sKey, fieldsToShow.map(([fKey]) => fKey), desc.root_level)}
                        className="text-[10px] text-text-dim hover:text-warning transition-colors flex items-center gap-1"
                        title={t("config.reset_section", "Reset section to defaults")}
                      >
                        <RotateCcw className="w-2.5 h-2.5" />
                        {t("config.reset_all", "Reset all")}
                      </button>
                    )}
                  </div>
                </div>
              )}
              <div className="divide-y divide-border-subtle/30">
                {fieldsToShow.map(([fieldKey, render]) => {
                  const { type: fieldType, options: rawOptions, min, max, step, itemSchema } = render;
                  // Collection-typed sections (string_map / number_map /
                  // struct_list) emit a single synthetic field whose key
                  // matches the section key. Treat them as root-level for
                  // path / value lookup so the editor reads from and posts
                  // to /<sectionKey>, not /<sectionKey>/<sectionKey>.
                  const isCollection =
                    fieldType === "string_map" ||
                    fieldType === "number_map" ||
                    fieldType === "struct_list";
                  const useRootSemantics = desc.root_level || isCollection;
                  const path = useRootSemantics ? fieldKey : `${sKey}.${fieldKey}`;
                  const currentValue = path in pendingChanges
                    ? pendingChanges[path]
                    : getNestedValue(config, sKey, fieldKey, useRootSemantics);
                  const hasPending = path in pendingChanges;
                  const isSaving = saveMutation.isPending && saveMutation.variables?.path === path;
                  const statusForField = saveStatus[path] ?? null;
                  const fieldDesc = t(`config.desc_${fieldKey}`, "");
                  const fieldLabel = t(`config.fld_${fieldKey}`, fieldLabelFallback(fieldKey));

                  // Cascading filter: when editing model in default_model, only show
                  // models matching the selected provider.
                  let options: SelectOption[] | undefined = rawOptions as SelectOption[] | undefined;
                  if (sKey === "default_model" && fieldKey === "model" && options) {
                    const providerPath = "default_model.provider";
                    const selectedProvider = providerPath in pendingChanges
                      ? String(pendingChanges[providerPath] ?? "")
                      : String(getNestedValue(config, "default_model", "provider") ?? "");
                    if (selectedProvider) {
                      options = options.filter((o: SelectOption) => {
                        if (typeof o === "object" && o !== null && "provider" in o) {
                          return o.provider === selectedProvider;
                        }
                        return true;
                      });
                    }
                  }

                  return (
                    <div key={fieldKey} className="flex items-start gap-4 px-5 py-3 group">
                      <div className="w-44 shrink-0 pt-1">
                        <p className="text-xs font-semibold leading-tight">
                          <Highlight text={fieldLabel} query={q} />
                        </p>
                        <div className="flex items-center gap-1 mt-0.5">
                          <p className="text-[10px] text-text-dim font-mono leading-tight">
                            <Highlight text={fieldKey} query={q} />
                          </p>
                          <CopyPathButton path={path} />
                        </div>
                        <div className="mt-0.5">
                          <FieldTypeBadge type={fieldType} />
                        </div>
                      </div>
                      <div className="flex-1 min-w-0 flex flex-col gap-1 pt-1">
                        <ConfigFieldInput
                          fieldKey={fieldKey}
                          fieldType={fieldType}
                          options={options}
                          min={min}
                          max={max}
                          step={step}
                          value={currentValue}
                          onChange={(v) => handleFieldChange(sKey, fieldKey, v, useRootSemantics)}
                          itemSchema={itemSchema}
                          schemaRoot={schemaRoot}
                        />
                        {fieldDesc && (
                          <p className="text-[10px] text-text-dim leading-relaxed">{fieldDesc}</p>
                        )}
                      </div>
                      <div className="w-24 shrink-0 flex items-center justify-end gap-1">
                        {statusForField ? (
                          <span
                            className={`text-[10px] font-semibold truncate ${statusForField.ok ? "text-success" : "text-danger"}`}
                            title={statusForField.msg}
                          >
                            {statusForField.msg}
                          </span>
                        ) : hasPending ? (
                          <>
                            <button
                              onClick={() => handleResetField(sKey, fieldKey, desc.root_level)}
                              className="p-1 rounded-md text-text-dim hover:text-warning hover:bg-surface-hover transition-colors"
                              title={t("config.reset_default", "Reset to default")}
                            >
                              <RotateCcw className="w-3 h-3" />
                            </button>
                            <Button
                              variant="primary"
                              size="sm"
                              onClick={() => {
                                if (path in pendingChanges) saveMutation.mutate({ path, value: pendingChanges[path] });
                              }}
                              isLoading={isSaving}
                              disabled={isSaving}
                            >
                              <Save className="w-3 h-3" />
                            </Button>
                          </>
                        ) : null}
                      </div>
                    </div>
                  );
                })}
              </div>
            </div>
          );
        })}
      </div>

      {/* Sticky unsaved changes bar */}
      {hasPendingChanges && (
        <div className="fixed bottom-0 left-0 right-0 z-40 flex justify-center pointer-events-none pb-safe">
          <div className="mb-5 flex items-center gap-3 px-4 py-2.5 rounded-2xl border border-warning/30 bg-surface shadow-lg pointer-events-auto">
            <AlertTriangle className="w-3.5 h-3.5 text-warning shrink-0" />
            <span className="text-xs font-semibold text-warning">
              {Object.keys(pendingChanges).length} {t("config.unsaved", "unsaved")} {t("config.changes", "changes")}
            </span>
            <div className="w-px h-4 bg-border-subtle" />
            <Button variant="ghost" size="sm" onClick={() => setPendingChanges({})}>
              {t("config.discard", "Discard")}
            </Button>
            <Button variant="primary" size="sm" onClick={handleBatchSave} isLoading={batchSaving} disabled={batchSaving}>
              <Save className="w-3 h-3 mr-1" />
              {t("config.save_all", "Save All")}
            </Button>
          </div>
        </div>
      )}
      <TomlViewer
        isOpen={showRawToml}
        onClose={() => setShowRawToml(false)}
        title={t("config.raw_toml_title", "config.toml")}
        toml={rawTomlQuery.data}
        downloadName="librefang-config.toml"
        error={
          rawTomlQuery.error
            ? (rawTomlQuery.error as Error).message ?? t("config.raw_toml_error", "Failed to load config.toml")
            : null
        }
      />
    </div>
  );
}
