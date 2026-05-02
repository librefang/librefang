import React, { Component, lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
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
  useReconnectMcpServer,
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
  Plug, Plus, X, Trash2, Settings, Wrench, Terminal, Globe, Radio,
  Shield, ShieldCheck, ShieldAlert, ShieldX, Check, ExternalLink,
  Search, Filter, Store, Key, Download, RefreshCw, Activity,
  ShieldHalf, Server, FileText, RotateCw,
} from "lucide-react";
import { TaintPolicyEditor } from "../components/TaintPolicyEditor";
// Lazy-load individual lucide icons by name — avoids pulling the full
// ~1500-icon registry that `lucide-react/dynamic` bundles.
type IconName = string;
const lazyIconCache = new Map<string, React.LazyExoticComponent<React.ComponentType<{ className?: string }>>>();
function getLazyIcon(name: string) {
  if (!lazyIconCache.has(name)) {
    lazyIconCache.set(
      name,
      lazy(() =>
        import("lucide-react").then((mod) => {
          // Convert kebab-case icon name to PascalCase component name
          const pascal = name
            .split("-")
            .map((s) => s.charAt(0).toUpperCase() + s.slice(1))
            .join("");
          const Component = (mod as Record<string, unknown>)[pascal] as React.ComponentType<{ className?: string }> | undefined;
          if (!Component) throw new Error(`lucide icon not found: ${pascal}`);
          return { default: Component };
        }),
      ),
    );
  }
  return lazyIconCache.get(name)!;
}

// Error boundary wrapping each lazily-loaded catalog icon. If the icon
// name from the backend doesn't map to a real lucide export the lazy
// import rejects — the boundary catches the render error and shows the
// neutral fallback instead of blowing up the surrounding card.
class LazyIconBoundary extends Component<
  { children: ReactNode; fallback: ReactNode },
  { hasError: boolean }
> {
  state = { hasError: false };
  static getDerivedStateFromError() {
    return { hasError: true };
  }
  componentDidCatch(error: Error) {
    // eslint-disable-next-line no-console
    console.warn("CatalogIcon: lazy icon failed to load, using fallback.", error);
  }
  render() {
    return this.state.hasError ? this.props.fallback : this.props.children;
  }
}

