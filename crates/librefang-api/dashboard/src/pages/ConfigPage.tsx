import { useTranslation } from "react-i18next";
import { useState, useCallback, useEffect, useMemo, useRef } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useRouter } from "@tanstack/react-router";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { RefreshCw, Save, Zap, Settings, Search, RotateCcw, AlertTriangle, X } from "lucide-react";
import {
  getConfigSchema, getFullConfig, setConfigValue, reloadConfig,
  type ConfigFieldSchema,
} from "../api";

/* ------------------------------------------------------------------ */
/*  Category → sections mapping                                        */
/* ------------------------------------------------------------------ */

const CATEGORY_SECTIONS: Record<string, string[]> = {
  general: ["general", "default_model", "thinking", "budget", "reload"],
  memory: ["memory", "proactive_memory"],
  tools: ["web", "browser", "links", "media", "tts", "canvas"],
  channels: ["channels", "broadcast", "auto_reply"],
  security: ["approval", "exec_policy", "vault", "oauth", "external_auth"],
  network: ["network", "a2a", "pairing"],
  infra: ["docker", "extensions", "session", "queue", "webhook_triggers", "vertex_ai"],
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
): { type: string; options?: (string | { id: string; name: string; provider: string })[] } {
  if (typeof schema === "string") return { type: schema };
  return { type: schema.type || "string", options: schema.options };
}

