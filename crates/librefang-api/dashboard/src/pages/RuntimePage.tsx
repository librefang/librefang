import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import type {
  DashboardSnapshot, VersionResponse, QueueStatusResponse, HealthCheck,
  HealthDetailResponse, SecurityStatusResponse, AuditEntry, BackupItem, TaskQueueItem,
} from "../api";
import {
  loadDashboardSnapshot, getVersionInfo, getQueueStatus, shutdownServer, reloadConfig,
  getHealthDetail, getSecurityStatus, listAuditRecent, verifyAuditChain,
  listBackups, createBackup, restoreBackup, deleteBackup,
  getTaskQueueStatus, listTaskQueue, deleteTaskFromQueue, retryTask, cleanupSessions,
} from "../api";
import { PageHeader } from "../components/ui/PageHeader";
import { CardSkeleton } from "../components/ui/Skeleton";
import { isProviderAvailable } from "../lib/status";
import { Card } from "../components/ui/Card";
import { Badge } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import { ConfirmDialog } from "../components/ui/ConfirmDialog";
import {
  Activity, Cpu, HardDrive, Zap, Timer, Layers, CheckCircle2, GitCommit,
  Calendar, Server, Monitor, Settings, HeartPulse, Box, Globe, FolderOpen,
  FileText, Gauge, Network, XCircle, RefreshCw, Power,
  Shield, ShieldCheck, Archive, Download, Trash2, RotateCcw,
  AlertTriangle, Clock, Brain, Database, Lock, Eye,
} from "lucide-react";

const REFRESH_MS = 30000;