function CatalogIcon({ icon, className }: { icon: string; className?: string }) {
  if (icon.startsWith("lucide:")) {
    const name = icon.slice("lucide:".length) as IconName;
    const fallback = <Plug className={className} />;
    const LazyIcon = getLazyIcon(name);
    return (
      <LazyIconBoundary fallback={fallback}>
        <Suspense fallback={fallback}>
          <LazyIcon className={className} />
        </Suspense>
      </LazyIconBoundary>
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

// Kernel-side names tools as `mcp_<normalized-server>_<normalized-tool>` and
// prepends `[MCP:<server>] ` to descriptions so the LLM can disambiguate
// across servers. Both prefixes are noise once we already show the
// server's name as the page header \u2014 strip them for display, keep
// the full names in `title=` for copy/inspect.
//
// `normalize_name` in crates/librefang-runtime-mcp/src/lib.rs does
// `to_lowercase().replace('-', "_")`, so a server named `test-filesystem`
// gets tool prefix `mcp_test_filesystem_`. Mirror that here or strip
// silently fails for hyphenated names.
function normalizeMcpName(name: string): string {
  return name.toLowerCase().replace(/-/g, "_");
}

function stripMcpToolPrefix(toolName: string, serverName: string): string {
  const prefix = `mcp_${normalizeMcpName(serverName)}_`;
  return toolName.startsWith(prefix) ? toolName.slice(prefix.length) : toolName;
}

function stripMcpDescPrefix(description: string, serverName: string): string {
  // The description prefix uses the raw server name, not the normalized one
  // (kernel writes it as `[MCP:{server.name}]`). Match that exactly.
  const prefix = `[MCP:${serverName}] `;
  return description.startsWith(prefix) ? description.slice(prefix.length) : description;
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
            className="flex-1 rounded-lg border border-border-subtle bg-surface px-3 py-1.5 text-sm font-mono text-text-main placeholder:text-text-dim/40 focus:border-primary focus:outline-none focus:ring-2 focus:ring-primary/10 hover:border-primary/20 transition-colors duration-200 shadow-sm"
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
        className="flex items-center gap-1 text-[10px] font-bold text-text-dim hover:text-primary transition-colors py-0.5"
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
            className="flex-1 rounded-lg border border-border-subtle bg-surface px-3 py-1.5 text-sm font-mono text-text-main placeholder:text-text-dim/40 focus:border-primary focus:outline-none focus:ring-2 focus:ring-primary/10 hover:border-primary/20 transition-colors duration-200 shadow-sm"
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
        className="flex items-center gap-1 text-[10px] font-bold text-text-dim hover:text-primary transition-colors py-0.5"
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
    // We deliberately do NOT pass `noopener` here.  Per the HTML spec
    // `window.open(url, target, "noopener")` returns null, so the
    // dashboard loses its handle to the popup, the `if (authWindow)`
    // branch is dead, and the fallback navigates the dashboard tab
    // ITSELF to the OAuth provider — destroying the in-dashboard UX
    // (#3945 follow-up).  `noreferrer` implies `noopener` so it has
    // the same problem; drop both.
    //
    // Tabnabbing risk is mitigated below by setting
    // `authWindow.opener = null` immediately after navigation, which
    // achieves the same isolation `noopener` was meant to provide
    // without nuking the window handle we need.  Referer leak is not
    // a credential issue here — the OAuth provider already learns
    // the dashboard origin from `redirect_uri`.
    const authWindow = window.open("about:blank", "_blank");
    try {
      const result = await startAuthMutation.mutateAsync(serverIdentity);
      if (authWindow && !authWindow.closed) {
        // Nullify opener BEFORE navigation: the popup is still on
        // about:blank (same origin) so the assignment can't throw, and
        // once the navigation kicks the window into the IdP origin,
        // .opener is already null. No race window.
        authWindow.opener = null;
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

// ── Server Card (compact design) ────────────────────────────────────

function ServerCard({
  server,
  conn,
  onAuthSuccess,
  onViewDetail,
  t,
}: {
  server: McpServerConfigured;
  conn?: McpServerConnected;
  onAuthSuccess?: () => void;
  onViewDetail: () => void;
  t: TFunction;
}) {
  const isConnected = conn?.connected ?? false;
  const toolsCount = conn?.tools_count ?? 0;
  const transportType = useMemo(() => getTransportType(server), [server]);
  const transportDetail = useMemo(() => getTransportDetail(server), [server]);
  const authState = server.auth_state?.state ?? "not_required";
  const authLabel =
    authState === "authorized"
      ? "oauth"
      : authState === "not_required"
        ? (server.env ?? []).length > 0
          ? "token"
          : "none"
        : authState;

  return (
    <Card
      hover
      padding="none"
      className="overflow-hidden cursor-pointer group"
      onClick={onViewDetail}
      onKeyDown={(e: React.KeyboardEvent) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onViewDetail();
        }
      }}
      role="button"
      tabIndex={0}
      aria-label={t("mcp.view_detail", { defaultValue: "View server details" })}
    >
      <div className="p-3.5 flex items-start gap-2.5">
        <div className="grid place-items-center w-[30px] h-[30px] rounded-lg bg-brand/10 border border-brand/30 text-brand shrink-0">
          <Server className="w-3.5 h-3.5" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="font-mono text-[13px] font-medium truncate">{server.name}</span>
            <Badge variant={isConnected ? "success" : "error"} dot>
              {isConnected ? t("mcp.connected") : t("mcp.disconnected")}
            </Badge>
          </div>
          <div className="font-mono text-[11px] text-text-dim mt-0.5 truncate">
            {transportType === "stdio"
              ? `mcp+stdio://${transportDetail || server.name}`
              : transportDetail}
          </div>
          <div className="flex items-center gap-2.5 mt-2 text-[11px]">
            <span className="font-mono text-text-dim">
              {t("mcp.tools_count_short", { count: toolsCount, defaultValue: "{{count}} tools" })}
            </span>
            <span className="font-mono text-accent">{authLabel}</span>
          </div>
        </div>
      </div>
      {/* OAuth state — only renders when not "not_required". Stops the parent
          click so the auth button doesn't accidentally open the detail. */}
      {authState !== "not_required" && (
        <div
          className="px-3.5 pb-3 -mt-1"
          onClick={(e) => e.stopPropagation()}
          onKeyDown={(e) => e.stopPropagation()}
        >
          <AuthBadge server={server} onAuthSuccess={onAuthSuccess} />
        </div>
      )}
    </Card>
  );
}

// ── Server Detail Drawer Body (Tools / Logs / Config tabs) ─────────

function Mini({ label, value, tone }: { label: string; value: string; tone?: "ok" | "bad" }) {
  return (
    <div className="rounded-lg border border-border-subtle bg-main/40 p-2.5">
      <div className="text-[9px] font-bold uppercase tracking-wider text-text-dim mb-1">{label}</div>
      <div
        className={`font-mono text-[14px] font-semibold ${
          tone === "ok" ? "text-success" : tone === "bad" ? "text-error" : ""
        }`}
      >
        {value}
      </div>
    </div>
  );
}

type DetailTab = "tools" | "logs" | "config";

function ServerDetailBody({
  server,
  conn,
  onClose,
  onEdit,
  onEditTaintPolicy,
  onDelete,
}: {
  server: McpServerConfigured;
  conn?: McpServerConnected;
  onClose: () => void;
  onEdit: () => void;
  onEditTaintPolicy: () => void;
  onDelete: () => void;
}) {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);
  const reconnectMutation = useReconnectMcpServer();
  const [tab, setTab] = useState<DetailTab>("tools");

  const isConnected = conn?.connected ?? false;
  const transportType = getTransportType(server);
  const transportDetail = getTransportDetail(server);
  const transportLabel =
    transportType === "stdio"
      ? `stdio · ${transportDetail || "—"}`
      : `${transportType} · ${transportDetail || "—"}`;
  const authStateStr = server.auth_state?.state ?? "not_required";
  const tools = conn?.tools ?? [];

  const handleReconnect = () => {
    reconnectMutation.mutate(serverIdOf(server), {
      onSuccess: () => addToast(t("mcp.reconnect_success", { defaultValue: "Reconnect requested" }), "success"),
      onError: (e: unknown) =>
        addToast(errorMessage(e, t("mcp.reconnect_failed", { defaultValue: "Reconnect failed" })), "error"),
    });
  };

  return (
    <div className="flex flex-col h-full">
      {/* Hero */}
      <div className="px-5 py-4 border-b border-border-subtle">
        <div className="flex items-start gap-3">
          <div className="grid place-items-center w-10 h-10 rounded-lg bg-brand/10 border border-brand/30 text-brand shrink-0">
            <Server className="w-5 h-5" />
          </div>
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2 flex-wrap">
              <h2 className="text-base font-semibold tracking-tight truncate">{server.name}</h2>
              <Badge variant={isConnected ? "success" : "error"} dot>
                {isConnected ? t("mcp.connected") : t("mcp.disconnected")}
              </Badge>
            </div>
            <p className="font-mono text-[11px] text-text-dim mt-1 break-all">{transportLabel}</p>
          </div>
          <div className="flex items-center gap-1.5 shrink-0">
            <Button
              size="sm"
              variant="ghost"
              leftIcon={<RotateCw className={`h-3.5 w-3.5 ${reconnectMutation.isPending ? "animate-spin" : ""}`} />}
              onClick={handleReconnect}
              disabled={reconnectMutation.isPending}
            >
              {t("mcp.reconnect", { defaultValue: "Reconnect" })}
            </Button>
          </div>
        </div>
      </div>

      {/* Tabs */}
      <div className="flex border-b border-border-subtle px-5 bg-main/20">
        {([
          { id: "tools" as const, label: t("mcp.tab_tools", { defaultValue: "Tools" }), icon: Wrench, count: conn?.tools_count ?? 0 },
          { id: "logs" as const, label: t("mcp.tab_logs", { defaultValue: "Logs" }), icon: FileText },
          { id: "config" as const, label: t("mcp.tab_config", { defaultValue: "Config" }), icon: Settings },
        ]).map((td) => {
          const active = tab === td.id;
          const Icon = td.icon;
          return (
            <button
              key={td.id}
              role="tab"
              aria-selected={active}
              onClick={() => setTab(td.id)}
              className={`inline-flex items-center gap-2 px-3 py-2.5 text-[12.5px] border-b-2 transition-colors ${
                active
                  ? "border-brand font-semibold"
                  : "border-transparent text-text-dim font-medium hover:text-current"
              }`}
            >
              <Icon className="w-3.5 h-3.5" />
              {td.label}
              {td.count !== undefined && (
                <span
                  className={`font-mono text-[10px] px-1.5 py-px rounded-full ${
                    active ? "bg-brand/15 text-brand" : "bg-text-dim/10 text-text-dim"
                  }`}
                >
                  {td.count}
                </span>
              )}
            </button>
          );
        })}
      </div>

      {/* Tab body */}
      <div className="flex-1 overflow-y-auto p-5">
        {tab === "tools" && (
          <>
            <div className="grid grid-cols-2 sm:grid-cols-4 gap-2.5 mb-4">
              <Mini label="tools_count" value={String(conn?.tools_count ?? 0)} />
              <Mini label="connected" value={isConnected ? "true" : "false"} tone={isConnected ? "ok" : "bad"} />
              <Mini label="auth_state" value={authStateStr} />
              <Mini label="timeout_secs" value={String(server.timeout_secs ?? 30)} />
            </div>

            {tools.length === 0 ? (
              <EmptyState
                icon={<Wrench className="h-7 w-7" />}
                title={t("mcp.tools_empty", { defaultValue: "No tools advertised" })}
                description={t("mcp.tools_empty_desc", {
                  defaultValue: "The server hasn't reported any tools yet — confirm it's connected.",
                })}
              />
            ) : (
              <Card padding="none" className="overflow-hidden">
                <div className="hidden sm:grid grid-cols-[minmax(160px,1.4fr)_2fr_60px_50px_60px] items-center gap-2 px-3 py-2 border-b border-border-subtle bg-main/40 text-[10px] font-bold uppercase tracking-wider text-text-dim">
                  <span>{t("mcp.col_name", { defaultValue: "name" })}</span>
                  <span>{t("mcp.col_description", { defaultValue: "description" })}</span>
                  <span className="text-right">{t("mcp.col_calls", { defaultValue: "calls" })}</span>
                  <span className="text-right">{t("mcp.col_ok", { defaultValue: "ok %" })}</span>
                  <span className="text-right">{t("mcp.col_last", { defaultValue: "last" })}</span>
                </div>
                {tools.map((tool, i) => {
                  const displayName = stripMcpToolPrefix(tool.name, server.name);
                  const displayDesc = stripMcpDescPrefix(tool.description ?? "", server.name);
                  return (
                    <div
                      key={tool.name}
                      className={`grid grid-cols-[1fr_60px] sm:grid-cols-[minmax(160px,1.4fr)_2fr_60px_50px_60px] items-center gap-2 px-3 py-2 text-[12px] ${
                        i < tools.length - 1 ? "border-b border-border-subtle" : ""
                      }`}
                    >
                      <span
                        className="font-mono font-medium truncate min-w-0"
                        title={tool.name}
                      >
                        {displayName}
                      </span>
                      <span
                        className="hidden sm:block text-text-dim text-[11.5px] truncate min-w-0"
                        title={tool.description ?? ""}
                      >
                        {displayDesc}
                      </span>
                      {/* TODO: aggregate from /api/sessions to populate calls/ok%/last per tool */}
                      <span className="font-mono text-text-dim/60 text-right text-[11px]">—</span>
                      <span className="hidden sm:inline font-mono text-text-dim/60 text-right text-[11px]">—</span>
                      <span className="font-mono text-text-dim/60 text-right text-[11px]">—</span>
                    </div>
                  );
                })}
              </Card>
            )}
          </>
        )}

        {tab === "logs" && (
          <EmptyState
            icon={<FileText className="h-7 w-7" />}
            title={t("mcp.logs_empty", { defaultValue: "Logs not available" })}
            description={t("mcp.logs_empty_desc", {
              defaultValue: "Per-server MCP logs aren't exposed yet — check the daemon's stdout for connection traces.",
            })}
          />
        )}

        {tab === "config" && (
          <div className="flex flex-col gap-4">
            {/* OAuth banner if non-ok */}
            {server.auth_state && server.auth_state.state !== "ok" && server.auth_state.state !== "not_required" && (
              <div className="rounded-lg border border-warning/30 bg-warning/5 p-3 text-[12px] text-warning">
                <p className="font-bold mb-1">{server.auth_state.state}</p>
                {server.auth_state.message && (
                  <p className="text-text-dim">{server.auth_state.message}</p>
                )}
              </div>
            )}

            {/* Action row */}
            <div className="flex flex-wrap gap-2">
              <Button
                size="sm"
                variant="secondary"
                leftIcon={<Settings className="w-3.5 h-3.5" />}
                onClick={() => {
                  onClose();
                  onEdit();
                }}
              >
                {t("common.edit")}
              </Button>
              <Button
                size="sm"
                variant="secondary"
                leftIcon={<ShieldHalf className="w-3.5 h-3.5" />}
                onClick={() => {
                  onClose();
                  onEditTaintPolicy();
                }}
              >
                {t("mcp.taint_policy_short", { defaultValue: "Taint" })}
              </Button>
              <Button
                size="sm"
                variant="danger"
                leftIcon={<Trash2 className="w-3.5 h-3.5" />}
                onClick={() => {
                  onClose();
                  onDelete();
                }}
              >
                {t("common.delete")}
              </Button>
            </div>

            {/* JSON spec */}
            <Card padding="none" className="overflow-hidden">
              <div className="px-3 py-2 border-b border-border-subtle bg-main/40 flex items-center justify-between">
                <span className="text-[10px] font-bold uppercase tracking-wider text-text-dim">
                  McpServerConfigured
                </span>
                <span className="font-mono text-[10px] text-brand">
                  PUT /api/mcp/servers/{serverIdOf(server)}
                </span>
              </div>
              <pre className="font-mono text-[11px] leading-relaxed p-3 overflow-x-auto whitespace-pre">
                {JSON.stringify(server, null, 2)}
              </pre>
            </Card>

            <div className="rounded-lg border border-brand/25 bg-brand/5 p-3 text-[11.5px] text-text-dim font-mono leading-relaxed">
              {t("mcp.config_env_notice", {
                defaultValue:
                  "env vars are referenced by name only; secrets resolved from the host environment at connect-time.",
              })}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

// ── Catalog Card (compact) ──────────────────────────────────────────

function CatalogCard({
  tpl,
  alreadyAdded,
  onViewDetail,
  onInstall,
  t,
}: {
  tpl: McpCatalogEntry;
  alreadyAdded: boolean;
  onViewDetail: () => void;
  onInstall: () => void;
  t: TFunction;
}) {
  const reqEnvCount = (tpl.required_env ?? []).length;
  return (
    <Card
      hover={!alreadyAdded}
      padding="none"
      className={`overflow-hidden ${alreadyAdded ? "opacity-70" : ""}`}
    >
      <div
        className="p-3.5 cursor-pointer"
        role="button"
        tabIndex={0}
        onClick={onViewDetail}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            onViewDetail();
          }
        }}
      >
        <div className="flex items-start gap-2.5">
          <div
            className={`grid place-items-center w-[30px] h-[30px] rounded-lg shrink-0 ${
              alreadyAdded
                ? "bg-success/10 border border-success/30 text-success"
                : "bg-accent/10 border border-accent/30 text-accent"
            }`}
          >
            {tpl.icon ? (
              <CatalogIcon icon={tpl.icon} className="w-3.5 h-3.5" />
            ) : (
              <Plug className="w-3.5 h-3.5" />
            )}
          </div>
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <span className="font-mono text-[13px] font-medium truncate">{tpl.name}</span>
              {alreadyAdded && (
                <Badge variant="success" dot>
                  {t("mcp.catalog_installed")}
                </Badge>
              )}
            </div>
            {tpl.category && (
              <div className="font-mono text-[10px] uppercase tracking-widest text-text-dim mt-0.5 truncate">
                {tpl.category}
              </div>
            )}
            <p className="text-[11.5px] text-text-dim leading-snug line-clamp-2 mt-1.5">
              {tpl.description}
            </p>
            <div className="flex items-center gap-2.5 mt-2 text-[11px]">
              {reqEnvCount > 0 && (
                <span className="font-mono text-accent inline-flex items-center gap-1">
                  <Key className="w-2.5 h-2.5" />
                  {t("mcp.requires_env_count", {
                    count: reqEnvCount,
                    defaultValue: "{{count}} key",
                  })}
                </span>
              )}
              {(tpl.tags ?? []).slice(0, 2).map((tag) => (
                <span
                  key={tag}
                  className="font-mono text-text-dim text-[10.5px]"
                >
                  #{tag}
                </span>
              ))}
            </div>
          </div>
        </div>
      </div>
      <button
        onClick={onInstall}
        disabled={alreadyAdded}
        className={`w-full flex items-center justify-center gap-1.5 py-2.5 border-t border-border-subtle text-[12px] font-semibold transition-colors ${
          alreadyAdded
            ? "text-text-dim/40 cursor-not-allowed"
            : "text-brand hover:bg-brand/5"
        }`}
      >
        {alreadyAdded ? (
          <>
            <Check className="h-3.5 w-3.5" />
            {t("mcp.catalog_installed")}
          </>
        ) : (
          <>
            <Download className="h-3.5 w-3.5" />
            {t("mcp.catalog_install")}
          </>
        )}
      </button>
    </Card>
  );
}

// ── Catalog Install Wizard ──────────────────────────────────────────
//
// Replaces the old single-step "env setup" drawer with a 4-step flow
// modeled on the design bundle's MCPInstall mock (Permissions →
// Configure → Installing → Done). The Installing step is gated on the
// real addMcpServer mutation rather than mock theatre — when the
// promise resolves we advance to Done; on error we drop back to
// Configure with the mutation's error toast.

type WizardStep = "permissions" | "configure" | "installing" | "done";

function CatalogInstallWizard({
  template,
  onClose,
  onSuccess,
  t,
}: {
  template: McpCatalogEntry;
  onClose: () => void;
  onSuccess: () => void;
  t: TFunction;
}) {
  const addToast = useUIStore((s) => s.addToast);
  const addMutation = useAddMcpServer();
  const [step, setStep] = useState<WizardStep>("permissions");
  const [envInputs, setEnvInputs] = useState<Record<string, string>>(() => {
    const seed: Record<string, string> = {};
    for (const e of template.required_env ?? []) seed[e.name] = "";
    return seed;
  });
  const [accepted, setAccepted] = useState(false);
  const requiredEnv = template.required_env ?? [];
  const hasRequiredEnv = requiredEnv.length > 0;

  function runInstall() {
    setStep("installing");
    const cleanCreds: Record<string, string> = {};
    for (const [k, v] of Object.entries(envInputs)) {
      if (typeof v === "string" && v.trim().length > 0) cleanCreds[k] = v;
    }
    addMutation.mutate(
      hasRequiredEnv
        ? { template_id: template.id, credentials: cleanCreds }
        : { template_id: template.id },
      {
        onSuccess: () => {
          setStep("done");
          addToast(t("mcp.add_success"), "success");
        },
        onError: (e: unknown) => {
          addToast(errorMessage(e, t("mcp.add_failed")), "error");
          // Step back to whichever pane the user was last editing.
          setStep(hasRequiredEnv ? "configure" : "permissions");
        },
      },
    );
  }

  // Stepper at the top.
  const steps: { id: WizardStep; label: string }[] = [
    { id: "permissions", label: t("mcp.wizard.step_permissions", { defaultValue: "Overview" }) },
    { id: "configure", label: t("mcp.wizard.step_configure", { defaultValue: "Configure" }) },
    { id: "installing", label: t("mcp.wizard.step_installing", { defaultValue: "Installing" }) },
    { id: "done", label: t("mcp.wizard.step_done", { defaultValue: "Done" }) },
  ];
  const currentIdx = steps.findIndex((s) => s.id === step);

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="flex items-center gap-3 p-4 border-b border-border-subtle">
        <div className="grid place-items-center w-10 h-10 rounded-lg bg-accent/10 border border-accent/30 text-accent shrink-0">
          {template.icon ? (
            <CatalogIcon icon={template.icon} className="w-5 h-5" />
          ) : (
            <Plug className="w-5 h-5" />
          )}
        </div>
        <div className="min-w-0 flex-1">
          <div className="font-mono text-[10px] uppercase tracking-widest text-text-dim">
            {t("mcp.wizard.eyebrow", { defaultValue: "install · mcp server" })}
          </div>
          <h2 className="text-[16px] font-semibold mt-0.5 truncate">{template.name}</h2>
        </div>
      </div>

      {/* Stepper */}
      <div className="flex items-center gap-2 px-4 py-3 border-b border-border-subtle bg-main/30">
        {steps.map((s, i) => {
          const done = i < currentIdx;
          const active = i === currentIdx;
          return (
            <div key={s.id} className="flex items-center gap-2 min-w-0">
              <div
                className={`grid place-items-center w-[18px] h-[18px] rounded-full text-[9px] font-bold font-mono border ${
                  done
                    ? "bg-success border-success text-main"
                    : active
                      ? "bg-brand border-brand text-main"
                      : "border-border-subtle text-text-dim"
                }`}
                style={active ? { boxShadow: "0 0 8px var(--color-brand)" } : undefined}
              >
                {done ? <Check className="w-2.5 h-2.5" /> : i + 1}
              </div>
              <span
                className={`font-mono text-[11px] truncate ${
                  active ? "" : done ? "text-text-dim" : "text-text-dim/60"
                }`}
              >
                {s.label}
              </span>
              {i < steps.length - 1 && (
                <div className="w-6 h-px bg-border-subtle shrink-0" />
              )}
            </div>
          );
        })}
      </div>

      {/* Body */}
      <div className="flex-1 overflow-y-auto p-4">
        {step === "permissions" && (
          <div className="flex flex-col gap-3.5">
            <p className="text-[13px] leading-relaxed text-text-dim">
              {template.description}
            </p>
            {(template.tags ?? []).length > 0 && (
              <div className="flex flex-wrap gap-1.5">
                {template.tags!.map((tag) => (
                  <span
                    key={tag}
                    className="px-2 py-0.5 rounded-full text-[10px] font-bold bg-accent/10 text-accent"
                  >
                    {tag}
                  </span>
                ))}
              </div>
            )}
            {template.setup_instructions && (
              <div>
                <div className="text-[10px] font-bold uppercase tracking-widest text-text-dim mb-1.5">
                  {t("mcp.setup_instructions", { defaultValue: "Setup" })}
                </div>
                <pre className="text-[11.5px] leading-relaxed whitespace-pre-wrap font-sans p-3 rounded-lg bg-main/40 border border-border-subtle text-text-dim">
                  {template.setup_instructions}
                </pre>
              </div>
            )}
            {hasRequiredEnv && (
              <div>
                <div className="text-[10px] font-bold uppercase tracking-widest text-text-dim mb-1.5">
                  {t("mcp.wizard.requires", { defaultValue: "Requires" })}
                </div>
                <div className="flex flex-col gap-1">
                  {requiredEnv.map((e) => (
                    <div
                      key={e.name}
                      className="flex items-center gap-2 p-2 rounded-md bg-main/40 border border-border-subtle text-[11.5px]"
                    >
                      <Key className="w-3 h-3 text-text-dim shrink-0" />
                      <code className="font-mono text-[11px] font-bold">{e.name}</code>
                      {e.label && (
                        <span className="text-text-dim truncate flex-1">{e.label}</span>
                      )}
                      {e.get_url && (
                        <a
                          href={e.get_url}
                          target="_blank"
                          rel="noopener noreferrer"
                          className="text-brand hover:underline shrink-0"
                          aria-label={t("mcp.wizard.get_credential", { defaultValue: "Get credential" })}
                        >
                          <ExternalLink className="h-3 w-3" />
                        </a>
                      )}
                    </div>
                  ))}
                </div>
              </div>
            )}
            <div className="rounded-md border border-brand/25 bg-brand/5 p-3 text-[11.5px] text-text-dim leading-relaxed">
              {t("mcp.wizard.review_notice", {
                defaultValue:
                  "After install the server's tools start gated by the approval policy — destructive calls require human sign-off until you scope them down.",
              })}
            </div>
            <label className="flex items-center gap-2 cursor-pointer text-[12.5px] text-text-dim">
              <input
                type="checkbox"
                checked={accepted}
                onChange={(e) => setAccepted(e.target.checked)}
                className="accent-[var(--color-brand)]"
              />
              {t("mcp.wizard.confirm_review", {
                defaultValue: "I've reviewed what this server can do.",
              })}
            </label>
          </div>
        )}

        {step === "configure" && (
          <div className="flex flex-col gap-4">
            <div className="text-[12.5px] text-text-dim">
              {t("mcp.env_setup_desc")}
            </div>
            {requiredEnv.map((e) => (
              <div key={e.name} className="flex flex-col gap-1.5">
                <div className="flex items-center gap-1.5">
                  <label
                    htmlFor={`wiz-env-${e.name}`}
                    className="text-[10px] font-bold uppercase tracking-widest text-text-dim"
                  >
                    {e.label || e.name}
                  </label>
                  {e.get_url && (
                    <a
                      href={e.get_url}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="text-brand hover:underline"
                    >
                      <ExternalLink className="h-3 w-3" />
                    </a>
                  )}
                </div>
                {e.help && <span className="text-[10px] text-text-dim/70">{e.help}</span>}
                <input
                  id={`wiz-env-${e.name}`}
                  type={e.is_secret ? "password" : "text"}
                  value={envInputs[e.name] ?? ""}
                  onChange={(ev) =>
                    setEnvInputs((prev) => ({ ...prev, [e.name]: ev.target.value }))
                  }
                  placeholder={e.label || e.name}
                  className="w-full rounded-lg border border-border-subtle bg-main px-3 py-2 text-sm font-mono focus:border-brand focus:outline-none focus:ring-2 focus:ring-brand/10 transition-colors"
                />
              </div>
            ))}
            <div className="rounded-md border border-success/25 bg-success/5 p-3 text-[11.5px] text-text-dim leading-relaxed flex items-start gap-2">
              <ShieldCheck className="w-3.5 h-3.5 text-success shrink-0 mt-0.5" />
              <span>
                {t("mcp.wizard.creds_note", {
                  defaultValue:
                    "Credentials are encrypted in the local vault and only released to this server's process at connect-time.",
                })}
              </span>
            </div>
          </div>
        )}

        {step === "installing" && (
          <div className="grid place-items-center min-h-48 text-center">
            <div className="flex flex-col items-center gap-3">
              <div className="relative">
                <div className="w-12 h-12 rounded-full border-2 border-brand/20" />
                <div className="absolute inset-0 w-12 h-12 rounded-full border-2 border-brand border-t-transparent animate-spin" />
              </div>
              <div className="font-mono text-[12.5px]">
                {t("mcp.wizard.installing", {
                  name: template.name,
                  defaultValue: "Installing {{name}}…",
                })}
              </div>
              <div className="text-[11px] text-text-dim">
                {t("mcp.wizard.installing_sub", {
                  defaultValue: "Persisting config, registering with the runtime.",
                })}
              </div>
            </div>
          </div>
        )}

        {step === "done" && (
          <div className="flex flex-col items-center text-center gap-3 pt-4">
            <div
              className="grid place-items-center w-14 h-14 rounded-full bg-success/15 border-2 border-success/40"
              style={{ boxShadow: "0 0 24px color-mix(in oklab, var(--color-success) 25%, transparent)" }}
            >
              <Check className="w-6 h-6 text-success" />
            </div>
            <h3 className="text-[17px] font-semibold">
              {t("mcp.wizard.done_title", { defaultValue: "Installed" })}
            </h3>
            <p className="text-[12.5px] text-text-dim max-w-sm">
              {t("mcp.wizard.done_body", {
                name: template.name,
                defaultValue: "{{name}} is connected and exposing its tools to your agents.",
              })}
            </p>
          </div>
        )}
      </div>

      {/* Footer */}
      <div className="flex items-center gap-2 p-3 border-t border-border-subtle">
        {step === "permissions" && (
          <>
            <Button variant="ghost" onClick={onClose}>
              {t("common.cancel")}
            </Button>
            <Button
              className="ml-auto"
              disabled={!accepted}
              onClick={() => (hasRequiredEnv ? setStep("configure") : runInstall())}
            >
              {hasRequiredEnv
                ? t("common.continue", { defaultValue: "Continue" })
                : t("mcp.catalog_install")}
            </Button>
          </>
        )}
        {step === "configure" && (
          <>
            <Button variant="ghost" onClick={() => setStep("permissions")}>
              {t("common.back", { defaultValue: "Back" })}
            </Button>
            <Button
              className="ml-auto"
              leftIcon={<Download className="h-3.5 w-3.5" />}
              isLoading={addMutation.isPending}
              onClick={runInstall}
            >
              {t("mcp.catalog_install")}
            </Button>
          </>
        )}
        {step === "installing" && (
          <Button variant="ghost" disabled className="ml-auto">
            {t("common.cancel")}
          </Button>
        )}
        {step === "done" && (
          <>
            <Button variant="ghost" onClick={onClose}>
              {t("common.close", { defaultValue: "Close" })}
            </Button>
            <Button
              className="ml-auto"
              onClick={() => {
                onSuccess();
                onClose();
              }}
            >
              {t("mcp.wizard.view_servers", { defaultValue: "View servers" })}
            </Button>
          </>
        )}
      </div>
    </div>
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
  const [detailsServer, setDetailsServer] = useState<McpServerConfigured | null>(null);
  const [detailsCatalog, setDetailsCatalog] = useState<McpCatalogEntry | null>(null);
  const [form, setForm] = useState<ServerFormState>(defaultForm);
  const [searchQuery, setSearchQuery] = useState("");
  const [statusFilter, setStatusFilter] = useState<StatusFilter>("all");
  const [catalogSearch, setCatalogSearch] = useState("");
  const [installingTemplate, setInstallingTemplate] = useState<McpCatalogEntry | null>(null);

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

  // Catalog install drives the 4-step CatalogInstallWizard component;
  // bookkeeping (mutation, env inputs, success/error toasts) lives there.
  // The page only owns which template is currently being installed.

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
            tab === "servers" ? "bg-primary/10 text-primary shadow-sm" : "text-text-dim hover:text-text"
          }`}
        >
          <Plug className="h-3.5 w-3.5" />
          {t("mcp.tab_my_servers")}
          {configured.length > 0 && (
            <span className={`ml-1 px-1.5 py-0.5 rounded-full text-[9px] font-bold ${tab === "servers" ? "bg-primary/20 text-primary" : "bg-border-subtle text-text-dim"}`}>
              {configured.length}
            </span>
          )}
        </button>
        <button
          onClick={() => setTab("catalog")}
          className={`flex items-center gap-1.5 px-4 py-2 rounded-lg text-xs font-bold transition-colors ${
            tab === "catalog" ? "bg-primary/10 text-primary shadow-sm" : "text-text-dim hover:text-text"
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
                  className="w-full rounded-xl border border-border-subtle bg-surface pl-10 pr-4 py-2.5 text-sm font-medium text-text-main placeholder:text-text-dim/40 focus:border-primary focus:outline-none focus:ring-2 focus:ring-primary/10 hover:border-primary/20 transition-colors duration-200 shadow-sm"
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
                        ? "bg-primary/10 text-primary shadow-sm"
                        : "text-text-dim hover:text-text"
                    }`}
                  >
                    <Filter className="h-3 w-3" />
                    {label}
                    <span className={`px-1 py-0.5 rounded-full text-[8px] font-bold ${
                      statusFilter === value ? "bg-primary/20 text-primary" : "bg-border-subtle text-text-dim"
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
                    onViewDetail={() => setDetailsServer(server)}
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
                className="w-full rounded-xl border border-border-subtle bg-surface pl-10 pr-4 py-2.5 text-sm font-medium text-text-main placeholder:text-text-dim/40 focus:border-primary focus:outline-none focus:ring-2 focus:ring-primary/10 hover:border-primary/20 transition-colors duration-200 shadow-sm"
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
            <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3 2xl:grid-cols-4">
              {filteredTemplates.map((tpl) => {
                const alreadyAdded = tpl.installed || installedTemplateIds.has(tpl.id);
                return (
                  <CatalogCard
                    key={tpl.id}
                    tpl={tpl}
                    alreadyAdded={alreadyAdded}
                    onViewDetail={() => setDetailsCatalog(tpl)}
                    onInstall={() => setInstallingTemplate(tpl)}
                    t={t}
                  />
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
            <span className="text-[10px] font-black uppercase tracking-widest text-text-dim">
              {t("mcp.transport_type")}
            </span>
            <div className="flex gap-2">
              {(["stdio", "sse", "http"] as TransportType[]).map((tt) => (
                <button
                  key={tt}
                  onClick={() => updateField("transportType", tt)}
                  className={`flex items-center gap-1.5 rounded-xl border px-3 py-2 text-xs font-bold transition-colors ${
                    form.transportType === tt
                      ? "border-primary bg-primary/10 text-primary"
                      : "border-border-subtle bg-surface text-text-dim hover:border-primary/20"
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
                <span className="text-[10px] font-black uppercase tracking-widest text-text-dim">
                  {t("mcp.args")}
                </span>
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
                <label htmlFor="mcp-server-headers" className="text-[10px] font-black uppercase tracking-widest text-text-dim">
                  {t("mcp.headers")}
                </label>
                <textarea
                  id="mcp-server-headers"
                  value={form.headers}
                  onChange={(e) => updateField("headers", e.target.value)}
                  placeholder={t("mcp.headers_placeholder")}
                  rows={2}
                  className="w-full rounded-xl border border-border-subtle bg-surface px-4 py-2.5 text-sm font-mono text-text-main placeholder:text-text-dim/40 focus:border-primary focus:outline-none focus:ring-2 focus:ring-primary/10 hover:border-primary/20 transition-colors duration-200 shadow-sm resize-none"
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
            <span className="text-[10px] font-black uppercase tracking-widest text-text-dim">
              {t("mcp.env")}
            </span>
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

      {/* Catalog install wizard — Permissions / Configure / Installing / Done */}
      <DrawerPanel
        isOpen={!!installingTemplate}
        onClose={() => setInstallingTemplate(null)}
        title={installingTemplate?.name ?? ""}
        size="md"
      >
        {installingTemplate && (
          <CatalogInstallWizard
            template={installingTemplate}
            onClose={() => setInstallingTemplate(null)}
            onSuccess={() => setTab("servers")}
            t={t}
          />
        )}
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

      {/* Server detail drawer — tabbed Tools / Logs / Config */}
      <DrawerPanel
        isOpen={!!detailsServer}
        onClose={() => setDetailsServer(null)}
        title={detailsServer?.name ?? ""}
        size="lg"
      >
        {detailsServer && (
          <ServerDetailBody
            server={detailsServer}
            conn={connectedMap.get(serverIdentityOf(detailsServer))}
            onClose={() => setDetailsServer(null)}
            onEdit={() => openEdit(detailsServer)}
            onEditTaintPolicy={() => setTaintEditingServer(detailsServer)}
            onDelete={() => deleteServer(detailsServer)}
          />
        )}
      </DrawerPanel>

      {/* Catalog template detail drawer */}
      <DrawerPanel
        isOpen={!!detailsCatalog}
        onClose={() => setDetailsCatalog(null)}
        title={detailsCatalog?.name ?? ""}
        size="md"
      >
        {detailsCatalog && (() => {
          const alreadyAdded = detailsCatalog.installed || installedTemplateIds.has(detailsCatalog.id);
          return (
            <div className="p-5 space-y-5">
              <div className="flex items-start gap-3">
                <div className={`w-12 h-12 rounded-xl flex items-center justify-center shrink-0 ${
                  alreadyAdded ? "bg-success/15 text-success" : "bg-brand/10 text-brand"
                }`}>
                  {detailsCatalog.icon
                    ? <CatalogIcon icon={detailsCatalog.icon} className="w-5 h-5" />
                    : <Plug className="w-5 h-5" />}
                </div>
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2 flex-wrap">
                    <h2 className="text-lg font-black tracking-tight truncate">{detailsCatalog.name}</h2>
                    {alreadyAdded && (
                      <Badge variant="success" dot>
                        <Check className="h-3 w-3 mr-0.5" />
                        {t("mcp.catalog_installed")}
                      </Badge>
                    )}
                  </div>
                  {detailsCatalog.category && (
                    <p className="text-[10px] font-black uppercase tracking-widest text-text-dim/60 mt-0.5">{detailsCatalog.category}</p>
                  )}
                </div>
              </div>

              <p className="text-sm text-text-dim leading-relaxed whitespace-pre-wrap">{detailsCatalog.description}</p>

              {(detailsCatalog.tags ?? []).length > 0 && (
                <div className="flex flex-wrap gap-1.5">
                  {detailsCatalog.tags!.map(tag => (
                    <span key={tag} className="px-2 py-0.5 rounded-full text-[10px] font-bold bg-brand/10 text-brand">{tag}</span>
                  ))}
                </div>
              )}

              {(detailsCatalog.required_env ?? []).length > 0 && (
                <div>
                  <p className="text-[10px] font-black uppercase tracking-widest text-text-dim/60 mb-2">
                    {t("mcp.required_env", { defaultValue: "Required environment" })}
                  </p>
                  <div className="space-y-1.5">
                    {(detailsCatalog.required_env ?? []).map(e => (
                      <div key={e.name} className="flex items-center gap-2 p-2 rounded-lg bg-main/40 border border-border-subtle/50">
                        <Key className="w-3 h-3 text-text-dim/60 shrink-0" />
                        <code className="font-mono text-[11px] font-bold text-text-main">{e.name}</code>
                        {e.label && <span className="text-[10px] text-text-dim truncate flex-1">{e.label}</span>}
                        {e.get_url && (
                          <a href={e.get_url} target="_blank" rel="noopener noreferrer" className="text-brand hover:underline shrink-0" aria-label="Get key">
                            <ExternalLink className="h-3 w-3" />
                          </a>
                        )}
                      </div>
                    ))}
                  </div>
                </div>
              )}

              {detailsCatalog.setup_instructions && (
                <div>
                  <p className="text-[10px] font-black uppercase tracking-widest text-text-dim/60 mb-2">
                    {t("mcp.setup_instructions", { defaultValue: "Setup" })}
                  </p>
                  <pre className="text-[11px] text-text-dim leading-relaxed whitespace-pre-wrap font-sans p-3 rounded-lg bg-main/40 border border-border-subtle/50">{detailsCatalog.setup_instructions}</pre>
                </div>
              )}

              <div className="pt-3 border-t border-border-subtle/50">
                <Button
                  variant="primary"
                  className="w-full"
                  disabled={alreadyAdded}
                  leftIcon={alreadyAdded ? <Check className="w-3.5 h-3.5" /> : <Download className="w-3.5 h-3.5" />}
                  onClick={() => {
                    const tpl = detailsCatalog;
                    setDetailsCatalog(null);
                    setInstallingTemplate(tpl);
                  }}
                >
                  {alreadyAdded ? t("mcp.catalog_installed") : t("mcp.catalog_install")}
                </Button>
              </div>
            </div>
          );
        })()}
      </DrawerPanel>
    </div>
  );
}
