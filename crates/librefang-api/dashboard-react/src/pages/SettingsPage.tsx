import { useQuery } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { Input } from "../components/ui/Input";
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

export function SettingsPage() {
  const { t, i18n } = useTranslation();
  const { theme, toggleTheme, language, setLanguage, navLayout, setNavLayout } = useUIStore();
  const isZh = i18n.language === "zh";
  const [toolSearch, setToolSearch] = useState("");

  const toolsQuery = useQuery({ queryKey: ["tools"], queryFn: listTools });
  const tools = toolsQuery.data ?? [];
  const filteredTools = useMemo(() =>
    tools.filter((tool: any) => !toolSearch || (tool.name || "").toLowerCase().includes(toolSearch.toLowerCase()) || (tool.description || "").toLowerCase().includes(toolSearch.toLowerCase())),
    [tools, toolSearch]
  );

  const labelClass = "text-[10px] font-black uppercase tracking-widest text-text-dim mb-2 block";

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <header>
        <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
          <Settings className="h-4 w-4" />
          {t("settings.system_config")}
        </div>
        <h1 className="mt-2 text-3xl font-extrabold tracking-tight">{t("settings.title")}</h1>
        <p className="mt-1 text-text-dim font-medium">{t("settings.subtitle")}</p>
      </header>

      <div className="grid gap-6 lg:grid-cols-2">
        {/* 外观设置 */}
        <Card padding="lg" hover>
          <h2 className="text-lg font-black tracking-tight mb-6">{t("settings.appearance")}</h2>
          <div className="space-y-8">
            <div>
              <span className={labelClass}>{t("settings.theme")}</span>
              <div className="flex gap-3">
                <Button variant={theme === "light" ? "primary" : "secondary"} onClick={() => theme !== "light" && toggleTheme()}>
                  <Sun className="h-4 w-4" /> {t("settings.theme_light")}
                </Button>
                <Button variant={theme === "dark" ? "primary" : "secondary"} onClick={() => theme !== "dark" && toggleTheme()}>
                  <Moon className="h-4 w-4" /> {t("settings.theme_dark")}
                </Button>
              </div>
            </div>
            <div>
              <span className={labelClass}>{t("settings.language")}</span>
              <div className="flex gap-3">
                <Button variant={language === "en" ? "primary" : "secondary"} onClick={() => setLanguage("en")}>
                  <Globe className="h-4 w-4" /> English
                </Button>
                <Button variant={language === "zh" ? "primary" : "secondary"} onClick={() => setLanguage("zh")}>
                  <Globe className="h-4 w-4" /> 中文
                </Button>
              </div>
            </div>
            <div>
              <span className={labelClass}>{t("settings.nav_layout")}</span>
              <div className="flex gap-3">
                <Button variant={navLayout === "grouped" ? "primary" : "secondary"} onClick={() => setNavLayout("grouped")}>
                  <PanelLeft className="h-4 w-4" /> {t("settings.nav_grouped")}
                </Button>
                <Button variant={navLayout === "collapsible" ? "primary" : "secondary"} onClick={() => setNavLayout("collapsible")}>
                  <PanelLeftClose className="h-4 w-4" /> {t("settings.nav_collapsible")}
                </Button>
              </div>
            </div>
          </div>
        </Card>

        {/* Tools 管理 */}
        <Card padding="lg" hover>
          <div className="flex items-center justify-between mb-4">
            <h2 className="text-lg font-black tracking-tight flex items-center gap-2">
              <Wrench className="w-5 h-5 text-brand" />
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
              className="!text-xs" />
          </div>

          <div className="space-y-1.5 max-h-80 overflow-y-auto">
            {toolsQuery.isLoading ? (
              <div className="space-y-2">
                {[1, 2, 3].map(i => <div key={i} className="h-10 rounded-lg bg-main animate-pulse" />)}
              </div>
            ) : filteredTools.length === 0 ? (
              <p className="text-xs text-text-dim italic text-center py-4">{t("common.no_data")}</p>
            ) : (
              filteredTools.map((tool: any, i: number) => (
                <div key={tool.name || i} className="flex items-center gap-2.5 px-3 py-2 rounded-lg hover:bg-main transition-colors">
                  <div className="w-6 h-6 rounded-md bg-brand/10 flex items-center justify-center shrink-0">
                    <Wrench className="w-3 h-3 text-brand" />
                  </div>
                  <div className="min-w-0 flex-1">
                    <p className="text-xs font-bold truncate">{isZh ? (toolNameZh[tool.name] || tool.name) : (tool.name || tool.id)}</p>
                    {tool.description && <p className="text-[9px] text-text-dim truncate">{isZh ? (toolDescZh[tool.name] || tool.description) : tool.description}</p>}
                  </div>
                  {tool.source && <span className="text-[8px] px-1.5 py-0.5 rounded bg-main text-text-dim/60 shrink-0">{tool.source}</span>}
                </div>
              ))
            )}
          </div>
        </Card>
      </div>
    </div>
  );
}