function formatUptime(seconds?: number): string {
  if (seconds === undefined || seconds <= 0) return "-";
  const d = Math.floor(seconds / 86400);
  const h = Math.floor((seconds % 86400) / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m`;
  return "<1m";
}

function formatBytes(bytes?: number): string {
  if (!bytes) return "-";
  if (bytes >= 1_000_000) return `${(bytes / 1_000_000).toFixed(1)} MB`;
  if (bytes >= 1_000) return `${(bytes / 1_000).toFixed(1)} KB`;
  return `${bytes} B`;
}

function InfoRow({ icon: Icon, label, value, mono, color }: {
  icon: React.ComponentType<{ className?: string }>;
  label: string;
  value: React.ReactNode;
  mono?: boolean;
  color?: string;
}) {
  return (
    <div className="flex items-center gap-3">
      <Icon className="w-3.5 h-3.5 text-text-dim/40 shrink-0" />
      <span className="text-xs text-text-dim flex-1">{label}</span>
      <span className={`text-sm ${mono ? "font-mono" : ""} ${color ?? "text-text"} truncate max-w-[200px]`}>{value}</span>
    </div>
  );
}

function ProtectionBadge({ name, enabled }: { name: string; enabled: boolean }) {
  return (
    <div className="flex items-center gap-2 py-1">
      {enabled
        ? <CheckCircle2 className="w-3.5 h-3.5 text-success shrink-0" />
        : <XCircle className="w-3.5 h-3.5 text-error shrink-0" />
      }
      <span className="text-xs flex-1">{name.replace(/_/g, " ")}</span>
    </div>
  );
}

export function RuntimePage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [showShutdownConfirm, setShowShutdownConfirm] = useState(false);
  const [reloadResult, setReloadResult] = useState<string | null>(null);

  // --- Queries ---
  const snapshotQuery = useQuery<DashboardSnapshot>({
    queryKey: ["dashboard", "snapshot"],
    queryFn: loadDashboardSnapshot,
    refetchInterval: REFRESH_MS,
  });
  const versionQuery = useQuery<VersionResponse>({
    queryKey: ["version"],
    queryFn: getVersionInfo,
    staleTime: Infinity,
  });
  const queueQuery = useQuery<QueueStatusResponse>({
    queryKey: ["queue", "status"],
    queryFn: getQueueStatus,
    refetchInterval: 15000,
  });
  const healthDetailQuery = useQuery<HealthDetailResponse>({
    queryKey: ["health", "detail"],
    queryFn: getHealthDetail,
    refetchInterval: REFRESH_MS,
  });
  const securityQuery = useQuery<SecurityStatusResponse>({
    queryKey: ["security"],
    queryFn: getSecurityStatus,
    refetchInterval: REFRESH_MS * 4,
  });
  const auditQuery = useQuery({
    queryKey: ["audit", "recent"],
    queryFn: () => listAuditRecent(20),
    refetchInterval: REFRESH_MS,
  });
  const auditVerifyQuery = useQuery({
    queryKey: ["audit", "verify"],
    queryFn: verifyAuditChain,
    refetchInterval: REFRESH_MS * 4,
  });
  const backupsQuery = useQuery({
    queryKey: ["backups"],
    queryFn: listBackups,
    refetchInterval: REFRESH_MS * 2,
  });
  const taskStatusQuery = useQuery({
    queryKey: ["tasks", "status"],
    queryFn: getTaskQueueStatus,
    refetchInterval: 15000,
  });
  const taskListQuery = useQuery({
    queryKey: ["tasks", "list"],
    queryFn: () => listTaskQueue(),
    refetchInterval: REFRESH_MS,
  });

  // --- Mutations ---
  const shutdownMutation = useMutation({
    mutationFn: shutdownServer,
    onSuccess: () => setShowShutdownConfirm(false),
  });
  const reloadMutation = useMutation({
    mutationFn: reloadConfig,
    onSuccess: (data) => {
      setReloadResult(data.status);
      setTimeout(() => setReloadResult(null), 5000);
      snapshotQuery.refetch();
    },
  });
  const backupMutation = useMutation({
    mutationFn: createBackup,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["backups"] }),
  });
  const restoreMutation = useMutation({
    mutationFn: restoreBackup,
  });
  const deleteBackupMutation = useMutation({
    mutationFn: deleteBackup,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["backups"] }),
  });
  const deleteTaskMutation = useMutation({
    mutationFn: deleteTaskFromQueue,
    onSuccess: () => { queryClient.invalidateQueries({ queryKey: ["tasks"] }); },
  });
  const retryTaskMutation = useMutation({
    mutationFn: retryTask,
    onSuccess: () => { queryClient.invalidateQueries({ queryKey: ["tasks"] }); },
  });
  const cleanupMutation = useMutation({ mutationFn: cleanupSessions });

  // --- Derived data ---
  const snapshot = snapshotQuery.data ?? null;
  const version = versionQuery.data ?? null;
  const queue = queueQuery.data ?? null;
  const hd = healthDetailQuery.data ?? null;
  const security = securityQuery.data ?? null;
  const status = snapshot?.status;

  const uptimeStr = formatUptime(status?.uptime_seconds);
  const healthChecks = snapshot?.health?.checks ?? [];
  const allHealthy = healthChecks.length > 0 && healthChecks.every((c: HealthCheck) => c.status === "ok" || c.status === "pass" || c.status === "healthy");
  const lanes = queue?.lanes ?? [];
  const queueConfig = queue?.config;
  const auditEntries = auditQuery.data?.entries ?? [];
  const auditValid = auditVerifyQuery.data?.valid;
  const backups = backupsQuery.data?.backups ?? [];
  const taskStatus = taskStatusQuery.data;
  const tasks = taskListQuery.data?.tasks ?? [];

  const refreshAll = () => {
    snapshotQuery.refetch(); versionQuery.refetch(); queueQuery.refetch();
    healthDetailQuery.refetch(); securityQuery.refetch();
    auditQuery.refetch(); backupsQuery.refetch(); taskStatusQuery.refetch();
  };

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("runtime.kernel")}
        title={t("runtime.title")}
        subtitle={t("runtime.subtitle")}
        isFetching={snapshotQuery.isFetching}
        onRefresh={refreshAll}
        icon={<Activity className="h-4 w-4" />}
        helpText={t("runtime.help")}
      />

      {snapshotQuery.isLoading ? (
        <div className="grid gap-4 grid-cols-2 md:grid-cols-4 stagger-children">
          {[1, 2, 3, 4].map(i => <CardSkeleton key={i} />)}
        </div>
      ) : (
        <>
          {/* ── KPI Cards ── */}
          <div className="grid grid-cols-2 gap-2 sm:gap-4 xl:grid-cols-4 stagger-children">
            {[
              { icon: Timer, label: t("runtime.system_uptime"), value: uptimeStr, color: "text-success", bg: "bg-success/10" },
              { icon: Layers, label: t("runtime.active_agents"), value: `${status?.active_agent_count ?? 0} / ${status?.agent_count ?? 0}`, color: "text-brand", bg: "bg-brand/10" },
              { icon: Monitor, label: t("runtime.sessions"), value: String(status?.session_count ?? 0), color: "text-purple-500", bg: "bg-purple-500/10" },
              { icon: HardDrive, label: t("runtime.memory_used"), value: status?.memory_used_mb ? `${status.memory_used_mb} MB` : "-", color: "text-warning", bg: "bg-warning/10" },
            ].map((kpi, i) => (
              <Card key={i} hover padding="md">
                <div className="flex items-center justify-between">
                  <span className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">{kpi.label}</span>
                  <div className={`w-8 h-8 rounded-lg ${kpi.bg} flex items-center justify-center`}>
                    <kpi.icon className={`w-4 h-4 ${kpi.color}`} />
                  </div>
                </div>
                <p className={`text-2xl sm:text-3xl font-black tracking-tight mt-1 sm:mt-2 ${kpi.color}`}>{kpi.value}</p>
              </Card>
            ))}
          </div>

          {/* ── Engine Info + Runtime Config ── */}
          <div className="grid gap-3 sm:gap-6 md:grid-cols-2 stagger-children">
            <Card padding="lg">
              <div className="flex items-center gap-2 mb-5">
                <div className="w-8 h-8 rounded-lg bg-brand/10 flex items-center justify-center"><Cpu className="h-4 w-4 text-brand" /></div>
                <h2 className="text-sm font-black tracking-tight uppercase">{t("runtime.engine")}</h2>
              </div>
              <div className="space-y-3">
                <InfoRow icon={Activity} label={t("runtime.engine_version")} value={version?.version || status?.version || t("common.unknown")} mono color="font-bold text-brand" />
                <InfoRow icon={GitCommit} label={t("runtime.git_hash")} value={version?.git_sha ? version.git_sha.slice(0, 12) : "-"} mono />
                <InfoRow icon={Calendar} label={t("runtime.build_time")} value={version?.build_date || "-"} />
                <InfoRow icon={FileText} label={t("runtime.rust_version")} value={version?.rust_version || "-"} mono />
                <InfoRow icon={Server} label={t("runtime.platform")} value={version?.platform && version?.arch ? `${version.platform} / ${version.arch}` : "-"} />
                <InfoRow icon={Globe} label={t("runtime.hostname")} value={version?.hostname || "-"} />
              </div>
            </Card>

            <Card padding="lg">
              <div className="flex items-center gap-2 mb-5">
                <div className="w-8 h-8 rounded-lg bg-purple-500/10 flex items-center justify-center"><Settings className="h-4 w-4 text-purple-500" /></div>
                <h2 className="text-sm font-black tracking-tight uppercase">{t("runtime.config")}</h2>
              </div>
              <div className="space-y-3">
                <InfoRow icon={Box} label={t("runtime.default_provider")} value={status?.default_provider || "-"} color="font-bold" />
                <InfoRow icon={Cpu} label={t("runtime.default_model")} value={status?.default_model || "-"} mono />
                <InfoRow icon={Network} label={t("runtime.api_listen")} value={status?.api_listen || "-"} mono />
                <InfoRow icon={FolderOpen} label={t("runtime.home_dir")} value={status?.home_dir || "-"} mono />
                <InfoRow icon={Gauge} label={t("runtime.log_level")} value={
                  status?.log_level ? <Badge variant="info">{status.log_level}</Badge> : "-"
                } />
                <InfoRow icon={Globe} label={t("runtime.network_enabled")} value={
                  status?.network_enabled !== undefined
                    ? <Badge variant={status.network_enabled ? "success" : "default"}>{status.network_enabled ? t("runtime.enabled") : t("runtime.disabled")}</Badge>
                    : "-"
                } />
              </div>
            </Card>
          </div>

          {/* ── Health Detail + Security Posture ── */}
          <div className="grid gap-3 sm:gap-6 md:grid-cols-2 stagger-children">
            {/* Health Detail */}
            <Card padding="lg">
              <div className="flex items-center gap-2 mb-5">
                <div className={`w-8 h-8 rounded-lg ${allHealthy ? "bg-success/10" : "bg-warning/10"} flex items-center justify-center`}>
                  <HeartPulse className={`h-4 w-4 ${allHealthy ? "text-success" : "text-warning"}`} />
                </div>
                <h2 className="text-sm font-black tracking-tight uppercase">{t("runtime.health_checks")}</h2>
                <Badge variant={allHealthy ? "success" : "warning"} className="ml-auto">
                  {allHealthy ? t("runtime.all_passed") : t("runtime.degraded")}
                </Badge>
              </div>

              {healthChecks.length > 0 && (
                <div className="space-y-2 mb-4">
                  {healthChecks.map((check: HealthCheck) => {
                    const ok = check.status === "ok" || check.status === "pass" || check.status === "healthy";
                    return (
                      <div key={check.name} className="flex items-center gap-2.5">
                        {ok ? <CheckCircle2 className="w-3.5 h-3.5 text-success shrink-0" /> : <XCircle className="w-3.5 h-3.5 text-error shrink-0" />}
                        <span className="text-xs flex-1">{check.name}</span>
                        <Badge variant={ok ? "success" : "error"}>{check.status}</Badge>
                      </div>
                    );
                  })}
                </div>
              )}

              {hd && (
                <div className="space-y-3 pt-3 border-t border-border-subtle">
                  <InfoRow icon={Database} label={t("runtime.database")} value={
                    <Badge variant={hd.database === "connected" ? "success" : "error"}>{hd.database || "-"}</Badge>
                  } />
                  <InfoRow icon={Brain} label={t("runtime.embedding")} value={
                    hd.memory?.embedding_available
                      ? `${hd.memory.embedding_provider || ""} / ${hd.memory.embedding_model || ""}`
                      : t("runtime.disabled")
                  } />
                  <InfoRow icon={Brain} label={t("runtime.proactive_memory")} value={
                    <Badge variant={hd.memory?.proactive_memory_enabled ? "success" : "default"}>
                      {hd.memory?.proactive_memory_enabled ? t("runtime.enabled") : t("runtime.disabled")}
                    </Badge>
                  } />
                  <InfoRow icon={AlertTriangle} label={t("runtime.panics")} value={String(hd.panic_count ?? 0)} color={hd.panic_count ? "text-error font-bold" : ""} />
                  <InfoRow icon={RotateCcw} label={t("runtime.restarts")} value={String(hd.restart_count ?? 0)} />
                  {(hd.config_warnings?.length ?? 0) > 0 && (
                    <div className="mt-2">
                      <p className="text-[10px] font-bold uppercase text-warning mb-1">{t("runtime.config_warnings")}</p>
                      {hd.config_warnings!.map((w, i) => (
                        <p key={i} className="text-xs text-warning/80 ml-4">- {w}</p>
                      ))}
                    </div>
                  )}
                </div>
              )}
            </Card>

            {/* Security Posture */}
            <Card padding="lg">
              <div className="flex items-center gap-2 mb-5">
                <div className="w-8 h-8 rounded-lg bg-brand/10 flex items-center justify-center"><Shield className="h-4 w-4 text-brand" /></div>
                <h2 className="text-sm font-black tracking-tight uppercase">{t("runtime.security")}</h2>
                {security?.total_features != null && (
                  <Badge variant="brand" className="ml-auto">{t("runtime.features_enabled", { count: security.total_features })}</Badge>
                )}
              </div>

              {security ? (
                <div className="space-y-4">
                  {security.core_protections && (
                    <div>
                      <p className="text-[10px] font-bold uppercase tracking-wider text-text-dim/50 mb-1">{t("runtime.core_protections")}</p>
                      <div className="grid grid-cols-2 gap-x-3">
                        {Object.entries(security.core_protections).map(([k, v]) => (
                          <ProtectionBadge key={k} name={k} enabled={v} />
                        ))}
                      </div>
                    </div>
                  )}

                  <div className="pt-3 border-t border-border-subtle grid grid-cols-2 gap-3">
                    {security.configurable?.rate_limiter && (
                      <div>
                        <p className="text-[10px] font-bold uppercase text-text-dim/50">{t("runtime.rate_limiter")}</p>
                        <Badge variant={security.configurable.rate_limiter.enabled ? "success" : "default"} className="mt-1">
                          {security.configurable.rate_limiter.enabled ? `${security.configurable.rate_limiter.tokens_per_minute} ${t("runtime.tokens_per_min")}` : t("runtime.disabled")}
                        </Badge>
                      </div>
                    )}
                    {security.configurable?.auth && (
                      <div>
                        <p className="text-[10px] font-bold uppercase text-text-dim/50">{t("runtime.auth_mode")}</p>
                        <Badge variant="info" className="mt-1">{security.configurable.auth.mode || "-"}</Badge>
                      </div>
                    )}
                    {security.configurable?.websocket_limits && (
                      <div>
                        <p className="text-[10px] font-bold uppercase text-text-dim/50">{t("runtime.websocket_limits")}</p>
                        <p className="text-xs mt-1">{security.configurable.websocket_limits.max_per_ip} {t("runtime.max_per_ip")}</p>
                      </div>
                    )}
                    {security.configurable?.wasm_sandbox && (
                      <div>
                        <p className="text-[10px] font-bold uppercase text-text-dim/50">{t("runtime.wasm_sandbox")}</p>
                        <p className="text-xs mt-1">{t("runtime.timeout")}: {security.configurable.wasm_sandbox.default_timeout_secs}s</p>
                      </div>
                    )}
                  </div>

                  {security.monitoring && (
                    <div className="pt-3 border-t border-border-subtle space-y-2">
                      {security.monitoring.audit_trail && (
                        <div className="flex items-center gap-2">
                          <Eye className="w-3.5 h-3.5 text-text-dim/40" />
                          <span className="text-xs flex-1">{t("runtime.audit_trail")}</span>
                          <Badge variant={security.monitoring.audit_trail.enabled ? "success" : "default"}>
                            {security.monitoring.audit_trail.algorithm || "-"}
                          </Badge>
                        </div>
                      )}
                      {security.monitoring.taint_tracking && (
                        <div className="flex items-center gap-2">
                          <Lock className="w-3.5 h-3.5 text-text-dim/40" />
                          <span className="text-xs flex-1">{t("runtime.taint_tracking")}</span>
                          <span className="text-xs text-text-dim">{security.monitoring.taint_tracking.tracked_labels?.length ?? 0} labels</span>
                        </div>
                      )}
                      {security.monitoring.manifest_signing && (
                        <div className="flex items-center gap-2">
                          <ShieldCheck className="w-3.5 h-3.5 text-text-dim/40" />
                          <span className="text-xs flex-1">{t("runtime.manifest_signing")}</span>
                          <Badge variant="success">{security.monitoring.manifest_signing.algorithm || "-"}</Badge>
                        </div>
                      )}
                    </div>
                  )}
                </div>
              ) : (
                <p className="text-xs text-text-dim">{t("common.loading")}</p>
              )}
            </Card>
          </div>

          {/* ── Task Queue + Audit Trail ── */}
          <div className="grid gap-3 sm:gap-6 md:grid-cols-2 stagger-children">
            {/* Task Queue */}
            <Card padding="lg">
              <div className="flex items-center gap-2 mb-5">
                <div className="w-8 h-8 rounded-lg bg-warning/10 flex items-center justify-center"><Zap className="h-4 w-4 text-warning" /></div>
                <h2 className="text-sm font-black tracking-tight uppercase">{t("runtime.task_queue")}</h2>
              </div>

              {taskStatus && (
                <div className="grid grid-cols-5 gap-2 text-center mb-4">
                  {[
                    { label: t("runtime.total_tasks"), value: taskStatus.total ?? 0, color: "text-text" },
                    { label: t("runtime.pending_count"), value: taskStatus.pending ?? 0, color: "text-warning" },
                    { label: t("runtime.in_progress_count"), value: taskStatus.in_progress ?? 0, color: "text-brand" },
                    { label: t("runtime.completed_count"), value: taskStatus.completed ?? 0, color: "text-success" },
                    { label: t("runtime.failed_count"), value: taskStatus.failed ?? 0, color: taskStatus.failed ? "text-error" : "text-text-dim" },
                  ].map(s => (
                    <div key={s.label}>
                      <p className={`text-lg font-black ${s.color}`}>{s.value}</p>
                      <p className="text-[8px] text-text-dim uppercase leading-tight">{s.label}</p>
                    </div>
                  ))}
                </div>
              )}

              {lanes.length > 0 && (
                <div className="space-y-2.5 mb-4">
                  {lanes.map((lane) => {
                    const active = lane.active ?? 0;
                    const capacity = lane.capacity ?? 1;
                    const pct = capacity > 0 ? Math.min((active / capacity) * 100, 100) : 0;
                    const color = pct >= 80 ? "bg-error" : pct >= 50 ? "bg-warning" : "bg-brand";
                    return (
                      <div key={lane.lane ?? "default"}>
                        <div className="flex items-center justify-between mb-1">
                          <span className="text-xs font-medium">{lane.lane || "default"}</span>
                          <span className="text-xs text-text-dim font-mono">{active} / {capacity}</span>
                        </div>
                        <div className="h-1.5 rounded-full bg-main overflow-hidden">
                          <div className={`h-full rounded-full ${color} transition-all duration-500`} style={{ width: `${pct}%` }} />
                        </div>
                      </div>
                    );
                  })}
                </div>
              )}

              {tasks.length > 0 ? (
                <div className="space-y-1.5 max-h-40 overflow-y-auto">
                  {tasks.slice(0, 10).map((task: TaskQueueItem) => (
                    <div key={task.id} className="flex items-center gap-2 text-xs py-1 px-2 rounded-lg bg-main/30">
                      <Badge variant={task.status === "failed" ? "error" : task.status === "completed" ? "success" : task.status === "in_progress" ? "brand" : "warning"}>
                        {task.status || "-"}
                      </Badge>
                      <span className="flex-1 truncate font-mono text-[10px]">{task.id?.slice(0, 12)}</span>
                      {task.status === "failed" && (
                        <button onClick={() => retryTaskMutation.mutate(task.id!)} className="text-brand hover:text-brand/80 text-[10px] font-bold">{t("runtime.retry")}</button>
                      )}
                      {(task.status === "pending" || task.status === "in_progress") && (
                        <button onClick={() => deleteTaskMutation.mutate(task.id!)} className="text-error hover:text-error/80 text-[10px] font-bold">{t("runtime.cancel_task")}</button>
                      )}
                    </div>
                  ))}
                </div>
              ) : (
                <p className="text-xs text-text-dim">{t("runtime.no_tasks")}</p>
              )}

              {queueConfig && (
                <div className="mt-4 pt-3 border-t border-border-subtle">
                  <p className="text-[10px] font-bold uppercase tracking-wider text-text-dim/50 mb-2">{t("runtime.queue_config")}</p>
                  <div className="grid grid-cols-3 gap-2 text-center">
                    <div>
                      <p className="text-lg font-black text-brand">{queueConfig.max_depth_per_agent ?? "-"}</p>
                      <p className="text-[9px] text-text-dim uppercase">{t("runtime.max_depth_agent")}</p>
                    </div>
                    <div>
                      <p className="text-lg font-black text-brand">{queueConfig.max_depth_global ?? "-"}</p>
                      <p className="text-[9px] text-text-dim uppercase">{t("runtime.max_depth_global")}</p>
                    </div>
                    <div>
                      <p className="text-lg font-black text-brand">{queueConfig.task_ttl_secs ? `${queueConfig.task_ttl_secs}s` : "-"}</p>
                      <p className="text-[9px] text-text-dim uppercase">{t("runtime.task_ttl")}</p>
                    </div>
                  </div>
                </div>
              )}
            </Card>

            {/* Audit Trail */}
            <Card padding="lg">
              <div className="flex items-center gap-2 mb-5">
                <div className="w-8 h-8 rounded-lg bg-brand/10 flex items-center justify-center"><Eye className="h-4 w-4 text-brand" /></div>
                <h2 className="text-sm font-black tracking-tight uppercase">{t("runtime.audit")}</h2>
                {auditValid !== undefined && (
                  <Badge variant={auditValid ? "success" : "error"} className="ml-auto">
                    {auditValid ? t("runtime.audit_valid") : t("runtime.audit_invalid")}
                  </Badge>
                )}
              </div>

              {auditEntries.length > 0 ? (
                <div className="space-y-1.5 max-h-64 overflow-y-auto">
                  {auditEntries.map((entry: AuditEntry, i: number) => (
                    <div key={entry.seq ?? i} className="flex items-start gap-2 text-xs py-1.5 px-2 rounded-lg bg-main/30">
                      <Badge variant={entry.outcome === "ok" ? "success" : entry.outcome === "denied" ? "error" : "warning"}>
                        {entry.outcome || "-"}
                      </Badge>
                      <div className="flex-1 min-w-0">
                        <div className="flex items-center gap-2">
                          <span className="font-bold text-[10px] truncate">{entry.action || "-"}</span>
                          {entry.agent_id && entry.agent_id !== "system" && (
                            <span className="text-[9px] text-text-dim truncate">@{entry.agent_id.slice(0, 8)}</span>
                          )}
                        </div>
                        <p className="text-[10px] text-text-dim truncate">{entry.detail || "-"}</p>
                      </div>
                      <span className="text-[9px] text-text-dim shrink-0">
                        {entry.timestamp ? new Date(entry.timestamp).toLocaleTimeString() : "-"}
                      </span>
                    </div>
                  ))}
                </div>
              ) : (
                <p className="text-xs text-text-dim">{t("runtime.no_audit")}</p>
              )}

              {auditVerifyQuery.data && (
                <div className="mt-3 pt-3 border-t border-border-subtle flex items-center justify-between">
                  <span className="text-xs text-text-dim">{t("runtime.audit_entries", { count: auditVerifyQuery.data.entries ?? 0 })}</span>
                  <Button variant="ghost" size="sm" onClick={() => auditVerifyQuery.refetch()}>
                    {t("runtime.audit_verify")}
                  </Button>
                </div>
              )}
            </Card>
          </div>

          {/* ── Backups + Resource Summary ── */}
          <div className="grid gap-3 sm:gap-6 md:grid-cols-2 stagger-children">
            {/* Backup Management */}
            <Card padding="lg">
              <div className="flex items-center gap-2 mb-5">
                <div className="w-8 h-8 rounded-lg bg-brand/10 flex items-center justify-center"><Archive className="h-4 w-4 text-brand" /></div>
                <h2 className="text-sm font-black tracking-tight uppercase">{t("runtime.backups")}</h2>
                <div className="ml-auto">
                  <Button variant="secondary" size="sm" leftIcon={<Download className="w-3 h-3" />} isLoading={backupMutation.isPending} onClick={() => backupMutation.mutate()}>
                    {t("runtime.create_backup")}
                  </Button>
                </div>
              </div>

              {backupMutation.isSuccess && <p className="text-xs text-success mb-2">{t("runtime.backup_created")}</p>}
              {backupMutation.isError && <p className="text-xs text-error mb-2">{t("runtime.backup_error")}</p>}
              {restoreMutation.isSuccess && <p className="text-xs text-success mb-2">{t("runtime.restore_success")}</p>}
              {restoreMutation.isError && <p className="text-xs text-error mb-2">{t("runtime.restore_error")}</p>}

              {backups.length > 0 ? (
                <div className="space-y-2 max-h-48 overflow-y-auto">
                  {backups.map((b: BackupItem) => (
                    <div key={b.filename} className="flex items-center gap-2 text-xs py-2 px-3 rounded-lg bg-main/30">
                      <Archive className="w-3.5 h-3.5 text-text-dim/40 shrink-0" />
                      <div className="flex-1 min-w-0">
                        <p className="font-mono text-[10px] truncate">{b.filename}</p>
                        <p className="text-[9px] text-text-dim">
                          {formatBytes(b.size_bytes)}
                          {b.created_at && ` · ${new Date(b.created_at).toLocaleDateString()}`}
                        </p>
                      </div>
                      <button
                        onClick={() => b.filename && restoreMutation.mutate(b.filename)}
                        className="text-brand hover:text-brand/80 text-[10px] font-bold shrink-0"
                        disabled={restoreMutation.isPending}
                      >
                        {t("runtime.restore")}
                      </button>
                      <button
                        onClick={() => b.filename && deleteBackupMutation.mutate(b.filename)}
                        className="text-error hover:text-error/80 shrink-0"
                      >
                        <Trash2 className="w-3 h-3" />
                      </button>
                    </div>
                  ))}
                </div>
              ) : (
                <p className="text-xs text-text-dim">{t("runtime.no_backups")}</p>
              )}
            </Card>

            {/* Resource Summary */}
            <Card padding="lg">
              <div className="flex items-center gap-2 mb-5">
                <div className="w-8 h-8 rounded-lg bg-success/10 flex items-center justify-center"><Layers className="h-4 w-4 text-success" /></div>
                <h2 className="text-sm font-black tracking-tight uppercase">{t("runtime.resources")}</h2>
              </div>
              <div className="grid grid-cols-2 gap-4">
                {[
                  { label: t("runtime.providers"), value: snapshot?.providers?.length ?? 0, sub: `${snapshot?.providers?.filter(p => isProviderAvailable(p.auth_status)).length ?? 0} ${t("status.configured").toLowerCase()}`, color: "text-brand" },
                  { label: t("runtime.channels"), value: snapshot?.channels?.length ?? 0, sub: `${snapshot?.channels?.filter(c => c.configured).length ?? 0} ${t("status.configured").toLowerCase()}`, color: "text-purple-500" },
                  { label: t("runtime.skills"), value: snapshot?.skillCount ?? 0, sub: t("status.active").toLowerCase(), color: "text-success" },
                  { label: t("runtime.workflows"), value: snapshot?.workflowCount ?? 0, sub: t("common.config").toLowerCase(), color: "text-warning" },
                ].map((item) => (
                  <div key={item.label} className="text-center">
                    <p className={`text-2xl font-black ${item.color}`}>{item.value}</p>
                    <p className="text-xs font-bold">{item.label}</p>
                    <p className="text-[10px] text-text-dim">{item.sub}</p>
                  </div>
                ))}
              </div>

              <div className="mt-5 pt-4 border-t border-border-subtle flex items-center justify-between">
                <div className="flex items-center gap-2">
                  <span className="relative flex h-2.5 w-2.5">
                    <span className="absolute inline-flex h-full w-full rounded-full bg-success opacity-75 animate-pulse" />
                    <span className="relative inline-flex rounded-full h-2.5 w-2.5 bg-success" />
                  </span>
                  <span className="text-xs font-bold">{t("runtime.status")}</span>
                </div>
                <Badge variant="success">{t("status.nominal")}</Badge>
              </div>
            </Card>
          </div>

          {/* ── Server Control ── */}
          <Card padding="lg">
            <div className="flex items-center gap-2 mb-5">
              <div className="w-8 h-8 rounded-lg bg-error/10 flex items-center justify-center"><Server className="h-4 w-4 text-error" /></div>
              <h2 className="text-sm font-black tracking-tight uppercase">{t("runtime.server_control")}</h2>
            </div>
            <div className="flex flex-wrap gap-3">
              <Button variant="secondary" size="sm" leftIcon={<RefreshCw className="w-3.5 h-3.5" />} isLoading={reloadMutation.isPending} onClick={() => reloadMutation.mutate()}>
                {t("runtime.reload_config")}
              </Button>
              <Button variant="secondary" size="sm" leftIcon={<Clock className="w-3.5 h-3.5" />} isLoading={cleanupMutation.isPending} onClick={() => cleanupMutation.mutate()}>
                {t("runtime.cleanup_sessions")}
              </Button>
              <Button variant="danger" size="sm" leftIcon={<Power className="w-3.5 h-3.5" />} onClick={() => setShowShutdownConfirm(true)}>
                {t("runtime.shutdown")}
              </Button>
            </div>
            {reloadResult && <p className="text-xs text-success mt-3">{t("runtime.reload_success", { status: reloadResult })}</p>}
            {reloadMutation.isError && <p className="text-xs text-error mt-3">{t("runtime.reload_error")}</p>}
            {cleanupMutation.isSuccess && <p className="text-xs text-success mt-3">{t("runtime.sessions_deleted", { count: cleanupMutation.data?.sessions_deleted ?? 0 })}</p>}
            {shutdownMutation.isError && <p className="text-xs text-error mt-3">{t("runtime.shutdown_error")}</p>}
          </Card>
        </>
      )}

      {/* Shutdown Confirm Dialog */}
      <ConfirmDialog
        isOpen={showShutdownConfirm}
        title={t("runtime.shutdown_confirm_title")}
        message={t("runtime.shutdown_confirm_desc")}
        confirmLabel={t("runtime.shutdown_confirm")}
        tone="destructive"
        onConfirm={() => shutdownMutation.mutate()}
        onClose={() => setShowShutdownConfirm(false)}
      />
    </div>
  );
}
