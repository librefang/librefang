import { useTranslation } from "react-i18next";
import { useState, useCallback, useEffect, useMemo, useRef } from "react";
import { useRouter } from "@tanstack/react-router";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import {
  RefreshCw, Save, Zap, Settings, Search, RotateCcw,
  AlertTriangle, X, Copy, Check, FileText,
} from "lucide-react";
import { type ConfigFieldSchema } from "../api";
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

/* ------------------------------------------------------------------ */
/*  Category → sections mapping                                        */
/* ------------------------------------------------------------------ */

const CATEGORY_SECTIONS: Record<string, string[]> = {
  general: ["general", "default_model", "thinking", "budget", "reload"],
  memory: ["memory", "proactive_memory", "auto_dream"],
  tools: ["web", "browser", "links", "media", "tts", "canvas"],
  channels: ["channels", "broadcast", "auto_reply"],
  security: ["approval", "exec_policy", "vault", "oauth", "external_auth", "terminal"],
  network: ["network", "a2a", "pairing"],
  infra: ["docker", "extensions", "session", "queue", "webhook_triggers", "vertex_ai"],
};

// Explicit field ordering for sections where the server-side JSON schema
// ordering is wrong for the user. The API serialises `sec.fields` via
// serde_json's default BTreeMap, which alphabetises keys — so for sections
// like `default_model` the user sees `model` before `provider`, even though
// model is a cascaded filter of the selected provider (ConfigPage:679). The
// rendering code falls back to the alphabetised order for any section not
// listed here, so forgetting an entry is a no-op rather than a regression.
// Closes #2746.
const SECTION_FIELD_ORDER: Record<string, string[]> = {
  default_model: ["provider", "model", "api_key_env", "base_url"],
};

function sectionLabelFallback(key: string): string {
  return key.split("_").map((w) => w.charAt(0).toUpperCase() + w.slice(1)).join(" ");
}

function fieldLabelFallback(key: string): string {
  return key.replace(/_/g, " ").replace(/\b\w/g, (c) => c.toUpperCase())
    .replace(/\bApi\b/g, "API").replace(/\bUrl\b/g, "URL")
    .replace(/\bSql\b/g, "SQL").replace(/\bSsl\b/g, "SSL")
    .replace(/\bTls\b/g, "TLS").replace(/\bTtl\b/g, "TTL")
    .replace(/\bEnv\b/g, "Env Var").replace(/\bId\b/g, "ID")
    .replace(/\bUsd\b/g, "USD").replace(/\bLlm\b/g, "LLM")
    .replace(/\bMdns\b/g, "mDNS").replace(/\bTotp\b/g, "TOTP");
}

function resolveFieldType(
  schema: string | ConfigFieldSchema
): { type: string; options?: ConfigFieldSchema["options"]; min?: number; max?: number; step?: number } {
  if (typeof schema === "string") return { type: schema };
  return { type: schema.type || "string", options: schema.options, min: schema.min, max: schema.max, step: schema.step };
}

