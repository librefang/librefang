import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "@tanstack/react-router";
import { listAgents, getAgentDetail, spawnAgent, suspendAgent, resumeAgent } from "../api";
import { PageHeader } from "../components/ui/PageHeader";
import { CardSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { Card } from "../components/ui/Card";
import { Input } from "../components/ui/Input";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { Avatar } from "../components/ui/Avatar";
import { Search, Users, MessageCircle, X, Cpu, Wrench, Shield, Plus, Loader2, Pause, Play, Clock, Brain, Zap } from "lucide-react";

const REFRESH_MS = 30000;

function getStatusVariant(status?: string) {
  const value = (status ?? "").toLowerCase();
  if (value === "running") return "success";
  if (value === "suspended") return "warning";
  if (value === "idle") return "warning";
  if (value === "error" || value === "crashed") return "error";
  return "default";
}

export function AgentsPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [search, setSearch] = useState("");
  const [detailAgent, setDetailAgent] = useState<any>(null);
  const [, setDetailLoading] = useState(false);
  const [showCreate, setShowCreate] = useState(false);
  const [createMode, setCreateMode] = useState<"template" | "toml">("template");
  const [templateName, setTemplateName] = useState("");
  const [manifestToml, setManifestToml] = useState("");
  const queryClient = useQueryClient();
  const spawnMutation = useMutation({
    mutationFn: spawnAgent,
    onSuccess: () => { queryClient.invalidateQueries({ queryKey: ["agents"] }); setShowCreate(false); setTemplateName(""); setManifestToml(""); }
  });

  const agentsQuery = useQuery({
    queryKey: ["agents", "list"],
    queryFn: listAgents,
    refetchInterval: REFRESH_MS
  });

  const agents = agentsQuery.data ?? [];
  const filteredAgents = agents
    .filter(a => a.name.toLowerCase().includes(search.toLowerCase()) || a.id.toLowerCase().includes(search.toLowerCase()))
    .sort((a, b) => {
      // 1. Suspended agents last
      const aSusp = (a.state || "").toLowerCase() === "suspended" ? 1 : 0;
      const bSusp = (b.state || "").toLowerCase() === "suspended" ? 1 : 0;
      if (aSusp !== bSusp) return aSusp - bSusp;
      // 2. Core agents first, hands second
      const aHand = a.name.includes("-hand") ? 1 : 0;
      const bHand = b.name.includes("-hand") ? 1 : 0;
      if (aHand !== bHand) return aHand - bHand;
      // 3. Alphabetical
      return a.name.localeCompare(b.name);
    });

  // Group: core agents and hands
  const coreAgents = filteredAgents.filter(a => !a.name.includes("-hand"));
  const handAgents = filteredAgents.filter(a => a.name.includes("-hand"));

  const renderAgentCard = (agent: any) => {
    const isSuspended = (agent.state || "").toLowerCase() === "suspended";
    return (
      <Card key={agent.id} hover padding="lg" className={`cursor-pointer ${isSuspended ? "opacity-60" : ""}`} onClick={async () => {
        setDetailLoading(true);
        try { const d = await getAgentDetail(agent.id); setDetailAgent(d); } catch { setDetailAgent({ name: agent.name, id: agent.id }); }
        setDetailLoading(false);
      }}>
        <div className="flex items-start justify-between gap-4 mb-5">
          <div className="flex items-center gap-3 min-w-0">
            <div className="relative">
              <Avatar fallback={agent.name} size="lg" />
              {!isSuspended && <span className="absolute -bottom-0.5 -right-0.5 w-3 h-3 rounded-full bg-success border-2 border-surface animate-pulse" />}
            </div>
            <div className="min-w-0">
              <h2 className="text-base font-black tracking-tight truncate">{agent.name}</h2>
              <p className="text-[10px] font-mono text-text-dim/50 truncate mt-0.5">{agent.id.slice(0, 8)}</p>
            </div>
          </div>
          <Badge variant={getStatusVariant(agent.state)} dot>
            {agent.state ? t(`common.${agent.state.toLowerCase()}`, { defaultValue: agent.state }) : t("common.idle")}
          </Badge>
        </div>
        <div className="space-y-2.5 mb-5">
          <div className="flex items-center gap-3 text-xs">
            <div className="w-5 h-5 rounded bg-brand/10 flex items-center justify-center shrink-0"><Cpu className="w-3 h-3 text-brand" /></div>
            <span className="text-text-dim flex-1">{t("agents.model")}</span>
            <span className="font-black text-sm">{agent.model_name || t("common.unknown")}</span>
          </div>
          <div className="flex items-center gap-3 text-xs">
            <div className="w-5 h-5 rounded bg-success/10 flex items-center justify-center shrink-0"><Shield className="w-3 h-3 text-success" /></div>
            <span className="text-text-dim flex-1">{t("agents.provider")}</span>
            <span className="font-black text-brand text-sm">{agent.model_provider || t("common.local")}</span>
          </div>
          <div className="flex items-center gap-3 text-xs">
            <div className="w-5 h-5 rounded bg-warning/10 flex items-center justify-center shrink-0"><Clock className="w-3 h-3 text-warning" /></div>
            <span className="text-text-dim flex-1">{t("agents.last_active")}</span>
            <span className="font-mono text-[10px]">{agent.last_active ? new Date(agent.last_active).toLocaleTimeString() : t("common.never")}</span>
          </div>
        </div>
        <div className="pt-4 border-t border-border-subtle/30 flex gap-2">
          {isSuspended ? (
            <Button variant="secondary" size="sm" className="flex-1" onClick={async (e) => { e.stopPropagation(); await resumeAgent(agent.id); agentsQuery.refetch(); }}>
              <Play className="h-3.5 w-3.5 mr-1" /> {t("agents.resume")}
            </Button>
          ) : (
            <Button variant="secondary" size="sm" className="flex-1" onClick={async (e) => { e.stopPropagation(); await suspendAgent(agent.id); agentsQuery.refetch(); }}>
              <Pause className="h-3.5 w-3.5 mr-1" /> {t("agents.suspend")}
            </Button>
          )}
          <Button variant="primary" size="sm" className="flex-1" onClick={(e) => { e.stopPropagation(); navigate({ to: "/chat", search: { agentId: agent.id } }); }}>
            <MessageCircle className="h-3.5 w-3.5 mr-1" /> {t("common.interact")}
          </Button>
        </div>
      </Card>
    );
  };

  return (
    <div className="flex flex-col gap-4 sm:gap-6 transition-colors duration-300">
      <div className="flex flex-col sm:flex-row justify-between items-start sm:items-end gap-3">
        <PageHeader
          badge={t("common.kernel_runtime")}
          title={t("agents.title")}
          subtitle={t("agents.subtitle")}
          isFetching={agentsQuery.isFetching}
          onRefresh={() => void agentsQuery.refetch()}
          icon={<Users className="h-4 w-4" />}
        />
        <Button variant="primary" onClick={() => setShowCreate(true)} className="shrink-0">
          <Plus className="w-4 h-4" />
          {t("agents.create_agent")}
        </Button>
      </div>

      <Input
        value={search}
        onChange={(e) => setSearch(e.target.value)}
        placeholder={t("common.search")}
        leftIcon={<Search className="h-4 w-4" />}
      />

      {agentsQuery.isLoading ? (
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
          {[1, 2, 3, 4, 5, 6].map((i) => <CardSkeleton key={i} />)}
        </div>
      ) : filteredAgents.length === 0 ? (
        search ? (
          <EmptyState
            title={t("agents.no_matching")}
            icon={<Search className="h-6 w-6" />}
          />
        ) : (
          <EmptyState
            title={t("common.no_data")}
            icon={<Users className="h-6 w-6" />}
          />
        )
      ) : (
        <div className="space-y-6">
          {/* Core Agents */}
          {coreAgents.length > 0 && (
            <div>
              <h3 className="text-[10px] font-bold uppercase tracking-widest text-text-dim/50 mb-3">{t("agents.core_agents")}</h3>
              <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3 stagger-children">
                {coreAgents.map(agent => renderAgentCard(agent))}
              </div>
            </div>
          )}
          {/* Hands */}
          {handAgents.length > 0 && (
            <div>
              <h3 className="text-[10px] font-bold uppercase tracking-widest text-text-dim/50 mb-3">{t("agents.hands")} ({handAgents.length})</h3>
              <div className="space-y-2 stagger-children">
                {handAgents.map(agent => {
                  const isSuspended = (agent.state || "").toLowerCase() === "suspended";
                  return (
                    <div key={agent.id}
                      className={`flex items-center gap-3 sm:gap-4 p-3 sm:p-4 rounded-xl sm:rounded-2xl border border-border-subtle hover:border-brand/30 transition-all cursor-pointer ${isSuspended ? "opacity-60 hover:opacity-100" : "bg-surface"}`}
                      onClick={async () => {
                        setDetailLoading(true);
                        try { const d = await getAgentDetail(agent.id); setDetailAgent(d); } catch { setDetailAgent({ name: agent.name, id: agent.id }); }
                        setDetailLoading(false);
                      }}>
                      <Avatar fallback={agent.name} size="md" />
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-1.5 sm:gap-2 flex-wrap">
                          <h3 className="text-xs sm:text-sm font-bold truncate">{agent.name}</h3>
                          <span className="text-[8px] px-1.5 py-0.5 rounded-full bg-purple-100 text-purple-600 dark:bg-purple-900/30 dark:text-purple-400 font-bold hidden sm:inline">{t("agents.hand_badge")}</span>
                          <Badge variant={getStatusVariant(agent.state)}>
                            {agent.state ? t(`common.${agent.state.toLowerCase()}`, { defaultValue: agent.state }) : t("common.idle")}
                          </Badge>
                        </div>
                        <div className="flex items-center gap-2 sm:gap-3 mt-0.5 sm:mt-1 text-[9px] sm:text-[10px] text-text-dim/60">
                          <span className="font-mono">{agent.id.slice(0, 8)}</span>
                          <span className="hidden sm:inline">{agent.model_name || t("common.unknown")}</span>
                          <span className="text-brand">{agent.model_provider}</span>
                        </div>
                      </div>
                      <div className="flex items-center gap-1 sm:gap-2 shrink-0" onClick={e => e.stopPropagation()}>
                        {isSuspended ? (
                          <Button variant="secondary" size="sm" onClick={async () => { await resumeAgent(agent.id); agentsQuery.refetch(); }}>
                            <Play className="h-3.5 w-3.5 mr-1" /> <span className="hidden sm:inline">{t("agents.resume")}</span>
                          </Button>
                        ) : (
                          <Button variant="secondary" size="sm" onClick={async () => { await suspendAgent(agent.id); agentsQuery.refetch(); }}>
                            <Pause className="h-3.5 w-3.5 mr-1" /> <span className="hidden sm:inline">{t("agents.suspend")}</span>
                          </Button>
                        )}
                        <Button variant="primary" size="sm" onClick={() => navigate({ to: "/chat", search: { agentId: agent.id } })}>
                          <MessageCircle className="h-3.5 w-3.5" />
                        </Button>
                      </div>
                    </div>
                  );
                })}
              </div>
            </div>
          )}
        </div>
      )}
      {/* Agent Detail Modal */}
      {detailAgent && (
        <div className="fixed inset-0 z-50 flex items-end sm:items-center justify-center bg-black/40 backdrop-blur-xl backdrop-saturate-150" onClick={() => setDetailAgent(null)}>
          <div className="bg-surface rounded-t-2xl sm:rounded-2xl shadow-2xl border border-border-subtle w-full sm:w-[560px] sm:max-w-[90vw] max-h-[85vh] sm:max-h-[80vh] overflow-y-auto animate-fade-in-scale" onClick={e => e.stopPropagation()}>
            {/* Modal Header */}
            <div className="px-6 py-5 border-b border-border-subtle sticky top-0 bg-surface/95 backdrop-blur-xl backdrop-saturate-150 z-10">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-4">
                  <div className="relative">
                    <Avatar fallback={detailAgent.name} size="lg" />
                    <span className="absolute -bottom-0.5 -right-0.5 w-3 h-3 rounded-full bg-success border-2 border-surface" />
                  </div>
                  <div>
                    <h3 className="text-lg font-black tracking-tight">{detailAgent.name}</h3>
                    <p className="text-[10px] text-text-dim font-mono mt-0.5">{detailAgent.id?.slice(0, 16)}...</p>
                  </div>
                </div>
                <button onClick={() => setDetailAgent(null)} className="p-2 rounded-xl hover:bg-main transition-colors"><X className="w-4 h-4" /></button>
              </div>
            </div>
            <div className="p-6 space-y-5">
              {/* Model */}
              {detailAgent.model && (
                <div>
                  <h4 className="text-[10px] font-black text-text-dim uppercase tracking-widest mb-3 flex items-center gap-2">
                    <div className="w-5 h-5 rounded bg-brand/10 flex items-center justify-center"><Cpu className="w-3 h-3 text-brand" /></div>
                    {t("agents.model")}
                  </h4>
                  <div className="p-4 rounded-xl bg-main/50 border border-border-subtle/50 space-y-2.5 text-xs">
                    <div className="flex justify-between items-center"><span className="text-text-dim">{t("agents.provider")}</span><span className="font-black text-brand">{detailAgent.model.provider}</span></div>
                    <div className="flex justify-between items-center"><span className="text-text-dim">{t("agents.model")}</span><span className="font-black">{detailAgent.model.model}</span></div>
                  </div>
                </div>
              )}

              {/* System Prompt */}
              {detailAgent.system_prompt && (
                <div>
                  <h4 className="text-[10px] font-black text-text-dim uppercase tracking-widest mb-3">{t("agents.system_prompt")}</h4>
                  <pre className="p-4 rounded-xl bg-main/50 border border-border-subtle/50 text-xs text-text-dim whitespace-pre-wrap max-h-40 overflow-y-auto leading-relaxed font-mono">{detailAgent.system_prompt}</pre>
                </div>
              )}

              {/* Capabilities */}
              {detailAgent.capabilities && (
                <div>
                  <h4 className="text-[10px] font-black text-text-dim uppercase tracking-widest mb-3 flex items-center gap-2">
                    <div className="w-5 h-5 rounded bg-success/10 flex items-center justify-center"><Wrench className="w-3 h-3 text-success" /></div>
                    {t("agents.capabilities")}
                  </h4>
                  <div className="flex flex-wrap gap-2">
                    {detailAgent.capabilities.tools && <Badge variant="brand" dot>{t("agents.tools_cap")}</Badge>}
                    {detailAgent.capabilities.network && <Badge variant="brand" dot>{t("agents.network")}</Badge>}
                  </div>
                </div>
              )}

              {/* Skills */}
              {detailAgent.skills && detailAgent.skills.length > 0 && (
                <div>
                  <h4 className="text-[10px] font-black text-text-dim uppercase tracking-widest mb-3">{t("agents.skills")}</h4>
                  <div className="flex flex-wrap gap-2">
                    {detailAgent.skills.map((s: string, i: number) => (
                      <Badge key={i} variant="default">{s}</Badge>
                    ))}
                  </div>
                </div>
              )}

              {/* Tags */}
              {detailAgent.tags && detailAgent.tags.length > 0 && (
                <div>
                  <h4 className="text-[10px] font-black text-text-dim uppercase tracking-widest mb-3">{t("agents.tags")}</h4>
                  <div className="flex flex-wrap gap-1.5">
                    {detailAgent.tags.map((tag: string, i: number) => (
                      <span key={i} className="text-[10px] px-2.5 py-1 rounded-lg bg-main border border-border-subtle/50 text-text-dim font-medium">{tag}</span>
                    ))}
                  </div>
                </div>
              )}

              {/* Mode */}
              {detailAgent.mode && (
                <div className="flex items-center gap-3 p-3 rounded-xl bg-main/50 border border-border-subtle/50">
                  <div className="w-5 h-5 rounded bg-warning/10 flex items-center justify-center"><Shield className="w-3 h-3 text-warning" /></div>
                  <span className="text-xs font-bold flex-1">{t("agents.mode")}</span>
                  <Badge variant="warning">{detailAgent.mode}</Badge>
                </div>
              )}

              {/* Thinking / Extended Reasoning */}
              {detailAgent.thinking && (
                <div>
                  <h4 className="text-[10px] font-black text-text-dim uppercase tracking-widest mb-3 flex items-center gap-2">
                    <div className="w-5 h-5 rounded bg-purple-500/10 flex items-center justify-center"><Brain className="w-3 h-3 text-purple-500" /></div>
                    {t("agents.thinking")}
                  </h4>
                  <div className="p-4 rounded-xl bg-main/50 border border-border-subtle/50 space-y-2.5 text-xs">
                    <div className="flex justify-between items-center">
                      <span className="text-text-dim">{t("agents.thinking_enabled")}</span>
                      <Badge variant={detailAgent.thinking.budget_tokens > 0 ? "success" : "default"}>
                        {detailAgent.thinking.budget_tokens > 0 ? t("common.yes") : t("common.no")}
                      </Badge>
                    </div>
                    <div className="flex justify-between items-center">
                      <span className="text-text-dim">{t("agents.budget_tokens")}</span>
                      <span className="font-black text-sm">{detailAgent.thinking.budget_tokens?.toLocaleString() ?? 0}</span>
                    </div>
                    <div className="flex justify-between items-center">
                      <span className="text-text-dim">{t("agents.stream_thinking")}</span>
                      <Badge variant={detailAgent.thinking.stream_thinking ? "brand" : "default"}>
                        {detailAgent.thinking.stream_thinking ? t("common.yes") : t("common.no")}
                      </Badge>
                    </div>
                    <p className="text-[10px] text-text-dim/50 flex items-center gap-1 pt-1">
                      <Zap className="w-3 h-3" />
                      {t("agents.thinking_hint")}
                    </p>
                  </div>
                </div>
              )}

              {/* Actions */}
              <div className="flex gap-2 pt-2 border-t border-border-subtle">
                <Button variant="primary" size="sm" className="flex-1" onClick={() => { setDetailAgent(null); navigate({ to: "/chat", search: { agentId: detailAgent.id } }); }}>
                  <MessageCircle className="w-3.5 h-3.5 mr-1" />
                  {t("common.interact")}
                </Button>
              </div>
            </div>
          </div>
        </div>
      )}

      {/* Create Agent Modal */}
      {showCreate && (
        <div className="fixed inset-0 z-50 flex items-end sm:items-center justify-center bg-black/30 backdrop-blur-xl backdrop-saturate-150" onClick={() => setShowCreate(false)}>
          <div className="bg-surface rounded-t-2xl sm:rounded-2xl shadow-2xl border border-border-subtle w-full sm:w-[480px] sm:max-w-[90vw] animate-fade-in-scale" onClick={e => e.stopPropagation()}>
            <div className="flex items-center justify-between px-5 py-3 border-b border-border-subtle">
              <h3 className="text-sm font-bold">{t("agents.create_agent")}</h3>
              <button onClick={() => setShowCreate(false)} className="p-1 rounded hover:bg-main"><X className="w-4 h-4" /></button>
            </div>
            <div className="p-5 space-y-4">
              {/* Mode tabs */}
              <div className="flex gap-2">
                <button onClick={() => setCreateMode("template")}
                  className={`px-3 py-1.5 rounded-lg text-xs font-bold transition-colors ${createMode === "template" ? "bg-brand text-white" : "bg-main text-text-dim"}`}>
                  {t("agents.from_template")}
                </button>
                <button onClick={() => setCreateMode("toml")}
                  className={`px-3 py-1.5 rounded-lg text-xs font-bold transition-colors ${createMode === "toml" ? "bg-brand text-white" : "bg-main text-text-dim"}`}>
                  {t("agents.from_toml")}
                </button>
              </div>

              {createMode === "template" ? (
                <div>
                  <label className="text-[10px] font-bold text-text-dim uppercase">{t("agents.template_name")}</label>
                  <input value={templateName} onChange={e => setTemplateName(e.target.value)}
                    placeholder={t("agents.template_placeholder")}
                    className="mt-1 w-full rounded-xl border border-border-subtle bg-main px-3 py-2 text-sm outline-none focus:border-brand" />
                  <p className="text-[9px] text-text-dim/50 mt-1">{t("agents.template_hint")}</p>
                </div>
              ) : (
                <div>
                  <label className="text-[10px] font-bold text-text-dim uppercase">{t("agents.manifest_toml")}</label>
                  <textarea value={manifestToml} onChange={e => setManifestToml(e.target.value)}
                    placeholder={'[agent]\nname = "my-agent"\n\n[model]\nprovider = "openai"\nmodel = "gpt-4o"\n\n[thinking]\nbudget_tokens = 10000\nstream_thinking = false'}
                    rows={12}
                    className="mt-1 w-full rounded-xl border border-border-subtle bg-main px-3 py-2 text-xs font-mono outline-none focus:border-brand resize-none" />
                  <p className="text-[9px] text-text-dim/50 mt-1 flex items-center gap-1">
                    <Brain className="w-3 h-3" />
                    {t("agents.thinking_toml_hint")}
                  </p>
                </div>
              )}

              {spawnMutation.error && (
                <p className="text-xs text-error">{(spawnMutation.error as any)?.message || String(spawnMutation.error)}</p>
              )}

              <div className="flex gap-2 pt-2">
                <Button variant="primary" className="flex-1"
                  onClick={() => spawnMutation.mutate(createMode === "template" ? { template: templateName } : { manifest_toml: manifestToml })}
                  disabled={spawnMutation.isPending || (createMode === "template" ? !templateName.trim() : !manifestToml.trim())}>
                  {spawnMutation.isPending ? <Loader2 className="w-4 h-4 animate-spin mr-1" /> : <Plus className="w-4 h-4 mr-1" />}
                  {t("agents.create_agent")}
                </Button>
                <Button variant="secondary" onClick={() => setShowCreate(false)}>{t("common.cancel")}</Button>
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
