import { useQuery } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Card } from "../components/ui/Card";
import { Badge } from "../components/ui/Badge";
import { Input } from "../components/ui/Input";
import { PageHeader } from "../components/ui/PageHeader";
import { ListSkeleton } from "../components/ui/Skeleton";
import { Globe, Sun, Moon, Settings, PanelLeftClose, PanelLeft, Wrench, Search, RefreshCw } from "lucide-react";
import { useUIStore } from "../lib/store";

// Tools API (inline since it's small)
async function listTools(): Promise<any[]> {
  const resp = await fetch("/api/tools", { headers: { "Content-Type": "application/json" } });
  if (!resp.ok) return [];
  const data = await resp.json();
  return data.tools ?? data ?? [];
}

// Tool name i18n mapping for Chinese
const toolNameZh: Record<string, string> = {
  file_read: "读取文件", file_write: "写入文件", file_list: "列出文件",
  apply_patch: "应用补丁", web_fetch: "网页抓取", web_search: "网页搜索",
  shell_exec: "执行命令", memory_store: "存储记忆", memory_recall: "回忆记忆",
  schedule_create: "创建调度", schedule_list: "调度列表", schedule_delete: "删除调度",
  knowledge_add_entity: "添加实体", knowledge_add_relation: "添加关系", knowledge_query: "知识查询",
  event_publish: "发布事件", cron_create: "创建定时", cron_delete: "删除定时",
  agent_send: "发送消息", agent_list: "智能体列表",
};

const toolDescZh: Record<string, string> = {
  file_read: "读取指定路径的文件内容", file_write: "将内容写入指定文件", file_list: "列出目录下的文件",
  apply_patch: "应用代码补丁到文件", web_fetch: "抓取网页内容", web_search: "搜索互联网信息",
  shell_exec: "在系统终端执行命令", memory_store: "将信息存储到长期记忆", memory_recall: "从长期记忆中检索信息",
  schedule_create: "创建新的调度任务", schedule_list: "列出所有调度任务", schedule_delete: "删除指定的调度任务",
  knowledge_add_entity: "向知识图谱添加实体", knowledge_add_relation: "向知识图谱添加关系", knowledge_query: "查询知识图谱",
  event_publish: "发布事件到事件总线", cron_create: "创建定时任务", cron_delete: "删除定时任务",
  agent_send: "向智能体发送消息", agent_list: "列出所有智能体",
};

function OptionCard({ active, icon: Icon, label, onClick }: { active: boolean; icon: any; label: string; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      className={`flex flex-1 items-center justify-center gap-2 rounded-xl border py-2.5 text-sm font-bold transition-all duration-300 ${
        active
          ? "border-brand/30 bg-brand/10 text-brand shadow-sm shadow-brand/10"
          : "border-border-subtle bg-surface text-text-dim hover:border-brand/20 hover:text-brand"
      }`}
    >
      <Icon className="h-4 w-4" />
      <span className="truncate">{label}</span>
    </button>
  );
}

