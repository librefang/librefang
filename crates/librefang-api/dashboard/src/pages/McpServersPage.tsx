import { Component, useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { AnimatePresence, motion } from "motion/react";
import { tabContent } from "../lib/motion";
import {
  type McpServerConfigured, type McpServerConnected, type McpTransport,
  type McpCatalogEntry,
} from "../api";
import { useMcpServers, useMcpCatalog, useMcpHealth, mcpQueries } from "../lib/queries/mcp";
import {
  useAddMcpServer,
  useUpdateMcpServer,
  useDeleteMcpServer,
  useReloadMcp,
  useStartMcpAuth,
  useRevokeMcpAuth,
} from "../lib/mutations/mcp";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { PageHeader } from "../components/ui/PageHeader";
import { ListSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { DrawerPanel } from "../components/ui/DrawerPanel";
import { ConfirmDialog } from "../components/ui/ConfirmDialog";
import { Input } from "../components/ui/Input";
import { useUIStore } from "../lib/store";
import { useCreateShortcut } from "../lib/useCreateShortcut";
import {
  Plug, Plus, X, Trash2, Settings, ChevronDown, ChevronUp, Wrench, Terminal, Globe, Radio,
  Shield, ShieldCheck, ShieldAlert, ShieldX, Check, ExternalLink,
  Search, Clock, Filter, Store, Key, Download, RefreshCw, Activity,
  ShieldHalf,
} from "lucide-react";
import { TaintPolicyEditor } from "../components/TaintPolicyEditor";
import { DynamicIcon } from "lucide-react/dynamic";
import type { IconName } from "lucide-react/dynamic";

// Wraps `DynamicIcon` so a catalog entry with a stale or mistyped
// `lucide:xxx` name (backend-controlled, but still human-edited) falls back
// to a neutral icon instead of throwing and blowing up the surrounding card.
// `DynamicIcon` lazy-imports the icon module — if the name isn't a real
// lucide icon the import rejects and the `Suspense` fallback doesn't catch
// that; it bubbles up as a render error, which this boundary converts into
// `fallback`.
class DynamicIconBoundary extends Component<
  { children: ReactNode; fallback: ReactNode },
  { hasError: boolean }
> {
  state = { hasError: false };
  static getDerivedStateFromError() {
    return { hasError: true };
  }
  componentDidCatch(error: Error) {
    // eslint-disable-next-line no-console
    console.warn("CatalogIcon: DynamicIcon failed to load, using fallback.", error);
  }
  render() {
    return this.state.hasError ? this.props.fallback : this.props.children;
  }
}

function CatalogIcon({ icon, className }: { icon: string; className?: string }) {
  if (icon.startsWith("lucide:")) {
    const name = icon.slice("lucide:".length) as IconName;
    const fallback = <Plug className={className} />;
    return (
      <DynamicIconBoundary fallback={fallback}>
        <DynamicIcon name={name} className={className} fallback={() => fallback} />
      </DynamicIconBoundary>
    );
  }
  return <span className="text-xl">{icon}</span>;
}

type TransportType = "stdio" | "sse" | "http";
type StatusFilter = "all" | "connected" | "disconnected";

interface LabeledItem {
  id: string;
  value: string;
}

interface ServerFormState {
  name: string;
  transportType: TransportType;
  command: string;
  args: LabeledItem[];
  url: string;
  timeout: number;
  env: LabeledItem[];
  headers: string;
}

function makeLocalId() {
  return typeof crypto !== "undefined" && typeof crypto.randomUUID === "function"
    ? crypto.randomUUID()
    : `tmp-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

function errorMessage(error: unknown, fallback: string) {
  return error instanceof Error ? error.message : fallback;
}

const defaultForm: ServerFormState = {
  name: "",
  transportType: "stdio",
  command: "",
  args: [],
  url: "",
  timeout: 30,
  env: [],
  headers: "",
};

// Prefer the backend-assigned id, fall back to the user-facing name.
// Every URL operation (update / delete / auth / reconnect) should use this.
function serverIdOf(server: McpServerConfigured): string {
  return server.id ?? server.name;
}

function serverIdentityOf(server: Pick<McpServerConfigured, "id" | "name"> | Pick<McpServerConnected, "name">): string {
  return "id" in server && server.id ? server.id : server.name;
}

function formToPayload(form: ServerFormState): McpServerConfigured {
  let transport: McpTransport;
  if (form.transportType === "stdio") {
    transport = {
      type: "stdio",
      command: form.command,
      args: form.args.map(item => item.value.trim()).filter(Boolean),
    };
  } else {
    transport = { type: form.transportType, url: form.url };
  }

  const headers = form.headers.split("\n").map(s => s.trim()).filter(Boolean);
  const result: McpServerConfigured = {
    name: form.name,
    transport,
    timeout_secs: form.timeout || 30,
    env: form.env.map(item => item.value.trim()).filter(Boolean),
  };
  // Only include headers if user explicitly entered values, to avoid
  // overwriting server-side headers that the list API may not return.
  if (headers.length > 0) {
    result.headers = headers;
  }
  return result;
}

function configuredToForm(server: McpServerConfigured): ServerFormState {
  const transport = server.transport ?? { type: "stdio" as const };
  return {
    name: server.name,
    transportType: transport.type ?? "stdio",
    command: transport.command ?? "",
    args: (transport.args ?? []).map(v => ({ id: makeLocalId(), value: v })),
    url: transport.url ?? "",
    timeout: server.timeout_secs ?? 30,
    env: (server.env ?? []).map(v => ({ id: makeLocalId(), value: v })),
    headers: (server.headers ?? []).join("\n"),
  };
}

function getTransportType(server: McpServerConfigured): TransportType {
  return server.transport?.type ?? "stdio";
}

function getTransportDetail(server: McpServerConfigured): string {
  if (!server.transport) return "\u2014";
  if (server.transport.type === "stdio") {
    return `${server.transport.command ?? ""} ${(server.transport.args ?? []).join(" ")}`.trim();
  }
  return server.transport.url ?? "\u2014";
}

// ── ArgsEditor ──────────────────────────────────────────────────────

function ArgsEditor({ items, onChange }: { items: LabeledItem[]; onChange: (items: LabeledItem[]) => void }) {
  const { t } = useTranslation();
  const inputRefs = useRef<(HTMLInputElement | null)[]>([]);

  function addItem() {
    const next = [...items, { id: makeLocalId(), value: "" }];
    onChange(next);
    // Focus the newly added input after render
    setTimeout(() => {
      inputRefs.current[next.length - 1]?.focus();
    }, 0);
  }

  function removeItem(idx: number) {
    onChange(items.filter((_, i) => i !== idx));
  }

  function updateItem(idx: number, value: string) {
    const next = [...items];
    next[idx] = { ...next[idx], value };
    onChange(next);
  }

  return (
    <div className="space-y-1.5">
      {items.map((item, idx) => (
        <div key={item.id} className="flex items-center gap-1.5">
          <input
            ref={el => { inputRefs.current[idx] = el; }}
            type="text"
            value={item.value}
            onChange={(e) => updateItem(idx, e.target.value)}
            className="flex-1 rounded-lg border border-border-subtle bg-surface px-3 py-1.5 text-sm font-mono text-text-main placeholder:text-text-dim/40 focus:border-brand focus:outline-none focus:ring-2 focus:ring-brand/10 hover:border-brand/20 transition-colors duration-200 shadow-sm"
          />
          <button
            type="button"
            onClick={() => removeItem(idx)}
            className="shrink-0 flex items-center justify-center w-6 h-6 rounded-md text-text-dim hover:text-error hover:bg-error/8 transition-colors"
            aria-label={t("mcp.remove_argument")}
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
      ))}
      <button
        type="button"
        onClick={addItem}
        className="flex items-center gap-1 text-[10px] font-bold text-text-dim hover:text-brand transition-colors py-0.5"
      >
        <Plus className="h-3 w-3" />
        {t("mcp.add_argument")}
      </button>
    </div>
  );
}

// ── EnvEditor ───────────────────────────────────────────────────────

function EnvEditor({ items, onChange }: { items: LabeledItem[]; onChange: (items: LabeledItem[]) => void }) {
  const { t } = useTranslation();
  const inputRefs = useRef<(HTMLInputElement | null)[]>([]);

  function addItem() {
    const next = [...items, { id: makeLocalId(), value: "" }];
    onChange(next);
    setTimeout(() => {
      inputRefs.current[next.length - 1]?.focus();
    }, 0);
  }

  function removeItem(idx: number) {
    onChange(items.filter((_, i) => i !== idx));
  }

  function updateItem(idx: number, value: string) {
    const next = [...items];
    next[idx] = { ...next[idx], value };
    onChange(next);
  }

  return (
    <div className="space-y-1.5">
      {items.map((item, idx) => (
        <div key={item.id} className="flex items-center gap-1.5">
          <input
            ref={el => { inputRefs.current[idx] = el; }}
            type="text"
            value={item.value}
            onChange={(e) => updateItem(idx, e.target.value)}
            placeholder={t("mcp.env_placeholder")}
            className="flex-1 rounded-lg border border-border-subtle bg-surface px-3 py-1.5 text-sm font-mono text-text-main placeholder:text-text-dim/40 focus:border-brand focus:outline-none focus:ring-2 focus:ring-brand/10 hover:border-brand/20 transition-colors duration-200 shadow-sm"
          />
          <button
            type="button"
            onClick={() => removeItem(idx)}
            className="shrink-0 flex items-center justify-center w-6 h-6 rounded-md text-text-dim hover:text-error hover:bg-error/8 transition-colors"
            aria-label={t("mcp.remove_variable")}
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
      ))}
      <button
        type="button"
        onClick={addItem}
        className="flex items-center gap-1 text-[10px] font-bold text-text-dim hover:text-brand transition-colors py-0.5"
      >
        <Plus className="h-3 w-3" />
        {t("mcp.add_variable")}
      </button>
    </div>
  );
}

// ── Transport Icon ───────────────────────────────────────────────────

function TransportIcon({ type }: { type: TransportType }) {
  switch (type) {
    case "stdio": return <Terminal className="h-4 w-4" />;
    case "sse": return <Radio className="h-4 w-4" />;
    case "http": return <Globe className="h-4 w-4" />;
  }
}

// ── Auth Badge ──────────────────────────────────────────────────────

function AuthBadge({
  server,
  onAuthSuccess,
}: {
  server: McpServerConfigured;
  onAuthSuccess?: () => void;
}) {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);
  const queryClient = useQueryClient();
  const authState = server.auth_state?.state ?? "not_required";
  const [polling, setPolling] = useState(false);
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const startAuthMutation = useStartMcpAuth();
  const revokeAuthMutation = useRevokeMcpAuth();
  const serverIdentity = serverIdentityOf(server);

  useEffect(() => {
    if (polling) {
      pollRef.current = setInterval(async () => {
        try {
          const status = await queryClient.fetchQuery(mcpQueries.authStatus(serverIdentity));
          if (status.auth.state === "authorized") {
            setPolling(false);
            queryClient.invalidateQueries({ queryKey: mcpQueries.servers().queryKey });
            queryClient.invalidateQueries({ queryKey: mcpQueries.health().queryKey });
            onAuthSuccess?.();
          } else if (status.auth.state === "error") {
            setPolling(false);
            addToast(status.auth.message || t("mcp.auth_failed"), "error");
          }
        } catch {
          // ignore transient errors during polling
        }
      }, 2000);
    }
    return () => {
      if (pollRef.current) clearInterval(pollRef.current);
    };
  }, [authState, polling, serverIdentity, onAuthSuccess, addToast, queryClient, t]);

  const handleStartAuth = useCallback(async () => {
    const authWindow = window.open("about:blank", "_blank");
    try {
      const result = await startAuthMutation.mutateAsync(serverIdentity);
      if (authWindow && !authWindow.closed) {
        authWindow.location.href = result.auth_url;
      } else {
        window.location.href = result.auth_url;
      }
      setPolling(true);
      addToast(t("mcp.auth_started"), "info");
    } catch (e: unknown) {
      if (authWindow && !authWindow.closed) {
        authWindow.close();
      }
      addToast(errorMessage(e, t("mcp.auth_start_failed")), "error");
    }
  }, [serverIdentity, startAuthMutation, addToast, t]);

  const handleRevoke = useCallback(async () => {
    try {
      await revokeAuthMutation.mutateAsync(serverIdentity);
      onAuthSuccess?.();
      addToast(t("mcp.auth_revoked"), "success");
    } catch (e: unknown) {
      addToast(errorMessage(e, t("mcp.auth_revoke_failed")), "error");
    }
  }, [serverIdentity, revokeAuthMutation, onAuthSuccess, addToast, t]);

  if (authState === "not_required") return null;

  if (authState === "authorized") {
    return (
      <div className="flex items-center gap-1.5">
        <Badge variant="success" dot>
          <ShieldCheck className="h-3 w-3 mr-1" />
          {t("mcp.auth_authorized")}
        </Badge>
        <button
          onClick={handleRevoke}
          className="text-[10px] font-bold text-text-dim hover:text-error transition-colors"
        >
          {t("mcp.auth_revoke")}
        </button>
      </div>
    );
  }

  if (authState === "needs_auth") {
    return (
      <button
        onClick={handleStartAuth}
        className="inline-flex items-center gap-1 rounded-lg border border-warning/30 bg-warning/5 px-2 py-1 text-[10px] font-bold text-warning hover:bg-warning/10 transition-colors"
      >
        <Shield className="h-3 w-3" />
        {t("mcp.auth_authorize")}
      </button>
    );
  }

  if (authState === "pending_auth" || polling) {
    return (
      <Badge variant="warning" dot className="animate-pulse">
        <Shield className="h-3 w-3 mr-1" />
        {t("mcp.auth_pending")}
      </Badge>
    );
  }

  if (authState === "expired" || authState === "error") {
    return (
      <button
        onClick={handleStartAuth}
        className="inline-flex items-center gap-1 rounded-lg border border-error/30 bg-error/5 px-2 py-1 text-[10px] font-bold text-error hover:bg-error/10 transition-colors"
      >
        <ShieldAlert className="h-3 w-3" />
        {authState === "expired" ? t("mcp.auth_reauthorize") : t("mcp.auth_authorize")}
      </button>
    );
  }

  return (
    <button
      onClick={handleStartAuth}
      className="inline-flex items-center gap-1 rounded-lg border border-warning/30 bg-warning/5 px-2 py-1 text-[10px] font-bold text-warning hover:bg-warning/10 transition-colors"
    >
      <ShieldX className="h-3 w-3" />
      {t("mcp.auth_authorize")}
    </button>
  );
}

// ── Server Card ─────────────────────────────────────────────────────

function ServerCard({
  server,
  conn,
  isExpanded,
  onToggleTools,
  onEdit,
  onEditTaintPolicy,
  onDelete,
  onAuthSuccess,
  t,
}: {
  server: McpServerConfigured;
  conn?: McpServerConnected;
  isExpanded: boolean;
  onToggleTools: () => void;
  onEdit: () => void;
  onEditTaintPolicy: () => void;
  onDelete: () => void;
  onAuthSuccess?: () => void;
  t: (key: string, opts?: any) => string;
}) {
  const isConnected = conn?.connected ?? false;
  const toolsCount = conn?.tools_count ?? 0;
  const [showAllTools, setShowAllTools] = useState(false);

  // Cache transport info — computed once per render, not 3-4x
  const transportType = useMemo(() => getTransportType(server), [server]);
  const transportDetail = useMemo(() => getTransportDetail(server), [server]);

  // Reset "show all" when tools section is collapsed
  useEffect(() => {
    if (!isExpanded) setShowAllTools(false);
  }, [isExpanded]);

  const visibleTools = useMemo(() => {
    if (!conn?.tools) return [];
    if (showAllTools || conn.tools.length <= 5) return conn.tools;
    return conn.tools.slice(0, 5);
  }, [conn?.tools, showAllTools]);

  const hiddenCount = (conn?.tools?.length ?? 0) - visibleTools.length;

  return (
    <Card hover padding="none" className="flex flex-col overflow-hidden group">
      {/* Gradient top bar */}
      <div className={`h-1.5 bg-linear-to-r ${
        isConnected
          ? "from-success via-success/60 to-success/30"
          : "from-error via-error/60 to-error/30"
      }`} />

      <div className="p-5 flex-1 flex flex-col">
        {/* Header */}
        <div className="flex items-start justify-between gap-3 mb-4">
          <div className="flex items-center gap-3 min-w-0">
            <div className={`w-10 h-10 rounded-lg flex items-center justify-center shadow-sm ${
              isConnected
                ? "bg-linear-to-br from-success/10 to-success/5 border border-success/20"
                : "bg-linear-to-br from-brand/10 to-brand/5 border border-brand/20"
            }`}>
              <Plug className={`w-5 h-5 ${isConnected ? "text-success" : "text-brand"}`} />
            </div>
            <div className="min-w-0">
              <h2 className={`text-base font-black truncate transition-colors ${
                isConnected ? "group-hover:text-success" : "group-hover:text-brand"
              }`}>{server.name}</h2>
              <p className="text-[10px] font-black uppercase tracking-widest text-text-dim/60">
                {transportType}
              </p>
            </div>
          </div>
          <Badge variant={isConnected ? "success" : "error"} dot>
            {isConnected ? t("mcp.connected") : t("mcp.disconnected")}
          </Badge>
        </div>

        {/* OAuth auth badge */}
        <AuthBadge server={server} onAuthSuccess={onAuthSuccess} />

        {/* Stats */}
        <div className="grid grid-cols-2 gap-3 mb-4">
          <div className="p-3 rounded-xl bg-linear-to-br from-main/60 to-main/30 border border-border-subtle/50">
            <div className="flex items-center gap-1.5 mb-1">
              <Wrench className={`w-3 h-3 ${isConnected ? "text-success" : "text-brand"}`} />
              <p className="text-[9px] font-black uppercase tracking-wider text-text-dim/70">{t("mcp.tools")}</p>
            </div>
            <p className="text-xl font-black text-text-main">{toolsCount}</p>
          </div>
          <div className="p-3 rounded-xl bg-linear-to-br from-main/60 to-main/30 border border-border-subtle/50">
            <div className="flex items-center gap-1.5 mb-1">
              <Clock className="w-3 h-3 text-warning" />
              <p className="text-[9px] font-black uppercase tracking-wider text-text-dim/70">{t("mcp.timeout")}</p>
            </div>
            <p className="text-xl font-black text-text-main">{server.timeout_secs ?? 30}s</p>
          </div>
        </div>

        {/* Transport badge + detail */}
        <div className="flex items-center gap-2 mb-3">
          <Badge variant="default">
            <TransportIcon type={transportType} />
            <span className="ml-1">{transportType.toUpperCase()}</span>
          </Badge>
        </div>
        <div className="flex items-center gap-2 text-xs mb-2">
          {transportType === "stdio" ? (
            <Terminal className="w-3 h-3 text-text-dim/50 shrink-0" />
          ) : (
            <Globe className="w-3 h-3 text-text-dim/50 shrink-0" />
          )}
          <span className="text-text-dim font-mono text-[10px] truncate">{transportDetail}</span>
        </div>
      </div>

      {/* Tools expand */}
      {toolsCount > 0 && (
        <>
          <button
            onClick={onToggleTools}
            className="flex items-center justify-center gap-1.5 py-2.5 border-t border-border-subtle text-xs font-bold text-text-dim hover:text-brand hover:bg-surface-hover transition-colors"
            aria-expanded={isExpanded}
            aria-label={isExpanded ? t("mcp.hide_tools") : t("mcp.show_tools")}
          >
            {isExpanded ? <ChevronUp className="h-3.5 w-3.5" /> : <ChevronDown className="h-3.5 w-3.5" />}
            {t("mcp.tools")} ({toolsCount})
          </button>
          {isExpanded && conn?.tools && (
            <div className="border-t border-border-subtle px-4 py-3 space-y-2 max-h-64 overflow-y-auto scrollbar-thin">
              {visibleTools.map((tool) => (
                <div key={tool.name} className="p-2.5 rounded-lg bg-main/40 border border-border-subtle/50">
                  <span className="text-xs font-mono font-bold text-text-main">{tool.name}</span>
                  {tool.description && (
                    <p className="text-[10px] text-text-dim leading-snug mt-0.5">{tool.description}</p>
                  )}
                </div>
              ))}
              {hiddenCount > 0 && (
                <button
                  onClick={() => setShowAllTools(true)}
                  className="w-full text-center text-[10px] font-bold text-brand hover:text-brand/80 py-1.5 transition-colors"
                >
                  {t("mcp.show_more_tools", { count: hiddenCount })}
                </button>
              )}
            </div>
          )}
        </>
      )}

      {/* Actions */}
      <div className="flex border-t border-border-subtle">
        <button
          onClick={onEdit}
          className="flex-1 flex items-center justify-center gap-1.5 py-2.5 text-xs font-bold text-text-dim hover:text-brand hover:bg-surface-hover transition-colors rounded-bl-xl sm:rounded-bl-2xl"
          aria-label={t("mcp.edit_server")}
        >
          <Settings className="h-3.5 w-3.5" />
          {t("common.edit")}
        </button>
        <div className="w-px bg-border-subtle" />
        <button
          onClick={onEditTaintPolicy}
          className="flex-1 flex items-center justify-center gap-1.5 py-2.5 text-xs font-bold text-text-dim hover:text-brand hover:bg-surface-hover transition-colors"
          aria-label={t("mcp.edit_taint_policy", "Taint policy")}
          title={t("mcp.edit_taint_policy", "Taint policy")}
        >
          <ShieldHalf className="h-3.5 w-3.5" />
          {t("mcp.taint_policy_short", "Taint")}
        </button>
        <div className="w-px bg-border-subtle" />
        <button
          onClick={onDelete}
          className="flex-1 flex items-center justify-center gap-1.5 py-2.5 text-xs font-bold text-text-dim hover:text-error hover:bg-error/5 transition-colors rounded-br-xl sm:rounded-br-2xl"
          aria-label={t("mcp.delete_server")}
        >
          <Trash2 className="h-3.5 w-3.5" />
          {t("common.delete")}
        </button>
      </div>
    </Card>
  );
}

// ── Main Page ───────────────────────────────────────────────────────

export function McpServersPage() {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);

  const [tab, setTab] = useState<"servers" | "catalog">("servers");
  const [showAddModal, setShowAddModal] = useState(false);
  const [editingServer, setEditingServer] = useState<McpServerConfigured | null>(null);
  const [taintEditingServer, setTaintEditingServer] = useState<McpServerConfigured | null>(null);
  const [deletingServer, setDeletingServer] = useState<McpServerConfigured | null>(null);
  const [expandedTools, setExpandedTools] = useState<Set<string>>(new Set());
  const [form, setForm] = useState<ServerFormState>(defaultForm);
  const [searchQuery, setSearchQuery] = useState("");
  const [statusFilter, setStatusFilter] = useState<StatusFilter>("all");
  const [catalogSearch, setCatalogSearch] = useState("");
  const [installingTemplate, setInstallingTemplate] = useState<McpCatalogEntry | null>(null);
  const [envInputs, setEnvInputs] = useState<Record<string, string>>({});

  useCreateShortcut(() => setShowAddModal(true));

  const serversQuery = useMcpServers();
  const catalogQuery = useMcpCatalog({ enabled: tab === "catalog" });
  const healthQuery = useMcpHealth();

  const addMutation = useAddMcpServer();
  const updateMutation = useUpdateMcpServer();
  const deleteMutation = useDeleteMcpServer();
  const reloadMutation = useReloadMcp();

  const data = serversQuery.data;
  const configured = data?.configured ?? [];
  const connected = data?.connected ?? [];

  // First-time visitors with no servers configured land on the marketplace
  // tab — installing a template is the obvious next step, and the empty
  // "Servers" tab gave them nothing to act on. Only fires once per mount;
  // if the user manually switches back to "servers", we don't override.
  const autoSwitchedRef = useRef(false);
  useEffect(() => {
    if (autoSwitchedRef.current) return;
    if (!serversQuery.isSuccess) return;
    autoSwitchedRef.current = true;
    if (configured.length === 0) setTab("catalog");
  }, [serversQuery.isSuccess, configured.length]);

  const connectedMap = useMemo(() => {
    const map = new Map<string, McpServerConnected>();
    for (const c of connected) map.set(serverIdentityOf(c), c);
    return map;
  }, [connected]);

  // Search + filter
  const filteredServers = useMemo(() => {
    let result = configured;
    if (searchQuery.trim()) {
      const q = searchQuery.toLowerCase();
      result = result.filter(s =>
        s.name.toLowerCase().includes(q) ||
        getTransportDetail(s).toLowerCase().includes(q)
      );
    }
    if (statusFilter !== "all") {
      result = result.filter(s => {
        const isConn = connectedMap.get(serverIdentityOf(s))?.connected ?? false;
        return statusFilter === "connected" ? isConn : !isConn;
      });
    }
    return result;
  }, [configured, searchQuery, statusFilter, connectedMap]);

  const toggleTools = useCallback((server: McpServerConfigured) => {
    const identity = serverIdentityOf(server);
    setExpandedTools(prev => {
      const next = new Set(prev);
      if (next.has(identity)) next.delete(identity);
      else next.add(identity);
      return next;
    });
  }, []);

  function openAdd() {
    setForm(defaultForm);
    setShowAddModal(true);
  }

  const openEdit = useCallback((server: McpServerConfigured) => {
    setForm(configuredToForm(server));
    setEditingServer(server);
  }, []);

  const deleteServer = useCallback((server: McpServerConfigured) => {
    setDeletingServer(server);
  }, []);

  function handleSubmit() {
    const payload = formToPayload(form);
    if (editingServer) {
      updateMutation.mutate(
        { id: serverIdOf(editingServer), server: payload },
        {
          onSuccess: () => {
            setEditingServer(null);
            setForm(defaultForm);
            addToast(t("mcp.update_success"), "success");
          },
          onError: (e: unknown) => addToast(errorMessage(e, t("mcp.update_failed")), "error"),
        },
      );
    } else {
      addMutation.mutate(payload, {
        onSuccess: () => {
          setShowAddModal(false);
          setInstallingTemplate(null);
          setEnvInputs({});
          setForm(defaultForm);
          addToast(t("mcp.add_success"), "success");
        },
        onError: (e: unknown) => addToast(errorMessage(e, t("mcp.add_failed")), "error"),
      });
    }
  }

  function handleReload() {
    reloadMutation.mutate(undefined, {
      onSuccess: () => addToast(t("mcp.reload_success"), "success"),
      onError: (e: unknown) => addToast(errorMessage(e, t("mcp.reload_failed")), "error"),
    });
  }

  const isModalOpen = showAddModal || editingServer !== null;
  const isSubmitting = addMutation.isPending || updateMutation.isPending;

  const updateField = <K extends keyof ServerFormState>(key: K, value: ServerFormState[K]) =>
    setForm(prev => ({ ...prev, [key]: value }));

  // Catalog install: backend expects `{ template_id, credentials }` — the
  // dashboard no longer materializes a full server spec from the template.
  function installFromTemplate(tpl: McpCatalogEntry) {
    const hasEnv = (tpl.required_env ?? []).length > 0;
    if (hasEnv) {
      const defaults: Record<string, string> = {};
      for (const e of tpl.required_env ?? []) defaults[e.name] = "";
      setEnvInputs(defaults);
      setInstallingTemplate(tpl);
    } else {
      addMutation.mutate({ template_id: tpl.id }, {
        onSuccess: () => {
          setShowAddModal(false);
          setInstallingTemplate(null);
          setEnvInputs({});
          setForm(defaultForm);
          addToast(t("mcp.add_success"), "success");
        },
        onError: (e: unknown) => addToast(errorMessage(e, t("mcp.add_failed")), "error"),
      });
    }
  }

  function confirmTemplateInstall() {
    if (!installingTemplate) return;
    // Strip blank credential fields before submit. The server's installer
    // persists whatever is sent, so a skipped optional field would land
    // in the vault as an empty string and the downstream env lookup would
    // then skip its dotenv fallback (since the vault "has" a value,
    // albeit empty). Dropping the key lets the resolver chain work.
    const cleanCreds: Record<string, string> = {};
    for (const [k, v] of Object.entries(envInputs)) {
      if (typeof v === "string" && v.trim().length > 0) cleanCreds[k] = v;
    }
    addMutation.mutate(
      { template_id: installingTemplate.id, credentials: cleanCreds },
      {
        onSuccess: () => {
          setShowAddModal(false);
          setInstallingTemplate(null);
          setEnvInputs({});
          setForm(defaultForm);
          addToast(t("mcp.add_success"), "success");
        },
        onError: (e: unknown) => addToast(errorMessage(e, t("mcp.add_failed")), "error"),
      },
    );
  }

  const catalogEntries = catalogQuery.data?.entries ?? [];
  // Catalog entries are already flagged `installed` by the backend, but the
  // dashboard also treats a server whose `template_id` matches as installed.
  const installedTemplateIds = useMemo(
    () => new Set(configured.map(s => s.template_id).filter((x): x is string => Boolean(x))),
    [configured],
  );

  const filteredTemplates = useMemo(() => {
    if (!catalogSearch.trim()) return catalogEntries;
    const q = catalogSearch.toLowerCase();
    return catalogEntries.filter(tpl =>
      tpl.name.toLowerCase().includes(q) ||
      tpl.id.toLowerCase().includes(q) ||
      (tpl.description || "").toLowerCase().includes(q) ||
      (tpl.category || "").toLowerCase().includes(q) ||
      (tpl.tags ?? []).some(tag => tag.toLowerCase().includes(q))
    );
  }, [catalogEntries, catalogSearch]);

  const connectedCount = useMemo(
    () => configured.filter(s => connectedMap.get(serverIdentityOf(s))?.connected).length,
    [configured, connectedMap],
  );
  const disconnectedCount = configured.length - connectedCount;

  // Backend returns a list of per-server status entries — badge is "ok"
  // only when every entry reports a ready/healthy state. Null keeps the
  // badge hidden while loading or before any servers have been pinged.
  const healthEntries = healthQuery.data?.health;
  const healthOk =
    healthEntries === undefined
      ? null
      : healthEntries.length === 0
        ? null
        : healthEntries.every(h => h.status === "ready" || h.status === "ok");

  return (
    <div className="space-y-6">
      <PageHeader
        icon={<Plug className="h-5 w-5" />}
        badge={t("mcp.badge", { defaultValue: "MCP" })}
        title={t("mcp.title")}
        subtitle={tab === "catalog" ? t("mcp.catalog_subtitle") : t("mcp.subtitle")}
        isFetching={serversQuery.isFetching || catalogQuery.isFetching || healthQuery.isFetching}
        onRefresh={() => {
          serversQuery.refetch();
          healthQuery.refetch();
          if (tab === "catalog") catalogQuery.refetch();
        }}
        helpText={t("mcp.help")}
        actions={
          <>
            {healthOk !== null && (
              <Badge variant={healthOk ? "success" : "error"} dot>
                <Activity className="h-3 w-3 mr-1" />
                {healthOk ? t("mcp.health_ok") : t("mcp.health_degraded")}
              </Badge>
            )}
            <Button
              size="sm"
              variant="secondary"
              leftIcon={<RefreshCw className={`h-3.5 w-3.5 ${reloadMutation.isPending ? "animate-spin" : ""}`} />}
              onClick={handleReload}
              disabled={reloadMutation.isPending}
            >
              {t("mcp.reload")}
            </Button>
            <Button size="sm" leftIcon={<Plus className="h-3.5 w-3.5" />} onClick={openAdd}>
              {t("mcp.add_server")}
            </Button>
          </>
        }
      />

      {/* Tab switcher */}
      <div className="flex gap-1 rounded-xl border border-border-subtle bg-surface p-1">
        <button
          onClick={() => setTab("servers")}
          className={`flex items-center gap-1.5 px-4 py-2 rounded-lg text-xs font-bold transition-colors ${
            tab === "servers" ? "bg-brand/10 text-brand shadow-sm" : "text-text-dim hover:text-text"
          }`}
        >
          <Plug className="h-3.5 w-3.5" />
          {t("mcp.tab_my_servers")}
          {configured.length > 0 && (
            <span className={`ml-1 px-1.5 py-0.5 rounded-full text-[9px] font-bold ${tab === "servers" ? "bg-brand/20 text-brand" : "bg-border-subtle text-text-dim"}`}>
              {configured.length}
            </span>
          )}
        </button>
        <button
          onClick={() => setTab("catalog")}
          className={`flex items-center gap-1.5 px-4 py-2 rounded-lg text-xs font-bold transition-colors ${
            tab === "catalog" ? "bg-brand/10 text-brand shadow-sm" : "text-text-dim hover:text-text"
          }`}
        >
          <Store className="h-3.5 w-3.5" />
          {t("mcp.tab_catalog")}
        </button>
      </div>

      <AnimatePresence mode="wait">
      <motion.div key={tab} variants={tabContent} initial="initial" animate="animate" exit="exit" className="space-y-4">
      {tab === "servers" && (
        <>
          {/* Search + filter toolbar */}
          {configured.length > 0 && (
            <div className="flex flex-col sm:flex-row gap-3">
              {/* Search */}
              <div className="relative flex-1">
                <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-text-dim/50" />
                <input
                  type="text"
                  value={searchQuery}
                  onChange={(e) => setSearchQuery(e.target.value)}
                  placeholder={t("mcp.search_placeholder")}
                  className="w-full rounded-xl border border-border-subtle bg-surface pl-10 pr-4 py-2.5 text-sm font-medium text-text-main placeholder:text-text-dim/40 focus:border-brand focus:outline-none focus:ring-2 focus:ring-brand/10 hover:border-brand/20 transition-colors duration-200 shadow-sm"
                />
              </div>
              {/* Status filter */}
              <div className="flex gap-1 rounded-xl border border-border-subtle bg-surface p-1 shrink-0">
                {([
                  { value: "all" as const, label: t("mcp.filter_all"), count: configured.length },
                  { value: "connected" as const, label: t("mcp.filter_connected"), count: connectedCount },
                  { value: "disconnected" as const, label: t("mcp.filter_disconnected"), count: disconnectedCount },
                ] as const).map(({ value, label, count }) => (
                  <button
                    key={value}
                    onClick={() => setStatusFilter(value)}
                    className={`flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[10px] font-bold transition-colors ${
                      statusFilter === value
                        ? "bg-brand/10 text-brand shadow-sm"
                        : "text-text-dim hover:text-text"
                    }`}
                  >
                    <Filter className="h-3 w-3" />
                    {label}
                    <span className={`px-1 py-0.5 rounded-full text-[8px] font-bold ${
                      statusFilter === value ? "bg-brand/20 text-brand" : "bg-border-subtle text-text-dim"
                    }`}>
                      {count}
                    </span>
                  </button>
                ))}
              </div>
            </div>
          )}

          {/* Summary badges */}
          {data && (
            <div className="flex items-center gap-3 flex-wrap">
              <Badge variant="default">{t("mcp.total_configured", { count: data.total_configured })}</Badge>
              <Badge variant={data.total_connected > 0 ? "success" : "default"} dot>
                {t("mcp.total_connected", { count: data.total_connected })}
              </Badge>
            </div>
          )}

          {/* Loading */}
          {serversQuery.isLoading && <ListSkeleton rows={3} />}

          {/* Empty */}
          {!serversQuery.isLoading && configured.length === 0 && (
            <EmptyState
              icon={<Plug className="h-10 w-10" />}
              title={t("mcp.empty")}
              description={t("mcp.empty_desc")}
              action={
                <Button size="sm" leftIcon={<Store className="h-3.5 w-3.5" />} onClick={() => setTab("catalog")}>
                  {t("mcp.tab_catalog")}
                </Button>
              }
            />
          )}

          {/* No search results */}
          {!serversQuery.isLoading && configured.length > 0 && filteredServers.length === 0 && (
            <EmptyState
              icon={<Search className="h-10 w-10" />}
              title={t("mcp.no_results")}
              description={t("mcp.no_results_desc")}
            />
          )}

          {/* Server cards */}
          {filteredServers.length > 0 && (
            <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3 2xl:grid-cols-4">
              {filteredServers.map((server) => {
                const id = serverIdentityOf(server);
                return (
                  <ServerCard
                    key={id}
                    server={server}
                    conn={connectedMap.get(id)}
                    isExpanded={expandedTools.has(id)}
                    onToggleTools={() => toggleTools(server)}
                    onEdit={() => openEdit(server)}
                    onEditTaintPolicy={() => setTaintEditingServer(server)}
                    onDelete={() => deleteServer(server)}
                    t={t}
                  />
                );
              })}
            </div>
          )}
        </>
      )}

      {/* Catalog tab */}
      {tab === "catalog" && (
        <>
          {catalogQuery.isLoading && <ListSkeleton rows={3} />}

          {/* Catalog search — visible once data has loaded */}
          {!catalogQuery.isLoading && catalogEntries.length > 0 && (
            <div className="relative">
              <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-text-dim/50" />
              <input
                type="text"
                value={catalogSearch}
                onChange={(e) => setCatalogSearch(e.target.value)}
                placeholder={t("mcp.catalog_search_placeholder")}
                className="w-full rounded-xl border border-border-subtle bg-surface pl-10 pr-4 py-2.5 text-sm font-medium text-text-main placeholder:text-text-dim/40 focus:border-brand focus:outline-none focus:ring-2 focus:ring-brand/10 hover:border-brand/20 transition-colors duration-200 shadow-sm"
              />
            </div>
          )}
          {!catalogQuery.isLoading && catalogEntries.length === 0 && (
            <EmptyState
              icon={<Store className="h-10 w-10" />}
              title={t("mcp.catalog_empty")}
              description={t("mcp.catalog_empty_desc")}
            />
          )}
          {!catalogQuery.isLoading && catalogEntries.length > 0 && filteredTemplates.length === 0 && (
            <EmptyState
              icon={<Search className="h-10 w-10" />}
              title={t("mcp.no_results")}
              description={t("mcp.no_results_desc")}
            />
          )}
          {filteredTemplates.length > 0 && (
            <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3 2xl:grid-cols-4">
              {filteredTemplates.map((tpl) => {
                const alreadyAdded = tpl.installed || installedTemplateIds.has(tpl.id);
                return (
                  <Card key={tpl.id} hover={!alreadyAdded} padding="none" className={`flex flex-col overflow-hidden group ${alreadyAdded ? "opacity-75" : ""}`}>
                    <div className={`h-1.5 bg-linear-to-r ${
                      alreadyAdded
                        ? "from-success via-success/60 to-success/30"
                        : "from-brand via-brand/60 to-brand/30"
                    }`} />
                    <div className="p-5 flex-1 flex flex-col">
                      {/* Header */}
                      <div className="flex items-start justify-between gap-3 mb-3">
                        <div className="flex items-center gap-3 min-w-0">
                          <div className={`w-10 h-10 rounded-lg flex items-center justify-center shadow-sm ${
                            alreadyAdded
                              ? "bg-linear-to-br from-success/10 to-success/5 border border-success/20"
                              : "bg-linear-to-br from-brand/10 to-brand/5 border border-brand/20"
                          }`}>
                            {tpl.icon
                              ? <CatalogIcon icon={tpl.icon} className={`w-5 h-5 ${alreadyAdded ? "text-success" : "text-brand"}`} />
                              : <Plug className={`w-5 h-5 ${alreadyAdded ? "text-success" : "text-brand"}`} />
                            }
                          </div>
                          <div className="min-w-0 flex-1">
                            <h3 className={`text-sm font-black truncate transition-colors ${
                              alreadyAdded ? "" : "group-hover:text-brand"
                            }`}>{tpl.name}</h3>
                            {tpl.category && (
                              <span className="block truncate text-[10px] font-black uppercase tracking-widest text-text-dim/60">{tpl.category}</span>
                            )}
                          </div>
                        </div>
                        {alreadyAdded && (
                          <Badge variant="success" dot>
                            <Check className="h-3 w-3 mr-0.5" />
                            {t("mcp.catalog_installed")}
                          </Badge>
                        )}
                      </div>

                      {/* Description */}
                      <p className="text-xs text-text-dim leading-relaxed line-clamp-2 mb-3 flex-1">{tpl.description}</p>

                      {/* Tags */}
                      {(tpl.tags ?? []).length > 0 && (
                        <div className="flex flex-wrap gap-1 mb-3">
                          {tpl.tags!.slice(0, 4).map(tag => (
                            <span key={tag} className="px-1.5 py-0.5 rounded-full text-[9px] font-bold bg-brand/8 text-brand/70">{tag}</span>
                          ))}
                        </div>
                      )}

                      {/* Required env vars */}
                      {(tpl.required_env ?? []).length > 0 && (
                        <div className="space-y-1 mb-2">
                          {(tpl.required_env ?? []).map(e => (
                            <div key={e.name} className="flex items-center gap-1.5 text-[10px]">
                              <Key className="w-3 h-3 text-text-dim/50 shrink-0" />
                              <span className="font-mono font-bold text-text-dim">{e.name}</span>
                              {e.get_url && (
                                <a href={e.get_url} target="_blank" rel="noopener noreferrer" className="text-brand hover:underline ml-auto">
                                  <ExternalLink className="h-3 w-3" />
                                </a>
                              )}
                            </div>
                          ))}
                        </div>
                      )}
                    </div>

                    {/* Action */}
                    <div className="border-t border-border-subtle">
                      <button
                        onClick={() => installFromTemplate(tpl)}
                        disabled={alreadyAdded}
                        className={`w-full flex items-center justify-center gap-1.5 py-3 text-xs font-bold transition-colors rounded-b-xl sm:rounded-b-2xl ${
                          alreadyAdded
                            ? "text-text-dim/30 cursor-not-allowed"
                            : "text-brand hover:bg-brand/5"
                        }`}
                      >
                        {alreadyAdded
                          ? <><Check className="h-3.5 w-3.5" /> {t("mcp.catalog_installed")}</>
                          : <><Download className="h-3.5 w-3.5" /> {t("mcp.catalog_install")}</>
                        }
                      </button>
                    </div>
                  </Card>
                );
              })}
            </div>
          )}
        </>
      )}
      </motion.div>
      </AnimatePresence>

      {/* Add / Edit Modal */}
      <DrawerPanel
        isOpen={isModalOpen}
        onClose={() => { setShowAddModal(false); setEditingServer(null); setForm(defaultForm); }}
        title={editingServer ? t("mcp.edit_server") : t("mcp.add_server")}
        size="lg"
      >
        <div className="p-5 space-y-4">
          {/* Name */}
          <Input
            label={t("mcp.name")}
            value={form.name}
            onChange={(e) => updateField("name", e.target.value)}
            placeholder={t("mcp.name_placeholder")}
            disabled={!!editingServer}
          />

          {/* Transport type */}
          <div className="flex flex-col gap-1.5">
            <label className="text-[10px] font-black uppercase tracking-widest text-text-dim">
              {t("mcp.transport_type")}
            </label>
            <div className="flex gap-2">
              {(["stdio", "sse", "http"] as TransportType[]).map((tt) => (
                <button
                  key={tt}
                  onClick={() => updateField("transportType", tt)}
                  className={`flex items-center gap-1.5 rounded-xl border px-3 py-2 text-xs font-bold transition-colors ${
                    form.transportType === tt
                      ? "border-brand bg-brand/10 text-brand"
                      : "border-border-subtle bg-surface text-text-dim hover:border-brand/20"
                  }`}
                >
                  <TransportIcon type={tt} />
                  {tt.toUpperCase()}
                </button>
              ))}
            </div>
          </div>

          {/* stdio fields — grouped */}
          {form.transportType === "stdio" && (
            <div className="rounded-xl border border-border-subtle p-4 space-y-4 bg-main/30">
              <div className="flex items-center gap-1.5 text-[10px] font-black uppercase tracking-widest text-text-dim">
                <Terminal className="h-3 w-3" />
                {t("mcp.stdio_config")}
              </div>
              <Input
                label={t("mcp.command")}
                value={form.command}
                onChange={(e) => updateField("command", e.target.value)}
                placeholder={t("mcp.command_placeholder")}
              />
              <div className="flex flex-col gap-1.5">
                <label className="text-[10px] font-black uppercase tracking-widest text-text-dim">
                  {t("mcp.args")}
                </label>
                <ArgsEditor items={form.args} onChange={(v) => updateField("args", v)} />
              </div>
            </div>
          )}

          {/* sse/http fields — grouped */}
          {(form.transportType === "sse" || form.transportType === "http") && (
            <div className="rounded-xl border border-border-subtle p-4 space-y-4 bg-main/30">
              <div className="flex items-center gap-1.5 text-[10px] font-black uppercase tracking-widest text-text-dim">
                {form.transportType === "sse" ? <Radio className="h-3 w-3" /> : <Globe className="h-3 w-3" />}
                {form.transportType.toUpperCase()} {t("mcp.connection")}
              </div>
              <Input
                label={t("mcp.url")}
                value={form.url}
                onChange={(e) => updateField("url", e.target.value)}
                placeholder={t("mcp.url_placeholder")}
              />
              {form.url && !form.url.startsWith("http://") && !form.url.startsWith("https://") && (
                <p className="text-[10px] text-warning font-bold">{t("mcp.url_hint")}</p>
              )}
              <div className="flex flex-col gap-1.5">
                <label className="text-[10px] font-black uppercase tracking-widest text-text-dim">
                  {t("mcp.headers")}
                </label>
                <textarea
                  value={form.headers}
                  onChange={(e) => updateField("headers", e.target.value)}
                  placeholder={t("mcp.headers_placeholder")}
                  rows={2}
                  className="w-full rounded-xl border border-border-subtle bg-surface px-4 py-2.5 text-sm font-mono text-text-main placeholder:text-text-dim/40 focus:border-brand focus:outline-none focus:ring-2 focus:ring-brand/10 hover:border-brand/20 transition-colors duration-200 shadow-sm resize-none"
                />
              </div>
            </div>
          )}

          {/* Timeout */}
          <Input
            label={t("mcp.timeout")}
            type="number"
            value={String(form.timeout)}
            onChange={(e) => updateField("timeout", parseInt(e.target.value) || 30)}
            min={1}
            max={600}
          />

          {/* Env vars */}
          <div className="flex flex-col gap-1.5">
            <label className="text-[10px] font-black uppercase tracking-widest text-text-dim">
              {t("mcp.env")}
            </label>
            <EnvEditor items={form.env} onChange={(v) => updateField("env", v)} />
          </div>

          {/* Actions */}
          <div className="flex gap-3 pt-2">
            <Button
              variant="secondary"
              className="flex-1"
              onClick={() => { setShowAddModal(false); setEditingServer(null); setForm(defaultForm); }}
            >
              {t("common.cancel")}
            </Button>
            <Button
              className="flex-1"
              isLoading={isSubmitting}
              disabled={!form.name.trim() || (form.transportType === "stdio" ? !form.command.trim() : !form.url.trim())}
              onClick={handleSubmit}
            >
              {t("common.save")}
            </Button>
          </div>
        </div>
      </DrawerPanel>

      {/* Catalog env setup modal */}
      <DrawerPanel
        isOpen={!!installingTemplate}
        onClose={() => { setInstallingTemplate(null); setEnvInputs({}); }}
        title={t("mcp.env_setup_title", { name: installingTemplate?.name ?? "" })}
        size="md"
      >
        <div className="p-5 space-y-4">
          <p className="text-xs text-text-dim">{t("mcp.env_setup_desc")}</p>
          {(installingTemplate?.required_env ?? []).map(e => (
            <div key={e.name} className="flex flex-col gap-1.5">
              <div className="flex items-center gap-1.5">
                <label className="text-[10px] font-black uppercase tracking-widest text-text-dim">
                  {e.label || e.name}
                </label>
                {e.get_url && (
                  <a href={e.get_url} target="_blank" rel="noopener noreferrer" className="text-brand hover:underline">
                    <ExternalLink className="h-3 w-3" />
                  </a>
                )}
              </div>
              {e.help && <span className="text-[9px] text-text-dim/50">{e.help}</span>}
              <input
                type={e.is_secret ? "password" : "text"}
                value={envInputs[e.name] ?? ""}
                onChange={(ev) => setEnvInputs(prev => ({ ...prev, [e.name]: ev.target.value }))}
                placeholder={e.label || e.name}
                className="w-full rounded-xl border border-border-subtle bg-surface px-4 py-2.5 text-sm font-mono text-text-main placeholder:text-text-dim/40 focus:border-brand focus:outline-none focus:ring-2 focus:ring-brand/10 hover:border-brand/20 transition-colors duration-200 shadow-sm"
              />
            </div>
          ))}
          <div className="flex gap-3 pt-2">
            <Button
              variant="secondary"
              className="flex-1"
              onClick={() => { setInstallingTemplate(null); setEnvInputs({}); }}
            >
              {t("common.cancel")}
            </Button>
            <Button
              className="flex-1"
              isLoading={addMutation.isPending}
              leftIcon={<Download className="h-3.5 w-3.5" />}
              onClick={confirmTemplateInstall}
            >
              {t("mcp.catalog_install")}
            </Button>
          </div>
        </div>
      </DrawerPanel>

      {/* Delete confirmation */}
      <ConfirmDialog
        isOpen={!!deletingServer}
        title={t("mcp.delete_server")}
        message={t("mcp.delete_confirm")}
        tone="destructive"
        confirmLabel={t("common.delete")}
        onConfirm={() => {
          if (deletingServer) deleteMutation.mutate(serverIdOf(deletingServer), {
            onSuccess: () => {
              setDeletingServer(null);
              addToast(t("mcp.delete_success"), "success");
            },
            onError: (e: unknown) => addToast(errorMessage(e, t("mcp.delete_failed")), "error"),
          });
        }}
        onClose={() => setDeletingServer(null)}
      />

      {/* Issue #3050: granular taint-policy editor */}
      {taintEditingServer && (
        <TaintPolicyEditor
          server={taintEditingServer}
          isOpen={!!taintEditingServer}
          onClose={() => setTaintEditingServer(null)}
        />
      )}
    </div>
  );
}
