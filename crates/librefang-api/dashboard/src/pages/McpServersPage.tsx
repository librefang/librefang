import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  listMcpServers, addMcpServer, updateMcpServer, deleteMcpServer,
  type McpServerConfigured, type McpServerConnected, type McpServerTransport,
} from "../api";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { PageHeader } from "../components/ui/PageHeader";
import { ListSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { Modal } from "../components/ui/Modal";
import { ConfirmDialog } from "../components/ui/ConfirmDialog";
import { Input } from "../components/ui/Input";
import { useUIStore } from "../lib/store";
import { useCreateShortcut } from "../lib/useCreateShortcut";
import {
  Plug, Plus, Trash2, Settings, ChevronDown, ChevronUp, Wrench, Terminal, Globe, Radio,
} from "lucide-react";

const REFRESH_MS = 30000;

type TransportType = "stdio" | "sse" | "http";

interface ServerFormState {
  name: string;
  transportType: TransportType;
  command: string;
  args: string;
  url: string;
  timeout: number;
  env: string;
  headers: string;
}

const defaultForm: ServerFormState = {
  name: "",
  transportType: "stdio",
  command: "",
  args: "",
  url: "",
  timeout: 30,
  env: "",
  headers: "",
};

function formToPayload(form: ServerFormState): McpServerConfigured {
  let transport: McpServerTransport;
  if (form.transportType === "stdio") {
    transport = {
      type: "stdio",
      command: form.command,
      args: form.args.split("\n").map(s => s.trim()).filter(Boolean),
    };
  } else {
    transport = { type: form.transportType, url: form.url };
  }

  const headers = form.headers.split("\n").map(s => s.trim()).filter(Boolean);
  const result: McpServerConfigured = {
    name: form.name,
    transport,
    timeout_secs: form.timeout || 30,
    env: form.env.split("\n").map(s => s.trim()).filter(Boolean),
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
    args: (transport.args ?? []).join("\n"),
    url: transport.url ?? "",
    timeout: server.timeout_secs ?? 30,
    env: (server.env ?? []).join("\n"),
    headers: (server.headers ?? []).join("\n"),
  };
}

function getTransportType(server: McpServerConfigured): TransportType {
  return server.transport?.type ?? "stdio";
}

function getTransportDetail(server: McpServerConfigured): string {
  if (!server.transport) return "—";
  if (server.transport.type === "stdio") {
    return `${server.transport.command ?? ""} ${(server.transport.args ?? []).join(" ")}`.trim();
  }
  return server.transport.url ?? "—";
}

function TransportIcon({ type }: { type: TransportType }) {
  switch (type) {
    case "stdio": return <Terminal className="h-4 w-4" />;
    case "sse": return <Radio className="h-4 w-4" />;
    case "http": return <Globe className="h-4 w-4" />;
  }
}

export function McpServersPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const addToast = useUIStore((s) => s.addToast);

  const [showAddModal, setShowAddModal] = useState(false);
  const [editingServer, setEditingServer] = useState<McpServerConfigured | null>(null);
  const [deletingServer, setDeletingServer] = useState<string | null>(null);
  const [expandedTools, setExpandedTools] = useState<Set<string>>(new Set());
  const [form, setForm] = useState<ServerFormState>(defaultForm);

  useCreateShortcut(() => setShowAddModal(true));

  const serversQuery = useQuery({
    queryKey: ["mcp-servers"],
    queryFn: listMcpServers,
    refetchInterval: REFRESH_MS,
  });

  const addMutation = useMutation({
    mutationFn: (server: McpServerConfigured) => addMcpServer(server),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["mcp-servers"] });
      setShowAddModal(false);
      setForm(defaultForm);
      addToast(t("mcp.add_success"), "success");
    },
    onError: (e: any) => addToast(e?.message || t("mcp.add_failed"), "error"),
  });

  const updateMutation = useMutation({
    mutationFn: ({ name, server }: { name: string; server: Partial<McpServerConfigured> }) => updateMcpServer(name, server),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["mcp-servers"] });
      setEditingServer(null);
      setForm(defaultForm);
      addToast(t("mcp.update_success"), "success");
    },
    onError: (e: any) => addToast(e?.message || t("mcp.update_failed"), "error"),
  });

  const deleteMutation = useMutation({
    mutationFn: (name: string) => deleteMcpServer(name),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["mcp-servers"] });
      setDeletingServer(null);
      addToast(t("mcp.delete_success"), "success");
    },
    onError: (e: any) => addToast(e?.message || t("mcp.delete_failed"), "error"),
  });

  const data = serversQuery.data;
  const configured = data?.configured ?? [];
  const connected = data?.connected ?? [];

  const connectedMap = new Map<string, McpServerConnected>();
  for (const c of connected) {
    connectedMap.set(c.name, c);
  }

  function toggleTools(name: string) {
    setExpandedTools(prev => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });
  }

  function openAdd() {
    setForm(defaultForm);
    setShowAddModal(true);
  }

  function openEdit(server: McpServerConfigured) {
    setForm(configuredToForm(server));
    setEditingServer(server);
  }

  function handleSubmit() {
    const payload = formToPayload(form);
    if (editingServer) {
      updateMutation.mutate({ name: editingServer.name, server: payload });
    } else {
      addMutation.mutate(payload);
    }
  }

  const isModalOpen = showAddModal || editingServer !== null;
  const isSubmitting = addMutation.isPending || updateMutation.isPending;

  const updateField = <K extends keyof ServerFormState>(key: K, value: ServerFormState[K]) =>
    setForm(prev => ({ ...prev, [key]: value }));

  return (
    <div className="space-y-6">
      <PageHeader
        icon={<Plug className="h-5 w-5" />}
        badge="MCP"
        title={t("mcp.title")}
        subtitle={t("mcp.subtitle")}
        isFetching={serversQuery.isFetching}
        onRefresh={() => serversQuery.refetch()}
        helpText={t("mcp.help")}
        actions={
          <Button size="sm" leftIcon={<Plus className="h-3.5 w-3.5" />} onClick={openAdd}>
            {t("mcp.add_server")}
          </Button>
        }
      />

      {/* Summary badges */}
      {data && (
        <div className="flex items-center gap-3 flex-wrap">
          <Badge variant="default">{t("mcp.total_configured", { count: data.total_configured })}</Badge>
          <Badge variant={data.total_connected > 0 ? "success" : "default"} dot>
            {t("mcp.total_connected", { count: data.total_connected })}
          </Badge>
        </div>
      )}

      {/* Loading state */}
      {serversQuery.isLoading && <ListSkeleton rows={3} />}

      {/* Empty state */}
      {!serversQuery.isLoading && configured.length === 0 && (
        <EmptyState
          icon={<Plug className="h-10 w-10" />}
          title={t("mcp.empty")}
          description={t("mcp.empty_desc")}
          action={
            <Button size="sm" leftIcon={<Plus className="h-3.5 w-3.5" />} onClick={openAdd}>
              {t("mcp.add_server")}
            </Button>
          }
        />
      )}

      {/* Server cards */}
      {configured.length > 0 && (
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {configured.map((server) => {
            const conn = connectedMap.get(server.name);
            const isConnected = conn?.connected ?? false;
            const toolsCount = conn?.tools_count ?? 0;
            const isExpanded = expandedTools.has(server.name);

            return (
              <Card key={server.name} padding="none" className="flex flex-col">
                <div className="p-4 flex flex-col gap-3">
                  {/* Header row */}
                  <div className="flex items-start justify-between gap-2">
                    <div className="flex items-center gap-2.5 min-w-0">
                      <div className="p-2 rounded-xl bg-brand/10 text-brand shrink-0">
                        <Plug className="h-4 w-4" />
                      </div>
                      <div className="min-w-0">
                        <h3 className="text-sm font-bold tracking-tight truncate">{server.name}</h3>
                        <div className="flex items-center gap-1.5 mt-0.5">
                          <TransportIcon type={getTransportType(server)} />
                          <span className="text-[10px] font-bold uppercase tracking-wider text-text-dim">
                            {getTransportType(server)}
                          </span>
                        </div>
                      </div>
                    </div>
                    <Badge variant={isConnected ? "success" : "error"} dot>
                      {isConnected ? t("mcp.connected") : t("mcp.disconnected")}
                    </Badge>
                  </div>

                  {/* Transport detail */}
                  <div className="text-xs text-text-dim font-mono truncate">
                    {getTransportDetail(server)}
                  </div>

                  {/* Tools count */}
                  <div className="flex items-center gap-2">
                    <Wrench className="h-3.5 w-3.5 text-text-dim" />
                    <span className="text-xs text-text-dim">
                      {toolsCount > 0 ? t("mcp.tools_count", { count: toolsCount }) : t("mcp.no_tools")}
                    </span>
                  </div>
                </div>

                {/* Expand tools section */}
                {toolsCount > 0 && (
                  <>
                    <button
                      onClick={() => toggleTools(server.name)}
                      className="flex items-center justify-center gap-1.5 py-2 border-t border-border-subtle text-xs font-bold text-text-dim hover:text-brand hover:bg-surface-hover transition-colors"
                      aria-expanded={isExpanded}
                      aria-label={isExpanded ? t("mcp.hide_tools") : t("mcp.show_tools")}
                    >
                      {isExpanded ? <ChevronUp className="h-3.5 w-3.5" /> : <ChevronDown className="h-3.5 w-3.5" />}
                      {t("mcp.tools")}
                    </button>
                    {isExpanded && conn?.tools && (
                      <div className="border-t border-border-subtle px-4 py-3 space-y-1.5 max-h-48 overflow-y-auto scrollbar-thin">
                        {conn.tools.map((tool) => (
                          <div key={tool.name} className="flex flex-col">
                            <span className="text-xs font-bold text-text-main">{tool.name}</span>
                            {tool.description && (
                              <span className="text-[10px] text-text-dim leading-snug">{tool.description}</span>
                            )}
                          </div>
                        ))}
                      </div>
                    )}
                  </>
                )}

                {/* Actions */}
                <div className="flex border-t border-border-subtle">
                  <button
                    onClick={() => openEdit(server)}
                    className="flex-1 flex items-center justify-center gap-1.5 py-2.5 text-xs font-bold text-text-dim hover:text-brand hover:bg-surface-hover transition-colors rounded-bl-xl sm:rounded-bl-2xl"
                    aria-label={t("mcp.edit_server")}
                  >
                    <Settings className="h-3.5 w-3.5" />
                    {t("common.edit")}
                  </button>
                  <div className="w-px bg-border-subtle" />
                  <button
                    onClick={() => setDeletingServer(server.name)}
                    className="flex-1 flex items-center justify-center gap-1.5 py-2.5 text-xs font-bold text-text-dim hover:text-error hover:bg-error/5 transition-colors rounded-br-xl sm:rounded-br-2xl"
                    aria-label={t("mcp.delete_server")}
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                    {t("common.delete")}
                  </button>
                </div>
              </Card>
            );
          })}
        </div>
      )}

      {/* Add / Edit Modal */}
      <Modal
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

          {/* stdio fields */}
          {form.transportType === "stdio" && (
            <>
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
                <textarea
                  value={form.args}
                  onChange={(e) => updateField("args", e.target.value)}
                  placeholder={t("mcp.args_placeholder")}
                  rows={3}
                  className="w-full rounded-xl border border-border-subtle bg-surface px-4 py-2.5 text-sm font-medium text-text-main placeholder:text-text-dim/40 focus:border-brand focus:outline-none focus:ring-2 focus:ring-brand/10 hover:border-brand/20 transition-colors duration-200 shadow-sm resize-none"
                />
              </div>
            </>
          )}

          {/* sse/http fields */}
          {(form.transportType === "sse" || form.transportType === "http") && (
            <Input
              label={t("mcp.url")}
              value={form.url}
              onChange={(e) => updateField("url", e.target.value)}
              placeholder={t("mcp.url_placeholder")}
            />
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
            <textarea
              value={form.env}
              onChange={(e) => updateField("env", e.target.value)}
              placeholder={t("mcp.env_placeholder")}
              rows={2}
              className="w-full rounded-xl border border-border-subtle bg-surface px-4 py-2.5 text-sm font-medium font-mono text-text-main placeholder:text-text-dim/40 focus:border-brand focus:outline-none focus:ring-2 focus:ring-brand/10 hover:border-brand/20 transition-colors duration-200 shadow-sm resize-none"
            />
          </div>

          {/* Headers (only for sse/http) */}
          {(form.transportType === "sse" || form.transportType === "http") && (
            <div className="flex flex-col gap-1.5">
              <label className="text-[10px] font-black uppercase tracking-widest text-text-dim">
                {t("mcp.headers")}
              </label>
              <textarea
                value={form.headers}
                onChange={(e) => updateField("headers", e.target.value)}
                placeholder={t("mcp.headers_placeholder")}
                rows={2}
                className="w-full rounded-xl border border-border-subtle bg-surface px-4 py-2.5 text-sm font-medium font-mono text-text-main placeholder:text-text-dim/40 focus:border-brand focus:outline-none focus:ring-2 focus:ring-brand/10 hover:border-brand/20 transition-colors duration-200 shadow-sm resize-none"
              />
            </div>
          )}

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
      </Modal>

      {/* Delete confirmation */}
      <ConfirmDialog
        isOpen={!!deletingServer}
        title={t("mcp.delete_server")}
        message={t("mcp.delete_confirm")}
        tone="destructive"
        confirmLabel={t("common.delete")}
        onConfirm={() => { if (deletingServer) deleteMutation.mutate(deletingServer); }}
        onClose={() => setDeletingServer(null)}
      />
    </div>
  );
}