export function SettingsPage() {
  const { t, i18n } = useTranslation();
  const theme = useUIStore((s) => s.theme);
  const toggleTheme = useUIStore((s) => s.toggleTheme);
  const language = useUIStore((s) => s.language);
  const setLanguage = useUIStore((s) => s.setLanguage);
  const navLayout = useUIStore((s) => s.navLayout);
  const setNavLayout = useUIStore((s) => s.setNavLayout);
  const isZh = i18n.language === "zh";
  const [toolSearch, setToolSearch] = useState("");

  const toolsQuery = useQuery({ queryKey: ["tools"], queryFn: listTools });
  const tools = toolsQuery.data ?? [];
  const filteredTools = useMemo(() =>
    tools.filter((tool: any) => !toolSearch || (tool.name || "").toLowerCase().includes(toolSearch.toLowerCase()) || (tool.description || "").toLowerCase().includes(toolSearch.toLowerCase())),
    [tools, toolSearch]
  );

  const labelClass = "text-[10px] font-black uppercase tracking-widest text-text-dim mb-2.5 block";

  return (
    <div className="flex flex-col gap-4 sm:gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("settings.system_config")}
        title={t("settings.title")}
        subtitle={t("settings.subtitle")}
        icon={<Settings className="h-4 w-4" />}
        helpText={t("settings.help")}
      />

      <div className="grid gap-4 sm:gap-6 lg:grid-cols-2">
        {/* Appearance Settings */}
        <Card padding="lg" hover className="min-w-0 overflow-hidden">
          <h2 className="text-base sm:text-lg font-black tracking-tight mb-5 sm:mb-6">{t("settings.appearance")}</h2>
          <div className="space-y-5 sm:space-y-8">
            <div>
              <span className={labelClass}>{t("settings.theme")}</span>
              <div className="flex gap-2 sm:gap-3">
                <OptionCard active={theme === "light"} icon={Sun} label={t("settings.theme_light")} onClick={() => theme !== "light" && toggleTheme()} />
                <OptionCard active={theme === "dark"} icon={Moon} label={t("settings.theme_dark")} onClick={() => theme !== "dark" && toggleTheme()} />
              </div>
            </div>
            <div>
              <span className={labelClass}>{t("settings.language")}</span>
              <div className="flex gap-2 sm:gap-3">
                <OptionCard active={language === "en"} icon={Globe} label="English" onClick={() => setLanguage("en")} />
                <OptionCard active={language === "zh"} icon={Globe} label="中文" onClick={() => setLanguage("zh")} />
              </div>
            </div>
            <div>
              <span className={labelClass}>{t("settings.nav_layout")}</span>
              <div className="flex gap-2 sm:gap-3">
                <OptionCard active={navLayout === "grouped"} icon={PanelLeft} label={t("settings.nav_grouped")} onClick={() => setNavLayout("grouped")} />
                <OptionCard active={navLayout === "collapsible"} icon={PanelLeftClose} label={t("settings.nav_collapsible")} onClick={() => setNavLayout("collapsible")} />
              </div>
            </div>
          </div>
        </Card>

        {/* Tools Management */}
        <Card padding="lg" hover className="min-w-0 overflow-hidden">
          <div className="flex items-center justify-between mb-4">
            <h2 className="text-base sm:text-lg font-black tracking-tight flex items-center gap-2">
              <Wrench className="w-4 h-4 sm:w-5 sm:h-5 text-brand" />
              {t("settings.tools_title")}
            </h2>
            <div className="flex items-center gap-2">
              <Badge variant="brand">{tools.length}</Badge>
              <button onClick={() => toolsQuery.refetch()} className="p-1.5 rounded-lg hover:bg-main">
                <RefreshCw className={`w-3.5 h-3.5 text-text-dim ${toolsQuery.isFetching ? "animate-spin" : ""}`} />
              </button>
            </div>
          </div>

          <div className="mb-3">
            <Input value={toolSearch} onChange={(e) => setToolSearch(e.target.value)}
              placeholder={t("settings.tools_search")}
              leftIcon={<Search className="h-3.5 w-3.5" />}
              className="text-xs!" />
          </div>

          <div className="space-y-1 sm:max-h-80 sm:overflow-y-auto scrollbar-thin">
            {toolsQuery.isLoading ? (
              <ListSkeleton rows={3} />
            ) : filteredTools.length === 0 ? (
              <p className="text-xs text-text-dim italic text-center py-4">{t("common.no_data")}</p>
            ) : (
              filteredTools.map((tool: any, i: number) => (
                <div key={tool.name || i} className="flex items-start gap-3 px-3 py-2.5 rounded-xl border border-transparent hover:border-brand/15 hover:bg-gradient-to-r hover:from-brand/5 hover:to-transparent transition-all duration-300 group cursor-default">
                  <div className="w-8 h-8 rounded-xl bg-gradient-to-br from-brand/15 to-brand/5 flex items-center justify-center shrink-0 ring-1 ring-brand/10 group-hover:ring-brand/25 group-hover:shadow-sm group-hover:shadow-brand/10 transition-all duration-300">
                    <Wrench className="w-3.5 h-3.5 text-brand/70 group-hover:text-brand transition-colors duration-300" />
                  </div>
                  <div className="min-w-0 flex-1 pt-0.5">
                    <div className="flex items-center gap-2">
                      <p className="text-[13px] font-bold truncate group-hover:text-brand transition-colors duration-300">{isZh ? (toolNameZh[tool.name] || tool.name) : (tool.name || tool.id)}</p>
                      {tool.source && (
                        <span className="text-[8px] px-1.5 py-[1px] rounded-full bg-brand/8 text-brand/50 font-semibold uppercase tracking-wider shrink-0 border border-brand/10">{tool.source}</span>
                      )}
                    </div>
                    {tool.description && <p className="text-[11px] text-text-dim/80 truncate mt-0.5 leading-relaxed">{isZh ? (toolDescZh[tool.name] || tool.description) : tool.description}</p>}
                  </div>
                </div>
              ))
            )}
          </div>
        </Card>
      </div>
    </div>
  );
}
