import { useMutation, useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { listProviders, testProvider, type ApiActionResponse, type ProviderItem } from "../api";

const REFRESH_MS = 30000;

interface ProviderFeedback {
  type: "ok" | "error";
  text: string;
}

function isConfigured(provider: ProviderItem): boolean {
  return provider.auth_status === "configured";
}

function statusBadge(provider: ProviderItem): {
  label: string;
  className: string;
} {
  if (provider.auth_status === "configured") {
    return {
      label: "Active",
      className: "border-success/20 bg-success/10 text-success"
    };
  }
  if (provider.auth_status === "missing") {
    return {
      label: "Missing API Key",
      className: "border-warning/20 bg-warning/10 text-warning"
    };
  }
  return {
    label: provider.auth_status ?? "Offline",
    className: "border-border-subtle bg-surface text-text-dim"
  };
}

function actionText(action: ApiActionResponse): string {
  if (typeof action.message === "string" && action.message.trim().length > 0) return action.message;
  if (typeof action.status === "string" && action.status.trim().length > 0) return action.status;
  return JSON.stringify(action);
}

export function ProvidersPage() {
  const [feedback, setFeedback] = useState<Record<string, ProviderFeedback>>({});
  const [pendingProviderId, setPendingProviderId] = useState<string | null>(null);

  const providersQuery = useQuery({
    queryKey: ["providers", "list"],
    queryFn: listProviders,
    refetchInterval: REFRESH_MS
  });

  const testMutation = useMutation({
    mutationFn: testProvider
  });

  const providers = providersQuery.data ?? [];
  const configuredCount = providers.filter(isConfigured).length;
  const providersError = providersQuery.error instanceof Error ? providersQuery.error.message : "";

  async function handleTest(providerId: string) {
    setPendingProviderId(providerId);
    try {
      const result = await testMutation.mutateAsync(providerId);
      setFeedback((current) => ({
        ...current,
        [providerId]: { type: "ok", text: actionText(result) }
      }));
    } catch (error) {
      setFeedback((current) => ({
        ...current,
        [providerId]: {
          type: "error",
          text: error instanceof Error ? error.message : "Provider test failed."
        }
      }));
    } finally {
      setPendingProviderId(null);
    }
  }

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <header className="flex flex-col justify-between gap-4 sm:flex-row sm:items-end">
        <div>
          <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
            <svg className="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <rect x="2" y="2" width="20" height="8" rx="2" /><rect x="2" y="14" width="20" height="8" rx="2" />
            </svg>
            Inference Infrastructure
          </div>
          <h1 className="mt-2 text-3xl font-extrabold tracking-tight md:text-4xl">Providers</h1>
          <p className="mt-1 text-text-dim font-medium max-w-2xl">Manage LLM API connections and monitor inference health across all backends.</p>
        </div>
        <div className="flex items-center gap-3">
          <div className="hidden rounded-full border border-border-subtle bg-surface px-3 py-1.5 text-[10px] font-bold uppercase tracking-wider text-text-dim sm:block">
            {configuredCount} / {providers.length} Configured
          </div>
          <button
            className="flex h-9 items-center gap-2 rounded-xl border border-border-subtle bg-surface px-4 text-sm font-bold text-text-dim hover:text-brand hover:border-brand/30 transition-all shadow-sm disabled:opacity-50"
            type="button"
            onClick={() => void providersQuery.refetch()}
            disabled={providersQuery.isFetching}
          >
            <svg className={`h-3.5 w-3.5 ${providersQuery.isFetching ? "animate-spin" : ""}`} fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth="2">
              <path d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
            </svg>
            Refresh
          </button>
        </div>
      </header>

      {providersError ? (
        <div className="rounded-xl border border-error/20 bg-error/5 p-4 text-sm text-error font-bold">{providersError}</div>
      ) : null}

      {providersQuery.isLoading && providers.length === 0 ? (
        <div className="py-24 text-center">
          <div className="mx-auto h-10 w-10 animate-spin rounded-full border-2 border-brand border-t-transparent mb-4" />
          <p className="text-sm text-text-dim font-bold">Discovering inference providers...</p>
        </div>
      ) : null}

      {!providersQuery.isLoading && providers.length === 0 ? (
        <div className="py-24 text-center border border-dashed border-border-subtle rounded-2xl bg-surface/50">
          <p className="text-sm text-text-dim font-bold">No providers found in configuration.</p>
        </div>
      ) : null}

      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
        {providers.map((provider) => {
          const badge = statusBadge(provider);
          const providerFeedback = feedback[provider.id];
          return (
            <article key={provider.id} className="group flex flex-col rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm transition-all hover:border-brand/30 ring-1 ring-black/5 dark:ring-white/5">
              <div className="mb-5 flex items-start justify-between gap-3">
                <div className="min-w-0">
                  <h2 className="m-0 text-lg font-black tracking-tight truncate group-hover:text-brand transition-colors">{provider.display_name ?? provider.id}</h2>
                  <p className="text-[10px] font-black uppercase tracking-widest text-text-dim/60 mt-0.5">{provider.id}</p>
                </div>
                <span className={`rounded-lg border px-2 py-0.5 text-[10px] font-black uppercase tracking-widest ${badge.className}`}>{badge.label}</span>
              </div>

              <div className="grid grid-cols-2 gap-4 mb-6">
                <div className="p-3 rounded-xl bg-main/40 border border-border-subtle/50">
                  <p className="text-[10px] font-black text-text-dim/60 uppercase tracking-wider mb-1">Models</p>
                  <p className="text-xl font-black">{provider.model_count ?? 0}</p>
                </div>
                <div className="p-3 rounded-xl bg-main/40 border border-border-subtle/50">
                  <p className="text-[10px] font-black text-text-dim/60 uppercase tracking-wider mb-1">Latency</p>
                  <p className="text-xl font-black">{typeof provider.latency_ms === "number" ? `${provider.latency_ms}ms` : <span className="text-text-dim/30">—</span>}</p>
                </div>
              </div>

              <div className="space-y-2 mb-6">
                <div className="flex justify-between text-xs">
                  <span className="text-text-dim font-bold">Endpoint</span>
                  <span className="font-mono text-slate-700 dark:text-slate-300 truncate max-w-[160px]" title={provider.base_url ?? ""}>{provider.base_url || "Built-in"}</span>
                </div>
                <div className="flex justify-between text-xs">
                  <span className="text-text-dim font-bold">Env Key</span>
                  <span className="font-mono text-slate-700 dark:text-slate-300">{provider.api_key_env || "None"}</span>
                </div>
              </div>

              <div className="mt-auto pt-4 border-t border-border-subtle/30 flex items-center justify-between gap-3">
                <button
                  className="flex-1 rounded-xl border border-border-subtle bg-surface py-2 text-xs font-black text-text-dim hover:text-brand hover:border-brand/30 transition-all shadow-sm disabled:opacity-50"
                  type="button"
                  onClick={() => void handleTest(provider.id)}
                  disabled={pendingProviderId === provider.id}
                >
                  {pendingProviderId === provider.id ? "Analyzing..." : "Test Connection"}
                </button>
              </div>

              {providerFeedback ? (
                <div
                  className={`mt-4 animate-in slide-in-from-top-2 rounded-xl border p-3 text-[11px] font-bold shadow-inner ${
                    providerFeedback.type === "ok"
                      ? "border-success/20 bg-success/5 text-success"
                      : "border-error/20 bg-error/5 text-error"
                  }`}
                >
                  <div className="flex items-center gap-2">
                    <div className={`h-1.5 w-1.5 rounded-full ${providerFeedback.type === 'ok' ? 'bg-success' : 'bg-error'}`} />
                    {providerFeedback.text}
                  </div>
                </div>
              ) : null}
            </article>
          );
        })}
      </div>
    </div>
  );
}
