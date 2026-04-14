import { useTranslation } from "react-i18next";
import { useState, useCallback } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { PageHeader } from "../components/ui/PageHeader";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { RefreshCw, Save, Zap, Settings } from "lucide-react";
import {
  getConfigSchema, getFullConfig, setConfigValue, reloadConfig,
  type ConfigSectionSchema, type ConfigFieldSchema,
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

function sectionLabel(key: string): string {
  return key.split("_").map((w) => w.charAt(0).toUpperCase() + w.slice(1)).join(" ");
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

function ConfigFieldInput({
  fieldType, options, value, onChange,
}: {
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
    return (
      <select value={String(value ?? "")} onChange={(e) => onChange(e.target.value)} className={inputClass}>
        <option value="">—</option>
        {strOptions.map((o) => <option key={o} value={o}>{o}</option>)}
      </select>
    );
  }

  if (fieldType === "number") {
    return (
      <input type="number" value={value != null ? String(value) : ""}
        onChange={(e) => { const v = e.target.value; onChange(v === "" ? null : Number(v)); }}
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
    return (
      <pre className="text-[10px] text-text-dim font-mono bg-main rounded-lg px-3 py-2 max-h-24 overflow-auto border border-border-subtle">
        {value != null ? JSON.stringify(value, null, 2) : "—"}
      </pre>
    );
  }

  return (
    <input type="text" value={String(value ?? "")}
      onChange={(e) => onChange(e.target.value || null)} className={inputClass} />
  );
}

/* ------------------------------------------------------------------ */
/*  Section card                                                       */
/* ------------------------------------------------------------------ */

function SectionCard({
  sectionKey, sectionSchema, config,
}: {
  sectionKey: string;
  sectionSchema: ConfigSectionSchema;
  config: Record<string, unknown>;
}) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [pendingChanges, setPendingChanges] = useState<Record<string, unknown>>({});
  const [saveStatus, setSaveStatus] = useState<{ path: string; ok: boolean; msg: string } | null>(null);

  const saveMutation = useMutation({
    mutationFn: ({ path, value }: { path: string; value: unknown }) => setConfigValue(path, value),
    onSuccess: (_data, variables) => {
      setSaveStatus({ path: variables.path, ok: true, msg: t("common.saved", "Saved") });
      setPendingChanges((p) => { const next = { ...p }; delete next[variables.path]; return next; });
      queryClient.invalidateQueries({ queryKey: ["config", "full"] });
      setTimeout(() => setSaveStatus(null), 2000);
    },
    onError: (err: Error, variables) => {
      setSaveStatus({ path: variables.path, ok: false, msg: err.message });
      setTimeout(() => setSaveStatus(null), 3000);
    },
  });

  const fields = Object.entries(sectionSchema.fields);

  const handleFieldChange = useCallback(
    (fieldKey: string, value: unknown) => {
      const path = sectionSchema.root_level ? fieldKey : `${sectionKey}.${fieldKey}`;
      setPendingChanges((p) => ({ ...p, [path]: value }));
    },
    [sectionKey, sectionSchema.root_level]
  );

  const handleSave = useCallback(
    (path: string) => {
      if (path in pendingChanges) saveMutation.mutate({ path, value: pendingChanges[path] });
    },
    [pendingChanges, saveMutation]
  );

  return (
    <div className="rounded-2xl border border-border-subtle bg-surface overflow-hidden">
      <div className="flex items-center gap-3 px-5 py-4 border-b border-border-subtle/50">
        <h3 className="text-sm font-bold">{sectionLabel(sectionKey)}</h3>
        {sectionSchema.hot_reloadable && (
          <Badge variant="success"><Zap className="w-2.5 h-2.5 mr-0.5" />{t("config.hot_reload", "Hot Reload")}</Badge>
        )}
        {sectionSchema.root_level && (
          <Badge variant="info">{t("config.root_level", "Root Level")}</Badge>
        )}
        <span className="text-[10px] text-text-dim ml-auto">
          {fields.length} {fields.length === 1 ? "field" : "fields"}
        </span>
      </div>
      <div className="px-5 py-2">
        {fields.map(([fieldKey, fieldSchema]) => {
          const { type: fieldType, options } = resolveFieldType(fieldSchema);
          const path = sectionSchema.root_level ? fieldKey : `${sectionKey}.${fieldKey}`;
          const currentValue = path in pendingChanges
            ? pendingChanges[path]
            : getNestedValue(config, sectionKey, fieldKey, sectionSchema.root_level);
          const hasPending = path in pendingChanges;
          const isSaving = saveMutation.isPending && saveMutation.variables?.path === path;
          const statusForField = saveStatus?.path === path ? saveStatus : null;

          return (
            <div key={fieldKey} className="flex items-center gap-4 py-3 border-b border-border-subtle/30 last:border-0">
              <div className="w-48 shrink-0">
                <p className="text-xs font-semibold font-mono">{fieldKey}</p>
                <p className="text-[10px] text-text-dim">{fieldType}</p>
              </div>
              <div className="flex-1 min-w-0">
                <ConfigFieldInput fieldType={fieldType} options={options} value={currentValue}
                  onChange={(v) => handleFieldChange(fieldKey, v)} />
              </div>
              <div className="w-20 shrink-0 flex items-center justify-end gap-1">
                {fieldType !== "object" && hasPending && (
                  <Button variant="primary" size="sm" onClick={() => handleSave(path)} isLoading={isSaving} disabled={isSaving}>
                    <Save className="w-3 h-3" />
                  </Button>
                )}
                {statusForField && (
                  <span className={`text-[10px] font-semibold ${statusForField.ok ? "text-success" : "text-danger"}`}>
                    {statusForField.msg}
                  </span>
                )}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

/* ------------------------------------------------------------------ */
/*  Page component — one per category                                  */
/* ------------------------------------------------------------------ */

export function ConfigPage({ category }: { category: string }) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();

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

  const reloadMutation = useMutation({
    mutationFn: reloadConfig,
    onSuccess: () => { queryClient.invalidateQueries({ queryKey: ["config", "full"] }); },
  });

  const allSections = schemaQuery.data?.sections ?? {};
  const config = configQuery.data ?? {};
  const sectionKeys = (CATEGORY_SECTIONS[category] ?? []).filter((s) => s in allSections);

  const categoryTitle = t(`config.cat_${category}`, sectionLabel(category));

  if (schemaQuery.isLoading || configQuery.isLoading) {
    return (
      <div className="flex flex-col gap-6 p-6">
        <PageHeader title={categoryTitle} icon={Settings} description={t("config.desc", "System configuration editor")} />
        <div className="rounded-2xl border border-border-subtle bg-surface p-8 text-center text-text-dim text-sm">
          {t("common.loading", "Loading...")}
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-6 p-6">
      <div className="flex items-center justify-between">
        <PageHeader title={categoryTitle} icon={Settings} description={t("config.desc", "System configuration editor")} />
        <Button variant="secondary" size="sm" onClick={() => reloadMutation.mutate()} isLoading={reloadMutation.isPending}>
          <RefreshCw className="w-3 h-3 mr-1.5" />
          {t("config.reload", "Reload")}
        </Button>
      </div>

      <div className="flex flex-col gap-4">
        {sectionKeys.map((sKey) => (
          <SectionCard key={sKey} sectionKey={sKey} sectionSchema={allSections[sKey]} config={config} />
        ))}
      </div>
    </div>
  );
}
