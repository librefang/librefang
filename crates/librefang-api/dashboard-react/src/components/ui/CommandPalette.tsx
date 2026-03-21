import { useEffect, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { Search, Home, Layers, MessageCircle, Server, Network, Calendar, Shield, BarChart3, FileText, Settings, Bot } from "lucide-react";
import type { LucideIcon } from "lucide-react";

interface CommandItem {
  id: string;
  label: string;
  labelZh: string;
  icon: LucideIcon;
  action: () => void;
  category: string;
}

interface CommandPaletteProps {
  isOpen: boolean;
  onClose: () => void;
}

export function CommandPalette({ isOpen, onClose }: CommandPaletteProps) {
  const navigate = useNavigate();
  const [search, setSearch] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);

  const commands: CommandItem[] = [
    { id: "overview", label: "Overview", labelZh: "概览", category: "Core", icon: Home, action: () => navigate({ to: "/overview" }) },
    { id: "workflows", label: "Workflows", labelZh: "工作流", category: "Core", icon: Layers, action: () => navigate({ to: "/workflows" }) },
    { id: "canvas", label: "Canvas", labelZh: "画布", category: "Core", icon: Layers, action: () => navigate({ to: "/canvas", search: { t: Date.now() } }) },
    { id: "chat", label: "Chat", labelZh: "对话", category: "Core", icon: MessageCircle, action: () => navigate({ to: "/chat", search: { agentId: undefined } }) },
    { id: "agents", label: "Agents", labelZh: "智能体", category: "Resources", icon: Bot, action: () => navigate({ to: "/agents" }) },
    { id: "providers", label: "Providers", labelZh: "供应商", category: "Resources", icon: Server, action: () => navigate({ to: "/providers" }) },
    { id: "channels", label: "Channels", labelZh: "通道", category: "Resources", icon: Network, action: () => navigate({ to: "/channels" }) },
    { id: "scheduler", label: "Scheduler", labelZh: "调度器", category: "Automation", icon: Calendar, action: () => navigate({ to: "/scheduler" }) },
    { id: "goals", label: "Goals", labelZh: "目标", category: "Automation", icon: Shield, action: () => navigate({ to: "/goals" }) },
    { id: "analytics", label: "Analytics", labelZh: "分析", category: "System", icon: BarChart3, action: () => navigate({ to: "/analytics" }) },
    { id: "logs", label: "Logs", labelZh: "日志", category: "System", icon: FileText, action: () => navigate({ to: "/logs" }) },
    { id: "settings", label: "Settings", labelZh: "设置", category: "System", icon: Settings, action: () => navigate({ to: "/settings" }) },
  ];

  const filteredCommands = commands.filter(cmd => {
    const searchLower = search.toLowerCase();
    return cmd.label.toLowerCase().includes(searchLower) || cmd.labelZh.includes(search);
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
    if (!acc[cmd.category]) acc[cmd.category] = [];
    acc[cmd.category].push(cmd);
    return acc;
  }, {} as Record<string, CommandItem[]>);

  return (
    <div className="fixed inset-0 z-[100] flex items-start justify-center pt-[15vh]">
      <div className="fixed inset-0 bg-black/60 backdrop-blur-sm" onClick={onClose} />
      <div className="relative w-full max-w-xl rounded-2xl border border-border-subtle bg-surface shadow-2xl overflow-hidden animate-in zoom-in-95 duration-150">
        <div className="flex items-center gap-3 border-b border-border-subtle px-4 py-4">
          <Search className="h-5 w-5 text-text-dim shrink-0" />
          <input
            type="text"
            value={search}
            onChange={(e) => { setSearch(e.target.value); setSelectedIndex(0); }}
            placeholder="Search commands..."
            className="flex-1 bg-transparent text-sm font-medium outline-none placeholder:text-text-dim"
            autoFocus
          />
          <kbd className="hidden sm:inline-flex h-5 items-center gap-1 rounded border border-border-subtle bg-main px-1.5 text-[10px] font-medium text-text-dim">ESC</kbd>
        </div>
        <div className="max-h-[50vh] overflow-y-auto p-2">
          {filteredCommands.length === 0 ? (
            <p className="py-8 text-center text-sm text-text-dim">No results found</p>
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
                      className={`w-full flex items-center gap-3 px-3 py-2.5 rounded-xl text-left transition-colors ${globalIndex === selectedIndex ? 'bg-brand/10 text-brand' : 'hover:bg-surface-hover'}`}
                    >
                      <cmd.icon className="h-5 w-5" />
                      <span className="flex-1 text-sm font-medium">{cmd.label}</span>
                      <span className="text-xs text-text-dim">{cmd.labelZh}</span>
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
