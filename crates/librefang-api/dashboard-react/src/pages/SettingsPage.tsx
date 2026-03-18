import { useMutation, useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { listProviders, testProvider, type ProviderItem } from "../api";

const REFRESH_MS = 30000;

type SettingsTab = "providers" | "config" | "tools" | "security" | "network";

export function SettingsPage() {
  const [tab, setTab] = useState<SettingsTab>("providers");
  const [testingProviderId, setTestingProviderId] = useState<string | null>(null);
  const [testResults, setTestResults] = useState<Record<string, "ok" | "error">>({});

  const providersQuery = useQuery({
    queryKey: ["providers", "list"],
    queryFn: listProviders,
    refetchInterval: REFRESH_MS
  });

  const testMutation = useMutation({
    mutationFn: testProvider
  });

  const providers = providersQuery.data ?? [];

  async function handleTestProvider(providerId: string) {
    setTestingProviderId(providerId);
    try {
      const result = await testMutation.mutateAsync(providerId);
      setTestResults((prev) => ({
        ...prev,
        [providerId]: result.status === "ok" ? "ok" : "error"
      }));
    } catch {
      setTestResults((prev) => ({
        ...prev,
        [providerId]: "error"
      }));
    } finally {
      setTestingProviderId(null);
    }
  }

  return (
    <section className="flex flex-col gap-4">
      <header>
        <h1 className="m-0 text-2xl font-semibold">Settings</h1>
        <p className="text-sm text-slate-400">Configure providers, models, tools, and system options.</p>
      </header>

      {/* Tabs */}
      <div className="flex gap-2 border-b border-slate-800 pb-2">
        {(["providers", "config", "tools", "security", "network"] as SettingsTab[]).map((t) => (
          <button
            key={t}
            onClick={() => setTab(t)}
            className={`rounded-lg px-3 py-2 text-sm font-medium transition ${
              tab === t
                ? "bg-sky-600 text-white"
                : "text-slate-400 hover:bg-slate-800 hover:text-white"
            }`}
          >
            {t.charAt(0).toUpperCase() + t.slice(1)}
          </button>
        ))}
      </div>

      {/* Providers Tab */}
      {tab === "providers" && (
        <div className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <div className="mb-4 flex items-center justify-between">
            <h2 className="m-0 text-base font-semibold">LLM Providers</h2>
            <button
              className="rounded-lg border border-slate-600 bg-slate-800 px-3 py-2 text-sm font-medium text-slate-100 transition hover:border-sky-500 hover:bg-slate-700 disabled:cursor-not-allowed disabled:opacity-60"
              onClick={() => void providersQuery.refetch()}
              disabled={providersQuery.isFetching}
            >
              Refresh
            </button>
          </div>

          {providersQuery.isLoading ? (
            <p className="text-sm text-slate-400">Loading providers...</p>
          ) : providers.length === 0 ? (
            <p className="text-sm text-slate-400">No providers configured.</p>
          ) : (
            <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
              {providers.map((provider) => (
                <article
                  key={provider.id}
                  className="rounded-lg border border-slate-700 bg-slate-950/70 p-4"
                >
                  <div className="flex items-center justify-between">
                    <h3 className="m-0 text-sm font-semibold">{provider.display_name ?? provider.id}</h3>
                    <div className="flex items-center gap-2">
                      {provider.reachable ? (
                        <span className="rounded-full bg-emerald-700/20 px-2 py-0.5 text-xs text-emerald-400">
                          Reachable
                        </span>
                      ) : provider.key_required ? (
                        <span className="rounded-full bg-amber-700/20 px-2 py-0.5 text-xs text-amber-400">
                          Needs Key
                        </span>
                      ) : null}
                    </div>
                  </div>

                  <div className="mt-3 space-y-1 text-xs text-slate-400">
                    {provider.model_count !== undefined && (
                      <p>{provider.model_count} models available</p>
                    )}
                    {provider.latency_ms !== undefined && (
                      <p>Latency: {provider.latency_ms}ms</p>
                    )}
                    {provider.api_key_env && (
                      <p>
                        Env: <code className="text-sky-400">{provider.api_key_env}</code>
                      </p>
                    )}
                  </div>

                  <div className="mt-3 flex gap-2">
                    <button
                      className="rounded-lg border border-sky-500 bg-sky-600 px-3 py-1.5 text-xs font-medium text-white transition hover:bg-sky-500 disabled:cursor-not-allowed disabled:opacity-60"
                      onClick={() => void handleTestProvider(provider.id)}
                      disabled={testingProviderId === provider.id}
                    >
                      {testingProviderId === provider.id ? "Testing..." : "Test"}
                    </button>
                  </div>

                  {testResults[provider.id] && (
                    <p
                      className={`mt-2 text-xs ${
                        testResults[provider.id] === "ok" ? "text-emerald-400" : "text-rose-400"
                      }`}
                    >
                      {testResults[provider.id] === "ok" ? "Connection successful" : "Connection failed"}
                    </p>
                  )}
                </article>
              ))}
            </div>
          )}
        </div>
      )}

      {/* Config Tab */}
      {tab === "config" && (
        <div className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <h2 className="m-0 mb-4 text-base font-semibold">Configuration</h2>
          <p className="text-sm text-slate-400">
            Configuration editing is not yet implemented in the React dashboard. Please use the Alpine.js
            dashboard or CLI for configuration changes.
          </p>
        </div>
      )}

      {/* Tools Tab */}
      {tab === "tools" && (
        <div className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <h2 className="m-0 mb-4 text-base font-semibold">Tools</h2>
          <p className="text-sm text-slate-400">
            Tools management is not yet implemented in the React dashboard. Please use the Alpine.js dashboard.
          </p>
        </div>
      )}

      {/* Security Tab */}
      {tab === "security" && (
        <div className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <h2 className="m-0 mb-4 text-base font-semibold">Security</h2>
          <p className="text-sm text-slate-400">
            Security settings are not yet implemented in the React dashboard. Please use the Alpine.js dashboard.
          </p>
        </div>
      )}

      {/* Network Tab */}
      {tab === "network" && (
        <div className="rounded-xl border border-slate-800 bg-slate-900/70 p-4">
          <h2 className="m-0 mb-4 text-base font-semibold">Network</h2>
          <p className="text-sm text-slate-400">
            Network settings are not yet implemented in the React dashboard. Please use the Alpine.js dashboard.
          </p>
        </div>
      )}
    </section>
  );
}