function getNestedValue(obj: Record<string, unknown>, section: string, field: string, rootLevel?: boolean): unknown {
  if (rootLevel) return obj[field];
  const sec = obj[section] as Record<string, unknown> | undefined;
  return sec?.[field];
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
          error ? "border-danger" : "border-border-subtle focus:border-brand"
        }`}
      />
      {error && <p className="text-[10px] text-danger">{error}</p>}
    </div>
  );
}

const SENSITIVE_PATTERNS = /api_key|secret|password|token_env|client_secret|credentials/i;

function ConfigFieldInput({
  fieldKey, fieldType, options, value, onChange,
}: {
  fieldKey: string;
  fieldType: string;
  options?: (string | { id: string; name: string; provider: string })[];
  value: unknown;
  onChange: (v: unknown) => void;
}) {
  const inputClass =
    "w-full px-3 py-1.5 rounded-xl border border-border-subtle bg-main text-xs font-mono outline-none focus:border-brand transition-colors";

  if (fieldType === "boolean") {
    return (
      <button
        onClick={() => onChange(!value)}
        className={`relative w-10 h-5 rounded-full transition-colors ${value ? "bg-brand" : "bg-border-subtle"}`}
      >
        <span className={`absolute top-0.5 w-4 h-4 rounded-full bg-white shadow transition-transform ${value ? "left-5" : "left-0.5"}`} />
      </button>
    );
  }

  if (fieldType === "select" && options) {
    const strOptions = options.map((o) => (typeof o === "string" ? o : o.id));
    const rawValue = String(value ?? "");
    const matched = strOptions.find((o) => o.toLowerCase() === rawValue.toLowerCase()) ?? rawValue;
    return (
      <select value={matched} onChange={(e) => onChange(e.target.value)} className={inputClass}>
        {matched && !strOptions.includes(matched) && <option value={matched}>{matched}</option>}
        {strOptions.map((o) => <option key={o} value={o}>{o}</option>)}
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
  const queryClient = useQueryClient();
  const router = useRouter();

  const schemaQuery = useQuery({
    queryKey: ["config", "schema"],
    queryFn: getConfigSchema,
    staleTime: 300_000,
  });

  const configQuery = useQuery({
    queryKey: ["config", "full"],
    queryFn: getFullConfig,
    staleTime: 30_000,
  });

  const [pendingChanges, setPendingChanges] = useState<Record<string, unknown>>({});
  const [saveStatus, setSaveStatus] = useState<Record<string, { ok: boolean; msg: string }>>({});
  const [searchQuery, setSearchQuery] = useState("");
  const [reloadStatus, setReloadStatus] = useState<{ ok: boolean; msg: string } | null>(null);
  const [activeSection, setActiveSection] = useState<string | null>(null);
  const searchRef = useRef<HTMLInputElement>(null);

  const hasPendingChanges = Object.keys(pendingChanges).length > 0;

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
      setPendingChanges((p) => ({ ...p, [path]: value }));
    },
    []
  );

  const saveMutation = useMutation({
    mutationFn: ({ path, value }: { path: string; value: unknown }) => setConfigValue(path, value),
    onSuccess: (data, variables) => {
      const reloadFailed = data.status !== "ok" && data.status !== "saved";
      if (reloadFailed) {
        setSaveStatus((s) => ({ ...s, [variables.path]: { ok: false, msg: t("config.saved_reload_failed", "Saved but reload failed") } }));
      } else {
        const msg = data.restart_required ? t("config.saved_restart", "Saved (restart required)") : t("common.saved", "Saved");
        setSaveStatus((s) => ({ ...s, [variables.path]: { ok: true, msg } }));
      }
      setPendingChanges((p) => {
        if (!(variables.path in p) || JSON.stringify(p[variables.path]) === JSON.stringify(variables.value)) {
          const next = { ...p }; delete next[variables.path]; return next;
        }
        return p;
      });
      queryClient.invalidateQueries({ queryKey: ["config", "full"] });
      setTimeout(() => setSaveStatus((s) => { const next = { ...s }; delete next[variables.path]; return next; }), 3000);
    },
    onError: (err: Error, variables) => {
      setSaveStatus((s) => ({ ...s, [variables.path]: { ok: false, msg: err.message } }));
      setTimeout(() => setSaveStatus((s) => { const next = { ...s }; delete next[variables.path]; return next; }), 3000);
    },
  });

  const [batchSaving, setBatchSaving] = useState(false);
  const handleBatchSave = useCallback(async () => {
    const entries = Object.entries(pendingChanges);
    if (entries.length === 0) return;
    setBatchSaving(true);
    let errors = 0;
    for (const [path, value] of entries) {
      try {
        const data = await setConfigValue(path, value);
        const reloadFailed = data.status !== "ok" && data.status !== "saved";
        const msg = reloadFailed
          ? t("config.saved_reload_failed", "Saved but reload failed")
          : data.restart_required
            ? t("config.saved_restart", "Saved (restart required)")
            : t("common.saved", "Saved");
        setSaveStatus((s) => ({ ...s, [path]: { ok: !reloadFailed, msg } }));
      } catch (err: any) {
        setSaveStatus((s) => ({ ...s, [path]: { ok: false, msg: err.message || t("config.save_failed") } }));
        errors++;
      }
    }
    setPendingChanges({});
    queryClient.invalidateQueries({ queryKey: ["config", "full"] });
    setBatchSaving(false);
    setTimeout(() => setSaveStatus({}), errors > 0 ? 5000 : 3000);
  }, [pendingChanges, queryClient, t]);

  const handleResetField = useCallback(
    (sectionKey: string, fieldKey: string, rootLevel?: boolean) => {
      const path = rootLevel ? fieldKey : `${sectionKey}.${fieldKey}`;
      setPendingChanges((p) => ({ ...p, [path]: null }));
    },
    []
  );

  const reloadMutation = useMutation({
    mutationFn: reloadConfig,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["config", "full"] });
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
    <div className="flex flex-col p-6 max-w-5xl gap-4">

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
          <Button variant="secondary" size="sm" onClick={() => reloadMutation.mutate()} isLoading={reloadMutation.isPending}>
            <RefreshCw className="w-3 h-3 mr-1.5" />
            {t("config.reload", "Reload")}
          </Button>
        </div>
      </div>

      {/* Row 2: tabs — always visible when >1 section; grayed during search */}
      {sectionKeys.length > 1 && (
        <div className="flex items-center gap-0 border-b border-border-subtle -mx-6 px-6">
          {sectionKeys.map((sKey) => {
            const isActive = !isSearching && effectiveTab === sKey;
            return (
              <button
                key={sKey}
                onClick={() => { setActiveSection(sKey); setSearchQuery(""); }}
                disabled={isSearching}
                className={`px-3 py-2 text-xs font-medium border-b-2 -mb-px transition-colors whitespace-nowrap ${
                  isActive
                    ? "border-brand text-brand"
                    : isSearching
                      ? "border-transparent text-text-dim/40 cursor-not-allowed"
                      : "border-transparent text-text-dim hover:text-text hover:border-border-subtle"
                }`}
              >
                {t(`config.sec_${sKey}`, sectionLabelFallback(sKey))}
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

      {/* Row 3: search + unsaved actions */}
      <div className="flex items-center gap-3">
        <div className="relative flex-1">
          <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-text-dim pointer-events-none" />
          <input
            ref={searchRef}
            type="text"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            placeholder={t("config.search_placeholder", "Search fields...")}
            className="w-full pl-9 pr-8 py-2 rounded-xl border border-border-subtle bg-surface text-xs outline-none focus:border-brand transition-colors"
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
        {hasPendingChanges && (
          <div className="flex items-center gap-2 shrink-0">
            <Badge variant="warning">
              <AlertTriangle className="w-2.5 h-2.5 mr-1" />
              {Object.keys(pendingChanges).length} {t("config.unsaved", "unsaved")}
            </Badge>
            <Button variant="ghost" size="sm" onClick={() => setPendingChanges({})}>
              {t("config.discard", "Discard")}
            </Button>
            <Button variant="primary" size="sm" onClick={handleBatchSave} isLoading={batchSaving} disabled={batchSaving}>
              <Save className="w-3 h-3 mr-1" />
              {t("config.save_all", "Save All")}
            </Button>
          </div>
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
          const allFields = Object.entries(sec.fields);
          const fieldsToShow = q
            ? allFields.filter(([fKey]) => visibleFields.includes(fKey))
            : allFields;

          const hasBadges = sec.hot_reloadable || sec.root_level;
          // Only show section header when searching (to identify which section fields belong to)
          // or when there are metadata badges to display
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
                  {isSearching && (
                    <span className="text-[10px] text-text-dim ml-auto">
                      {fieldsToShow.length}/{allFields.length} {t("config.fields_unit")}
                    </span>
                  )}
                </div>
              )}
              <div className="divide-y divide-border-subtle/30">
                {fieldsToShow.map(([fieldKey, fieldSchema]) => {
                  const { type: fieldType, options } = resolveFieldType(fieldSchema);
                  const path = sec.root_level ? fieldKey : `${sKey}.${fieldKey}`;
                  const currentValue = path in pendingChanges
                    ? pendingChanges[path]
                    : getNestedValue(config, sKey, fieldKey, sec.root_level);
                  const hasPending = path in pendingChanges;
                  const isSaving = saveMutation.isPending && saveMutation.variables?.path === path;
                  const statusForField = saveStatus[path] ?? null;
                  const fieldDesc = t(`config.desc_${fieldKey}`, "");

                  return (
                    <div key={fieldKey} className="flex items-center gap-4 px-5 py-3">
                      {/* Label + key */}
                      <div className="w-44 shrink-0">
                        <p className="text-xs font-semibold leading-tight">
                          {t(`config.fld_${fieldKey}`, fieldLabelFallback(fieldKey))}
                        </p>
                        <p className="text-[10px] text-text-dim font-mono leading-tight mt-0.5">{fieldKey}</p>
                      </div>
                      {/* Input */}
                      <div className="flex-1 min-w-0">
                        <ConfigFieldInput
                          fieldKey={fieldKey}
                          fieldType={fieldType}
                          options={options}
                          value={currentValue}
                          onChange={(v) => handleFieldChange(sKey, fieldKey, v, sec.root_level)}
                        />
                      </div>
                      {/* Description — right of input, hidden on narrow screens */}
                      <div className="w-52 shrink-0 hidden xl:block">
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
    </div>
  );
}
