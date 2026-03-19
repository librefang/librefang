import { useMutation, useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { listProviders, testProvider } from "../api";
import { PageHeader } from "../components/ui/PageHeader";
import { CardSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { useUIStore } from "../lib/store";
import { Server } from "lucide-react";

const REFRESH_MS = 30000;

export function ProvidersPage() {
  const { t } = useTranslation();
  const [pendingId, setPendingId] = useState<string | null>(null);
  const addToast = useUIStore((s) => s.addToast);

  const providersQuery = useQuery({ queryKey: ["providers", "list"], queryFn: listProviders, refetchInterval: REFRESH_MS });
  const testMutation = useMutation({ mutationFn: testProvider });

  const providers = providersQuery.data ?? [];
  const configuredCount = providers.filter(p => p.auth_status === "configured").length;

  async function handleTest(id: string) {
    setPendingId(id);
    try {
      await testMutation.mutateAsync(id);
      addToast(t("common.success"), "success");
    } catch (e: any) {
      addToast(e.message || t("common.error"), "error");
    } finally {
      setPendingId(null);
    }
  }

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("common.infrastructure")}
        title={t("providers.title")}
        subtitle={t("providers.subtitle")}
        isFetching={providersQuery.isFetching}
        onRefresh={() => void providersQuery.refetch()}
        icon={<Server className="h-4 w-4" />}
        actions={
          <div className="hidden rounded-full border border-border-subtle bg-surface px-3 py-1.5 text-[10px] font-bold uppercase text-text-dim sm:block">
            {t("providers.configured_count", { configured: configuredCount, total: providers.length })}
          </div>
        }
      />

      {providersQuery.isLoading ? (
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
          {[1, 2, 3, 4, 5, 6].map((i) => <CardSkeleton key={i} />)}
        </div>
      ) : providers.length === 0 ? (
        <EmptyState
          title={t("common.no_data")}
          icon={<Server className="h-6 w-6" />}
        />
      ) : (
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
          {providers.map((p) => (
            <Card key={p.id} hover padding="lg" className="flex flex-col">
              <div className="mb-5 flex items-start justify-between gap-3">
                <div className="min-w-0"><h2 className="m-0 text-lg font-black truncate">{p.display_name || p.id}</h2><p className="text-[10px] font-black uppercase text-text-dim/60 mt-0.5">{p.id}</p></div>
                <Badge variant={p.auth_status === 'configured' ? 'success' : 'default'}>
                  {p.auth_status === 'configured' ? t("common.active") : t("common.setup")}
                </Badge>
              </div>
              <div className="grid grid-cols-2 gap-4 mb-6">
                <div className="p-3 rounded-xl bg-main/40"><p className="text-[10px] font-black text-text-dim/60 uppercase mb-1">{t("providers.models")}</p><p className="text-xl font-black">{p.model_count || 0}</p></div>
                <div className="p-3 rounded-xl bg-main/40"><p className="text-[10px] font-black text-text-dim/60 uppercase mb-1">{t("providers.latency")}</p><p className="text-xl font-black">{p.latency_ms ? `${p.latency_ms}ms` : "-"}</p></div>
              </div>
              <Button variant="secondary" className="w-full" onClick={() => handleTest(p.id)} disabled={pendingId === p.id}>{pendingId === p.id ? t("providers.analyzing") : t("providers.test_connection")}</Button>
            </Card>
          ))}
        </div>
      )}
    </div>
  );
}
