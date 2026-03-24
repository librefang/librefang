import { useEffect, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { Search, Home, Layers, MessageCircle, Server, Network, Calendar, Shield, BarChart3, FileText, Settings, Bot, Clock, CheckCircle, Database, Activity, Hand, Puzzle, Cpu, Radio } from "lucide-react";
import type { LucideIcon } from "lucide-react";

interface CommandItem {
  id: string;
  label: string;
  labelZh: string;
  icon: LucideIcon;
  action: () => void;
  category: string;
  categoryZh: string;
}

interface CommandPaletteProps {
  isOpen: boolean;
  onClose: () => void;
}

export function CommandPalette({ isOpen, onClose }: CommandPaletteProps) {
  const { t, i18n } = useTranslation();
  const navigate = useNavigate();
  const [search, setSearch] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const isZh = i18n.language === "zh";

  const commands: CommandItem[] = [
    { id: "overview", label: "Overview", labelZh: "概览", category: "Core", categoryZh: "核心", icon: Home, action: () => navigate({ to: "/overview" }) },
    { id: "workflows", label: "Workflows", labelZh: "工作流", category: "Core", categoryZh: "核心", icon: Layers, action: () => navigate({ to: "/workflows" }) },
    { id: "canvas", label: "Canvas", labelZh: "画布", category: "Core", categoryZh: "核心", icon: Layers, action: () => navigate({ to: "/canvas", search: { t: Date.now() } }) },
    { id: "chat", label: "Chat", labelZh: "对话", category: "Core", categoryZh: "核心", icon: MessageCircle, action: () => navigate({ to: "/chat", search: { agentId: undefined } }) },
    { id: "sessions", label: "Sessions", labelZh: "会话", category: "Core", categoryZh: "核心", icon: Clock, action: () => navigate({ to: "/sessions" }) },
    { id: "approvals", label: "Approvals", labelZh: "审批", category: "Core", categoryZh: "核心", icon: CheckCircle, action: () => navigate({ to: "/approvals" }) },
    { id: "scheduler", label: "Scheduler", labelZh: "调度器", category: "Automation", categoryZh: "自动化", icon: Calendar, action: () => navigate({ to: "/scheduler" }) },
    { id: "goals", label: "Goals", labelZh: "目标", category: "Automation", categoryZh: "自动化", icon: Shield, action: () => navigate({ to: "/goals" }) },
    { id: "agents", label: "Agents", labelZh: "智能体", category: "Resources", categoryZh: "资源", icon: Bot, action: () => navigate({ to: "/agents" }) },
    { id: "providers", label: "Providers", labelZh: "供应商", category: "Resources", categoryZh: "资源", icon: Server, action: () => navigate({ to: "/providers" }) },
    { id: "channels", label: "Channels", labelZh: "通道", category: "Resources", categoryZh: "资源", icon: Network, action: () => navigate({ to: "/channels" }) },
    { id: "skills", label: "Skills", labelZh: "技能", category: "Resources", categoryZh: "资源", icon: Shield, action: () => navigate({ to: "/skills" }) },
    { id: "hands", label: "Hands", labelZh: "Hands", category: "Resources", categoryZh: "资源", icon: Hand, action: () => navigate({ to: "/hands" }) },
    { id: "plugins", label: "Plugins", labelZh: "插件", category: "Resources", categoryZh: "资源", icon: Puzzle, action: () => navigate({ to: "/plugins" }) },
    { id: "models", label: "Models", labelZh: "模型", category: "Resources", categoryZh: "资源", icon: Cpu, action: () => navigate({ to: "/models" }) },
    { id: "analytics", label: "Analytics", labelZh: "分析", category: "System", categoryZh: "系统", icon: BarChart3, action: () => navigate({ to: "/analytics" }) },
    { id: "memory", label: "Memory", labelZh: "记忆", category: "System", categoryZh: "系统", icon: Database, action: () => navigate({ to: "/memory" }) },
    { id: "comms", label: "Comms", labelZh: "通信", category: "System", categoryZh: "系统", icon: Radio, action: () => navigate({ to: "/comms" }) },
    { id: "runtime", label: "Runtime", labelZh: "运行时", category: "System", categoryZh: "系统", icon: Activity, action: () => navigate({ to: "/runtime" }) },
    { id: "logs", label: "Logs", labelZh: "日志", category: "System", categoryZh: "系统", icon: FileText, action: () => navigate({ to: "/logs" }) },
    { id: "settings", label: "Settings", labelZh: "设置", category: "System", categoryZh: "系统", icon: Settings, action: () => navigate({ to: "/settings" }) },
  ];

  const filteredCommands = commands.filter(cmd => {
    const q = search.toLowerCase();
    return cmd.label.toLowerCase().includes(q) || cmd.labelZh.includes(search) || cmd.id.includes(q);
  });

  useEffect(() => {
    if (!isOpen) {
      setSearch("");
      setSelectedIndex(0);
    }
  }, [isOpen]);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (!isOpen) return;

      if (e.key === "ArrowDown") {
        e.preventDefault();
        setSelectedIndex(i => Math.min(i + 1, filteredCommands.length - 1));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setSelectedIndex(i => Math.max(i - 1, 0));
      } else if (e.key === "Enter" && filteredCommands[selectedIndex]) {
        e.preventDefault();
        filteredCommands[selectedIndex].action();
        onClose();
      } else if (e.key === "Escape") {
        onClose();
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [isOpen, filteredCommands, selectedIndex, onClose]);

  if (!isOpen) return null;

  const groupedCommands = filteredCommands.reduce((acc, cmd) => {
    const key = isZh ? cmd.categoryZh : cmd.category;
    if (!acc[key]) acc[key] = [];
    acc[key].push(cmd);
    return acc;
  }, {} as Record<string, CommandItem[]>);

  return (
    <div className="fixed inset-0 z-100 flex items-start justify-center pt-[15vh]">
      <div className="fixed inset-0 bg-black/60 backdrop-blur-xl backdrop-saturate-150" onClick={onClose} />
      <div className="relative w-full max-w-xl max-w-[90vw] rounded-2xl border border-border-subtle bg-surface shadow-2xl overflow-hidden animate-fade-in-scale">
        <div className="flex items-center gap-3 border-b border-border-subtle px-4 py-4">
          <Search className="h-5 w-5 text-text-dim shrink-0" />
          <input
            type="text"
            value={search}
            onChange={(e) => { setSearch(e.target.value); setSelectedIndex(0); }}
            placeholder={isZh ? "搜索页面..." : "Search pages..."}
            className="flex-1 bg-transparent text-sm font-medium outline-none placeholder:text-text-dim/40"
            autoFocus
          />
          <kbd className="hidden sm:inline-flex h-5 items-center gap-1 rounded border border-border-subtle bg-main px-1.5 text-[10px] font-medium text-text-dim">ESC</kbd>
        </div>
        <div className="px-4 py-2 border-b border-border-subtle/50 flex items-center gap-4 text-[10px] text-text-dim/50">
          <span className="flex items-center gap-1"><kbd className="px-1 py-0.5 rounded bg-main text-[9px] font-mono">↑↓</kbd> {isZh ? "导航" : "Navigate"}</span>
          <span className="flex items-center gap-1"><kbd className="px-1 py-0.5 rounded bg-main text-[9px] font-mono">↵</kbd> {isZh ? "打开" : "Open"}</span>
          <span className="flex items-center gap-1"><kbd className="px-1 py-0.5 rounded bg-main text-[9px] font-mono">esc</kbd> {isZh ? "关闭" : "Close"}</span>
        </div>
        <div className="max-h-[50vh] overflow-y-auto p-2 scrollbar-thin">
          {filteredCommands.length === 0 ? (
            <p className="py-8 text-center text-sm text-text-dim">{t("common.no_data")}</p>
          ) : (
            Object.entries(groupedCommands).map(([category, cmds]) => (
              <div key={category}>
                <p className="px-3 py-2 text-[10px] font-bold uppercase tracking-widest text-text-dim/60">{category}</p>
                {cmds.map((cmd) => {
                  const globalIndex = filteredCommands.indexOf(cmd);
                  return (
                    <button
                      key={cmd.id}
                      onClick={() => { cmd.action(); onClose(); }}
                      className={`w-full flex items-center gap-3 px-3 py-2.5 rounded-xl text-left transition-all duration-200 ${globalIndex === selectedIndex ? 'bg-brand/10 text-brand' : 'hover:bg-surface-hover'}`}
                    >
                      <cmd.icon className="h-4 w-4 shrink-0" />
                      <span className="flex-1 text-sm font-medium">{isZh ? cmd.labelZh : cmd.label}</span>
                      {!isZh && <span className="text-[10px] text-text-dim/40">{cmd.labelZh}</span>}
                    </button>
                  );
                })}
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  );
}

export function useCommandPalette() {
  const [isOpen, setIsOpen] = useState(false);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "k") {
        e.preventDefault();
        setIsOpen(true);
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);

  return { isOpen, setIsOpen };
}