function getNestedValue(obj: Record<string, unknown>, section: string, field: string, rootLevel?: boolean): unknown {
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
  array:   "text-teal-500 bg-teal-500/10",
  "string[]": "text-teal-500 bg-teal-500/10",
  object:  "text-orange-500 bg-orange-500/10",
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
      try { if (JSON.stringify(JSON.parse(prev), null, 2) === incoming) return prev; } catch {}
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

function ConfigFieldInput({
  fieldKey, fieldType, options, min, max, step, value, onChange,
}: {
  fieldKey: string;
  fieldType: string;
  options?: ConfigFieldSchema["options"];
  min?: number;
  max?: number;
  step?: number;
  value: unknown;
  onChange: (v: unknown) => void;
}) {
  const { t } = useTranslation();
  const inputClass =
    "w-full px-3 py-1.5 rounded-xl border border-border-subtle bg-main text-xs font-mono outline-none focus:border-primary transition-colors";

  if (fieldType === "boolean") {
    // Wrap in a fixed-height container so it aligns with other input rows
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
    const normalizedOptions = options.map((o) => {
      if (typeof o === "string") {
        return { value: o, label: t(`config.${fieldKey}_${o}`, o) };
      }
      if ("value" in o && "label" in o) return { value: String((o as { value: unknown }).value), label: String((o as { label: unknown }).label) };
      if ("id" in o) return { value: (o as { id: string; name?: string }).id, label: (o as { name?: string; id: string }).name ?? (o as { id: string }).id };
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
  const router = useRouter();

  const schemaQuery = useConfigSchema();
  const configQuery = useFullConfig();

  const [pendingChanges, setPendingChanges] = useState<Record<string, unknown>>({});
  const [saveStatus, setSaveStatus] = useState<Record<string, { ok: boolean; msg: string }>>({});
  const [searchQuery, setSearchQuery] = useState("");
  const [reloadStatus, setReloadStatus] = useState<{ ok: boolean; msg: string } | null>(null);
  const [activeSection, setActiveSection] = useState<string | null>(null);
  const [showRawToml, setShowRawToml] = useState(false);
  const searchRef = useRef<HTMLInputElement>(null);
  const rawTomlQuery = useRawConfigToml(showRawToml);

  const hasPendingChanges = Object.keys(pendingChanges).length > 0;

  // ── Global keyboard shortcuts: / to focus search, Esc to clear ────
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

  useEffect(() => {
    if (!hasPendingChanges) return;
    const unsub = router.subscribe("onBeforeNavigate", () => {
      if (Object.keys(pendingChanges).length > 0) {
        if (!window.confirm(t("config.unsaved_warning", "You have unsaved changes. Discard them?"))) {
          throw new Error("Navigation cancelled");
        }
        setPendingChanges({});
      }
    });
    return unsub;
  }, [hasPendingChanges, pendingChanges, router, t]);

  const handleFieldChange = useCallback(
    (sectionKey: string, fieldKey: string, value: unknown, rootLevel?: boolean) => {
      const path = rootLevel ? fieldKey : `${sectionKey}.${fieldKey}`;
      setPendingChanges((p) => {
        const next = { ...p, [path]: value };
        // Cascading: when provider changes in default_model, clear model if
        // the current selection doesn't belong to the new provider.
        if (sectionKey === "default_model" && fieldKey === "provider" && value) {
          const modelPath = "default_model.model";
          const currentModel = modelPath in p ? p[modelPath] : getNestedValue(configQuery.data ?? {}, "default_model", "model");
          const modelOptions = (schemaQuery.data?.sections?.default_model?.fields?.model as ConfigFieldSchema)?.options;
          if (modelOptions && currentModel) {
            const modelBelongsToProvider = modelOptions.some((o: unknown) =>
              typeof o === "object" && o !== null && "id" in o && "provider" in o
              && (o as { id: string }).id === String(currentModel)
              && (o as { provider: string }).provider === String(value)
            );
            if (!modelBelongsToProvider) {
              next[modelPath] = null;
            }
          }
        }
        return next;
      });
    },
    [configQuery.data, schemaQuery.data]
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
      setTimeout(() => setSaveStatus((s) => { const next = { ...s }; delete next[variables.path]; return next; }), 3000);
    },
    onError: (err: Error, variables) => {
      setSaveStatus((s) => ({ ...s, [variables.path]: { ok: false, msg: err.message } }));
      setTimeout(() => setSaveStatus((s) => { const next = { ...s }; delete next[variables.path]; return next; }), 3000);
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
      setTimeout(() => setSaveStatus((s) => {
        const next = { ...s };
        delete next.__batch__;
        return next;
      }), 5000);
    },
  });
  const handleBatchSave = useCallback(async () => {
    const entries = Object.entries(pendingChanges);
    if (entries.length === 0) return;
    setBatchSaving(true);
    try {
      // Per-entry failures are folded into the successful batch result.
      // Mutation-level failures are handled in the hook's onError; await here
      // so batchSaving tracks the full request and no rejection is left unhandled.
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
  const allSections = schemaQuery.data?.sections ?? {};
  const config = configQuery.data ?? {};
  const sectionKeys = (CATEGORY_SECTIONS[category] ?? []).filter((s) => s in allSections);
  const categoryTitle = t(`config.cat_${category}`, sectionLabelFallback(category));
  const q = searchQuery.toLowerCase();
  const isSearching = q.length > 0;

  const effectiveTab = isSearching
    ? null
    : (activeSection && sectionKeys.includes(activeSection) ? activeSection : sectionKeys[0] ?? null);

  // Which sections have pending changes (for tab dot indicators)
  const sectionHasPending = useCallback((sKey: string): boolean => {
    const sec = allSections[sKey];
    if (!sec) return false;
    return Object.keys(pendingChanges).some((path) =>
      sec.root_level ? path in sec.fields : path.startsWith(sKey + ".")
    );
  }, [allSections, pendingChanges]);

  const filteredSections = useMemo(() => {
    const keysToShow = effectiveTab ? [effectiveTab] : sectionKeys;
    if (!q) return keysToShow.map((sKey) => ({ sKey, fields: Object.keys(allSections[sKey]?.fields ?? {}) }));
    return keysToShow
      .map((sKey) => {
        const sec = allSections[sKey];
        if (!sec) return null;
        const sectionMatches = t(`config.sec_${sKey}`, sectionLabelFallback(sKey)).toLowerCase().includes(q) || sKey.includes(q);
        const matchedFields = Object.keys(sec.fields).filter((fKey) =>
          sectionMatches || fKey.includes(q) || t(`config.fld_${fKey}`, fieldLabelFallback(fKey)).toLowerCase().includes(q)
        );
        return matchedFields.length > 0 ? { sKey, fields: matchedFields } : null;
      })
      .filter((x): x is { sKey: string; fields: string[] } => x !== null);
  }, [sectionKeys, allSections, q, effectiveTab]);

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

      {/* Row 2: tabs — always visible when >1 section; grayed/disabled during search */}
      {sectionKeys.length > 1 && (
        <div className="flex items-center border-b border-border-subtle -mx-6 px-6">
          {sectionKeys.map((sKey) => {
            const isActive = !isSearching && effectiveTab === sKey;
            const hasDot = sectionHasPending(sKey);
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
          const sec = allSections[sKey];
          const rawFields = Object.entries(sec.fields);
          type FieldEntry = (typeof rawFields)[number];
          // Apply the per-section override (see SECTION_FIELD_ORDER docstring).
          // Listed keys come first in the declared order; any un-listed keys
          // keep their original (alphabetical) position at the tail.
          const order = SECTION_FIELD_ORDER[sKey];
          const allFields: FieldEntry[] = order
            ? [
                ...order
                  .map((k) => rawFields.find(([fk]) => fk === k))
                  .filter((e): e is FieldEntry => e !== undefined),
                ...rawFields.filter(([fk]) => !order.includes(fk)),
              ]
            : rawFields;
          const fieldsToShow = q
            ? allFields.filter(([fKey]) => visibleFields.includes(fKey))
            : allFields;

          const hasBadges = sec.hot_reloadable || sec.root_level;
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
                  {sec.hot_reloadable && (
                    <Badge variant="success"><Zap className="w-2.5 h-2.5 mr-0.5" />{t("config.hot_reload", "Hot Reload")}</Badge>
                  )}
                  {sec.root_level && (
                    <Badge variant="info">{t("config.root_level", "Root Level")}</Badge>
                  )}
                  <div className="ml-auto flex items-center gap-2">
                    {isSearching && (
                      <span className="text-[10px] text-text-dim">
                        {fieldsToShow.length}/{allFields.length} {t("config.fields_unit")}
                      </span>
                    )}
                    {/* Section-level reset: only show when any field in this section has a pending change */}
                    {fieldsToShow.some(([fKey]) => {
                      const p = sec.root_level ? fKey : `${sKey}.${fKey}`;
                      return p in pendingChanges;
                    }) && (
                      <button
                        onClick={() => handleResetSection(sKey, fieldsToShow.map(([fKey]) => fKey), sec.root_level)}
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
                {fieldsToShow.map(([fieldKey, fieldSchema]) => {
                  const { type: fieldType, options: rawOptions, min, max, step } = resolveFieldType(fieldSchema);
                  const path = sec.root_level ? fieldKey : `${sKey}.${fieldKey}`;
                  const currentValue = path in pendingChanges
                    ? pendingChanges[path]
                    : getNestedValue(config, sKey, fieldKey, sec.root_level);
                  const hasPending = path in pendingChanges;
                  const isSaving = saveMutation.isPending && saveMutation.variables?.path === path;
                  const statusForField = saveStatus[path] ?? null;
                  const fieldDesc = t(`config.desc_${fieldKey}`, "");
                  const fieldLabel = t(`config.fld_${fieldKey}`, fieldLabelFallback(fieldKey));

                  // Cascading filter: when editing model in default_model,
                  // only show models matching the selected provider.
                  let options = rawOptions;
                  if (sKey === "default_model" && fieldKey === "model" && rawOptions) {
                    const providerPath = "default_model.provider";
                    const selectedProvider = providerPath in pendingChanges
                      ? String(pendingChanges[providerPath] ?? "")
                      : String(getNestedValue(config, "default_model", "provider") ?? "");
                    if (selectedProvider) {
                      options = rawOptions.filter((o) => {
                        if (typeof o === "object" && o !== null && "provider" in o) {
                          return (o as { provider: string }).provider === selectedProvider;
                        }
                        return true;
                      });
                    }
                  }

                  return (
                    <div key={fieldKey} className="flex items-start gap-4 px-5 py-3 group">
                      {/* Label + key + type badge */}
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
                      {/* Input + description below */}
                      <div className="flex-1 min-w-0 flex flex-col gap-1 pt-1">
                        <ConfigFieldInput
                          fieldKey={fieldKey}
                          fieldType={fieldType}
                          options={options}
                          min={min}
                          max={max}
                          step={step}
                          value={currentValue}
                          onChange={(v) => handleFieldChange(sKey, fieldKey, v, sec.root_level)}
                        />
                        {fieldDesc && (
                          <p className="text-[10px] text-text-dim leading-relaxed">{fieldDesc}</p>
                        )}
                      </div>
                      {/* Actions */}
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
                              onClick={() => handleResetField(sKey, fieldKey, sec.root_level)}
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
        <div className="fixed bottom-0 left-0 right-0 z-40 flex justify-center pointer-events-none">
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
