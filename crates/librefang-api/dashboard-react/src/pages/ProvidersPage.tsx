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
      label: "Configured",
      className: "border-emerald-700 bg-emerald-700/20 text-emerald-300"
    };
  }
  if (provider.auth_status === "missing") {
    return {
      label: "Missing Key",
      className: "border-amber-700 bg-amber-700/20 text-amber-300"
    };
  }
  return {
    label: provider.auth_status ?? "Unknown",
    className: "border-slate-700 bg-slate-800/60 text-slate-300"
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
    <section className="flex flex-col gap-4">
      <header className="flex flex-col justify-between gap-3 sm:flex-row sm:items-start">
        <div>
          <h1 className="m-0 text-2xl font-semibold">Providers</h1>
          <p className="text-sm text-slate-400">Connection status and model-provider health.</p>
        </div>
        <div className="flex items-center gap-2">
          <span className="rounded-full border border-slate-700 bg-slate-800/60 px-2 py-1 text-xs text-slate-300">
            {configuredCount}/{providers.length} configured
          </span>
          <button
            className="rounded-lg border border-sky-500 bg-sky-600 px-3 py-2 text-sm font-medium text-white transition hover:bg-sky-500 disabled:cursor-not-allowed disabled:opacity-60"
            type="button"
            onClick={() => void providersQuery.refetch()}
            disabled={providersQuery.isFetching}
          >
            Refresh
          </button>
        </div>
      </header>

      {providersError ? (
        <div className="rounded-xl border border-rose-700 bg-rose-700/15 p-4 text-rose-200">{providersError}</div>
      ) : null}

      {providersQuery.isLoading && providers.length === 0 ? (
        <div className="rounded-xl border border-slate-800 bg-slate-900/70 p-4 text-sm text-slate-400">Loading providers...</div>
      ) : null}

      {!providersQuery.isLoading && providers.length === 0 ? (
        <div className="rounded-xl border border-slate-800 bg-slate-900/70 p-4 text-sm text-slate-400">No providers found.</div>
      ) : null}

      <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
        {providers.map((provider) => {
          const badge = statusBadge(provider);
          const providerFeedback = feedback[provider.id];
          return (
            <article key={provider.id} className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
              <div className="mb-3 flex items-start justify-between gap-3">
                <div>
                  <h2 className="m-0 text-base font-semibold">{provider.display_name ?? provider.id}</h2>
                  <p className="text-xs text-slate-500">{provider.id}</p>
                </div>
                <span className={`rounded-full border px-2 py-1 text-[11px] ${badge.className}`}>{badge.label}</span>
              </div>

              <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-sm">
                <dt className="text-slate-400">Models</dt>
                <dd>{provider.model_count ?? 0}</dd>
                <dt className="text-slate-400">Base URL</dt>
                <dd className="truncate">{provider.base_url ?? "-"}</dd>
                <dt className="text-slate-400">API key env</dt>
                <dd>{provider.api_key_env ?? "-"}</dd>
                <dt className="text-slate-400">Latency</dt>
                <dd>{typeof provider.latency_ms === "number" ? `${provider.latency_ms} ms` : "-"}</dd>
              </dl>

              <div className="mt-3 flex items-center justify-between gap-2">
                <button
                  className="rounded-lg border border-slate-600 bg-slate-800 px-3 py-2 text-xs font-medium text-slate-100 transition hover:border-sky-500 hover:bg-slate-700 disabled:cursor-not-allowed disabled:opacity-60"
                  type="button"
                  onClick={() => void handleTest(provider.id)}
                  disabled={pendingProviderId === provider.id}
                >
                  {pendingProviderId === provider.id ? "Testing..." : "Test Connection"}
                </button>
              </div>

              {providerFeedback ? (
                <p
                  className={`mt-3 rounded-lg border p-2 text-xs ${
                    providerFeedback.type === "ok"
                      ? "border-emerald-700 bg-emerald-700/10 text-emerald-200"
                      : "border-rose-700 bg-rose-700/10 text-rose-200"
                  }`}
                >
                  {providerFeedback.text}
                </p>
              ) : null}
            </article>
          );
        })}
      </div>
    </section>
  );
}
