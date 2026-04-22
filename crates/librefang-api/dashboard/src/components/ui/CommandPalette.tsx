import { useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { Search, Home, Layers, MessageCircle, Server, Network, Calendar, Shield, BarChart3, FileText, Settings, Bot, Clock, CheckCircle, Database, Activity, Hand, Puzzle, Cpu, Radio, Terminal, ExternalLink } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { useFocusTrap } from "../../lib/useFocusTrap";

interface CommandItem {
  id: string;
  labelKey: string;
  icon: LucideIcon;
  action: () => void;
  categoryKey: string;
  external?: boolean;
}

// Public librefang.ai registry catalog. Keys match the registry categories
// in librefang-registry, labels are i18n keys already in the dashboard.
const REGISTRY_ITEMS: { slug: string; labelKey: string; icon: LucideIcon }[] = [
  { slug: "skills",    labelKey: "nav.skills",    icon: Shield },
  { slug: "hands",     labelKey: "nav.hands",     icon: Hand },
  { slug: "agents",    labelKey: "nav.agents",    icon: Bot },
  { slug: "providers", labelKey: "nav.providers", icon: Server },
  { slug: "workflows", labelKey: "nav.workflows", icon: Layers },
  { slug: "channels",  labelKey: "nav.channels",  icon: Network },
  { slug: "plugins",   labelKey: "nav.plugins",   icon: Puzzle },
  { slug: "mcp",       labelKey: "nav.mcp_servers", icon: Cpu },
  // `models` intentionally omitted — librefang.ai has no /models route.
];

interface CommandPaletteProps {
  isOpen: boolean;
  onClose: () => void;
}

export function CommandPalette({ isOpen, onClose }: CommandPaletteProps) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [search, setSearch] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const dialogRef = useRef<HTMLDivElement>(null);
  useFocusTrap(isOpen, dialogRef, { setAriaModal: true });

  const commands = useMemo<CommandItem[]>(() => [
    { id: "overview", labelKey: "nav.overview", categoryKey: "nav.core", icon: Home, action: () => navigate({ to: "/overview" }) },
    { id: "workflows", labelKey: "nav.workflows", categoryKey: "nav.core", icon: Layers, action: () => navigate({ to: "/workflows" }) },
    { id: "canvas", labelKey: "nav.canvas", categoryKey: "nav.core", icon: Layers, action: () => navigate({ to: "/canvas", search: { t: Date.now(), wf: undefined } }) },
    { id: "chat", labelKey: "nav.chat", categoryKey: "nav.core", icon: MessageCircle, action: () => navigate({ to: "/chat", search: { agentId: undefined } }) },
    { id: "sessions", labelKey: "nav.sessions", categoryKey: "nav.core", icon: Clock, action: () => navigate({ to: "/sessions" }) },
    { id: "approvals", labelKey: "nav.approvals", categoryKey: "nav.core", icon: CheckCircle, action: () => navigate({ to: "/approvals" }) },
    { id: "scheduler", labelKey: "nav.scheduler", categoryKey: "nav.automation", icon: Calendar, action: () => navigate({ to: "/scheduler" }) },
    { id: "goals", labelKey: "nav.goals", categoryKey: "nav.automation", icon: Shield, action: () => navigate({ to: "/goals" }) },
    { id: "agents", labelKey: "nav.agents", categoryKey: "nav.resources", icon: Bot, action: () => navigate({ to: "/agents" }) },
    { id: "providers", labelKey: "nav.providers", categoryKey: "nav.resources", icon: Server, action: () => navigate({ to: "/providers" }) },
    { id: "channels", labelKey: "nav.channels", categoryKey: "nav.resources", icon: Network, action: () => navigate({ to: "/channels" }) },
    { id: "skills", labelKey: "nav.skills", categoryKey: "nav.resources", icon: Shield, action: () => navigate({ to: "/skills" }) },
    { id: "hands", labelKey: "nav.hands", categoryKey: "nav.resources", icon: Hand, action: () => navigate({ to: "/hands" }) },
    { id: "plugins", labelKey: "nav.plugins", categoryKey: "nav.resources", icon: Puzzle, action: () => navigate({ to: "/plugins" }) },
    { id: "models", labelKey: "nav.models", categoryKey: "nav.resources", icon: Cpu, action: () => navigate({ to: "/models" }) },
    { id: "analytics", labelKey: "nav.analytics", categoryKey: "nav.system", icon: BarChart3, action: () => navigate({ to: "/analytics" }) },
    { id: "memory", labelKey: "nav.memory", categoryKey: "nav.system", icon: Database, action: () => navigate({ to: "/memory" }) },
    { id: "comms", labelKey: "nav.comms", categoryKey: "nav.system", icon: Radio, action: () => navigate({ to: "/comms" }) },
    { id: "runtime", labelKey: "nav.runtime", categoryKey: "nav.system", icon: Activity, action: () => navigate({ to: "/runtime" }) },
    { id: "logs", labelKey: "nav.logs", categoryKey: "nav.system", icon: FileText, action: () => navigate({ to: "/logs" }) },
    { id: "settings", labelKey: "nav.settings", categoryKey: "nav.system", icon: Settings, action: () => navigate({ to: "/settings" }) },
    { id: "config-general", labelKey: "config.cat_general", categoryKey: "nav.config", icon: Settings, action: () => navigate({ to: "/config/general" }) },
    { id: "config-memory", labelKey: "config.cat_memory", categoryKey: "nav.config", icon: Database, action: () => navigate({ to: "/config/memory" }) },
    { id: "config-tools", labelKey: "config.cat_tools", categoryKey: "nav.config", icon: Settings, action: () => navigate({ to: "/config/tools" }) },
    { id: "config-channels", labelKey: "config.cat_channels", categoryKey: "nav.config", icon: Network, action: () => navigate({ to: "/config/channels" }) },
    { id: "config-security", labelKey: "config.cat_security", categoryKey: "nav.config", icon: Shield, action: () => navigate({ to: "/config/security" }) },
    { id: "config-network", labelKey: "config.cat_network", categoryKey: "nav.config", icon: Network, action: () => navigate({ to: "/config/network" }) },
    { id: "config-infra", labelKey: "config.cat_infra", categoryKey: "nav.config", icon: Server, action: () => navigate({ to: "/config/infra" }) },
    { id: "terminal", labelKey: "nav.terminal", categoryKey: "nav.advanced", icon: Terminal, action: () => navigate({ to: "/terminal" }) },
    // Registry browse shortcuts — open the public librefang.ai catalog in a
    // new tab so users can discover community skills / hands / templates
    // without leaving the keyboard.
    ...REGISTRY_ITEMS.map(({ slug, labelKey, icon }): CommandItem => ({
      id: `registry-${slug}`,
      labelKey,
      categoryKey: "command_palette.registry",
      icon,
      external: true,
      action: () => window.open(`https://librefang.ai/${slug}`, "_blank", "noopener,noreferrer"),
    })),
  ], [navigate]);

  const filteredCommands = useMemo(() => commands.filter(cmd => {
    const q = search.toLowerCase();
    const label = t(cmd.labelKey).toLowerCase();
    return label.includes(q) || cmd.id.includes(q);
  }), [commands, search, t]);

  const filteredRef = useRef(filteredCommands);
  filteredRef.current = filteredCommands;
  const selectedRef = useRef(selectedIndex);
  selectedRef.current = selectedIndex;

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
        setSelectedIndex(i => Math.min(i + 1, filteredRef.current.length - 1));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setSelectedIndex(i => Math.max(i - 1, 0));
      } else if (e.key === "Enter" && filteredRef.current[selectedRef.current]) {
        e.preventDefault();
        filteredRef.current[selectedRef.current].action();
        onClose();
      } else if (e.key === "Escape") {
        onClose();
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [isOpen, onClose]);

  if (!isOpen) return null;

  const groupedCommands = filteredCommands.reduce((acc, cmd) => {
    const key = t(cmd.categoryKey);
    if (!acc[key]) acc[key] = [];
    acc[key].push(cmd);
    return acc;
  }, {} as Record<string, CommandItem[]>);

  return (
    <div className="fixed inset-0 z-100 flex items-start justify-center pt-[15vh]">
      <div className="fixed inset-0 bg-black/60 backdrop-blur-sm" onClick={onClose} />
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-label={t("command_palette.search_placeholder")}
        className="relative w-full max-w-xl max-w-[90vw] rounded-2xl border border-border-subtle bg-surface shadow-2xl overflow-hidden animate-fade-in-scale"
      >
        <div className="flex items-center gap-3 border-b border-border-subtle px-4 py-4">
          <Search className="h-5 w-5 text-text-dim shrink-0" />
          <input
            type="text"
            value={search}
            onChange={(e) => { setSearch(e.target.value); setSelectedIndex(0); }}
            placeholder={t("command_palette.search_placeholder")}
            className="flex-1 bg-transparent text-sm font-medium outline-none placeholder:text-text-dim/40"
            autoFocus
          />
          <kbd className="hidden sm:inline-flex h-5 items-center gap-1 rounded border border-border-subtle bg-main px-1.5 text-[10px] font-medium text-text-dim">ESC</kbd>
        </div>
        <div className="px-4 py-2 border-b border-border-subtle/50 flex items-center gap-4 text-[10px] text-text-dim/50">
          <span className="flex items-center gap-1"><kbd className="px-1 py-0.5 rounded bg-main text-[9px] font-mono">↑↓</kbd> {t("command_palette.navigate")}</span>
          <span className="flex items-center gap-1"><kbd className="px-1 py-0.5 rounded bg-main text-[9px] font-mono">↵</kbd> {t("command_palette.open")}</span>
          <span className="flex items-center gap-1"><kbd className="px-1 py-0.5 rounded bg-main text-[9px] font-mono">esc</kbd> {t("command_palette.close")}</span>
          <span className="hidden sm:flex items-center gap-1 ml-auto"><kbd className="px-1 py-0.5 rounded bg-main text-[9px] font-mono">?</kbd> {t("command_palette.all_shortcuts", { defaultValue: "all shortcuts" })}</span>
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
                      className={`w-full flex items-center gap-3 px-3 py-2.5 rounded-xl text-left transition-colors duration-200 ${globalIndex === selectedIndex ? 'bg-brand/10 text-brand' : 'hover:bg-surface-hover'}`}
                    >
                      <cmd.icon className="h-4 w-4 shrink-0" />
                      <span className="flex-1 text-sm font-medium">{t(cmd.labelKey)}</span>
                      {cmd.external && <ExternalLink className="h-3 w-3 shrink-0 text-text-dim/60" />}
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
