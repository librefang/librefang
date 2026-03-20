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
import { Search, Users, Settings, MessageCircle, X, Cpu, Wrench, Shield, Plus, Loader2, Pause, Play } from "lucide-react";

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
  const [detailLoading, setDetailLoading] = useState(false);
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
      // 1. Suspended last
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

  // 分组：core agents 和 hands
  const coreAgents = filteredAgents.filter(a => !a.name.includes("-hand"));
  const handAgents = filteredAgents.filter(a => a.name.includes("-hand"));

  const renderAgentCard = (agent: any) => {
    const isSuspended = (agent.state || "").toLowerCase() === "suspended";
    const isHand = agent.name.includes("-hand");
    return (
      <Card key={agent.id} hover padding="lg" className={`cursor-pointer ${isSuspended ? "opacity-60" : ""}`} onClick={async () => {
        setDetailLoading(true);
        try { const d = await getAgentDetail(agent.id); setDetailAgent(d); } catch { setDetailAgent({ name: agent.name, id: agent.id }); }
        setDetailLoading(false);
      }}>
        <div className="flex items-start justify-between gap-4 mb-6">
          <div className="flex items-center gap-4 min-w-0">
            <Avatar fallback={agent.name} size="lg" />
            <div className="min-w-0">
              <h2 className="text-lg font-black tracking-tight truncate">{agent.name}</h2>
              <div className="flex items-center gap-2">
                <p className="text-[10px] font-mono text-text-dim/60 truncate">{agent.id.slice(0, 8)}</p>
                {isHand && <span className="text-[8px] px-1.5 py-0.5 rounded-full bg-purple-100 text-purple-600 dark:bg-purple-900/30 dark:text-purple-400 font-bold">HAND</span>}
              </div>
            </div>
          </div>
          <Badge variant={getStatusVariant(agent.state)}>
            {agent.state ? t(`common.${agent.state.toLowerCase()}`, { defaultValue: agent.state }) : t("common.idle")}
          </Badge>
        </div>
        <div className="space-y-3 mb-6">
          <div className="flex justify-between items-center text-xs">
            <span className="text-text-dim font-bold uppercase tracking-wider text-[10px]">{t("agents.model")}</span>
            <span className="font-black text-slate-700 dark:text-slate-300">{agent.model_name || t("common.unknown")}</span>
          </div>
          <div className="flex justify-between items-center text-xs">
            <span className="text-text-dim font-bold uppercase tracking-wider text-[10px]">{t("agents.provider")}</span>
            <span className="font-black text-brand">{agent.model_provider || t("common.local")}</span>
          </div>
          <div className="flex justify-between items-center text-xs">
            <span className="text-text-dim font-bold uppercase tracking-wider text-[10px]">{t("agents.last_active")}</span>
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
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <div className="flex justify-between items-end">
        <PageHeader
          badge={t("common.kernel_runtime")}
          title={t("agents.title")}
          subtitle={t("agents.subtitle")}
          isFetching={agentsQuery.isFetching}
          onRefresh={() => void agentsQuery.refetch()}
          icon={<Users className="h-4 w-4" />}
        />
        <Button variant="primary" onClick={() => setShowCreate(true)}>
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
              <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
                {coreAgents.map(agent => renderAgentCard(agent))}
              </div>
            </div>
          )}
          {/* Hands */}
          {handAgents.length > 0 && (
            <div>
              <h3 className="text-[10px] font-bold uppercase tracking-widest text-text-dim/50 mb-3">{t("agents.hands")} ({handAgents.length})</h3>
              <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
                {handAgents.map(agent => renderAgentCard(agent))}
              </div>
            </div>
          )}
        </div>
      )}
      {/* Agent Detail Modal */}
      {detailAgent && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 backdrop-blur-sm" onClick={() => setDetailAgent(null)}>
          <div className="bg-surface rounded-2xl shadow-2xl border border-border-subtle w-[560px] max-h-[80vh] overflow-y-auto" onClick={e => e.stopPropagation()}>
            <div className="flex items-center justify-between px-5 py-3 border-b border-border-subtle sticky top-0 bg-surface z-10">
              <div className="flex items-center gap-3">
                <Avatar fallback={detailAgent.name} size="lg" />
                <div>
                  <h3 className="text-sm font-bold">{detailAgent.name}</h3>
                  <p className="text-[9px] text-text-dim font-mono">{detailAgent.id}</p>
                </div>
              </div>
              <button onClick={() => setDetailAgent(null)} className="p-1 rounded hover:bg-main"><X className="w-4 h-4" /></button>
            </div>
            <div className="p-5 space-y-4">
              {/* Model */}
              {detailAgent.model && (
                <div>
                  <h4 className="text-[10px] font-bold text-text-dim uppercase mb-2 flex items-center gap-1"><Cpu className="w-3 h-3" /> {t("agents.model")}</h4>
                  <div className="p-3 rounded-xl bg-main space-y-1 text-xs">
                    <div className="flex justify-between"><span className="text-text-dim">Provider</span><span className="font-bold">{detailAgent.model.provider}</span></div>
                    <div className="flex justify-between"><span className="text-text-dim">Model</span><span className="font-bold">{detailAgent.model.model}</span></div>
                  </div>
                </div>
              )}

              {/* System Prompt */}
              {detailAgent.system_prompt && (
                <div>
                  <h4 className="text-[10px] font-bold text-text-dim uppercase mb-2">{t("agents.system_prompt")}</h4>
                  <pre className="p-3 rounded-xl bg-main text-xs text-text-dim whitespace-pre-wrap max-h-32 overflow-y-auto">{detailAgent.system_prompt}</pre>
                </div>
              )}

              {/* Capabilities */}
              {detailAgent.capabilities && (
                <div>
                  <h4 className="text-[10px] font-bold text-text-dim uppercase mb-2 flex items-center gap-1"><Wrench className="w-3 h-3" /> {t("agents.capabilities")}</h4>
                  <div className="flex flex-wrap gap-1.5">
                    {detailAgent.capabilities.tools && <Badge variant="brand">Tools</Badge>}
                    {detailAgent.capabilities.network && <Badge variant="brand">Network</Badge>}
                  </div>
                </div>
              )}

              {/* Skills */}
              {detailAgent.skills && detailAgent.skills.length > 0 && (
                <div>
                  <h4 className="text-[10px] font-bold text-text-dim uppercase mb-2">{t("agents.skills")}</h4>
                  <div className="flex flex-wrap gap-1.5">
                    {detailAgent.skills.map((s: string, i: number) => (
                      <Badge key={i} variant="default">{s}</Badge>
                    ))}
                  </div>
                </div>
              )}

              {/* Tags */}
              {detailAgent.tags && detailAgent.tags.length > 0 && (
                <div>
                  <h4 className="text-[10px] font-bold text-text-dim uppercase mb-2">{t("agents.tags")}</h4>
                  <div className="flex flex-wrap gap-1.5">
                    {detailAgent.tags.map((tag: string, i: number) => (
                      <span key={i} className="text-[10px] px-2 py-0.5 rounded-full bg-main text-text-dim">{tag}</span>
                    ))}
                  </div>
                </div>
              )}

              {/* Mode */}
              {detailAgent.mode && (
                <div className="flex items-center gap-2">
                  <Shield className="w-3.5 h-3.5 text-text-dim" />
                  <span className="text-xs font-bold">{t("agents.mode")}:</span>
                  <Badge variant="default">{detailAgent.mode}</Badge>
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
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 backdrop-blur-sm" onClick={() => setShowCreate(false)}>
          <div className="bg-surface rounded-2xl shadow-2xl border border-border-subtle w-[480px]" onClick={e => e.stopPropagation()}>
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
                    placeholder={'[agent]\nname = "my-agent"\n\n[model]\nprovider = "openai"\nmodel = "gpt-4o"'}
                    rows={10}
                    className="mt-1 w-full rounded-xl border border-border-subtle bg-main px-3 py-2 text-xs font-mono outline-none focus:border-brand resize-none" />
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
