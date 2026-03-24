import { useTranslation } from "react-i18next";
import { X, Trash2, Sparkles, ChevronDown, ChevronUp } from "lucide-react";
import { useState } from "react";
import { useUIStore } from "../../lib/store";

export function SkillOutputPanel() {
  const { t } = useTranslation();
  const skillOutputs = useUIStore((s) => s.skillOutputs);
  const dismissSkillOutput = useUIStore((s) => s.dismissSkillOutput);
  const clearSkillOutputs = useUIStore((s) => s.clearSkillOutputs);
  const isSidebarCollapsed = useUIStore((s) => s.isSidebarCollapsed);
  const isMobileMenuOpen = useUIStore((s) => s.isMobileMenuOpen);
  const [collapsed, setCollapsed] = useState(false);

  if (skillOutputs.length === 0) return null;

  return (
    <div className={`fixed bottom-0 left-0 right-0 pointer-events-none ${isMobileMenuOpen ? "z-[30]" : "z-[90]"} ${isSidebarCollapsed ? "lg:left-[72px]" : "lg:left-[280px]"}`}>
      <div className="mx-auto max-w-7xl px-3 sm:px-4 lg:px-8 pb-[env(safe-area-inset-bottom,8px)]">
        <div className="pointer-events-auto rounded-t-xl border border-b-0 border-border-subtle bg-surface/95 backdrop-blur-xl shadow-2xl">
          {/* Header */}
          <div className="flex items-center justify-between px-3 sm:px-4 py-2 border-b border-border-subtle/50">
            <button
              onClick={() => setCollapsed(!collapsed)}
              className="flex items-center gap-2 text-xs font-bold text-brand"
            >
              <Sparkles className="w-3.5 h-3.5" />
              {t("skills.outputs", { defaultValue: "Skill Outputs" })}
              <span className="px-1.5 py-0.5 rounded-full bg-brand/10 text-[10px]">
                {skillOutputs.length}
              </span>
              {collapsed ? <ChevronUp className="w-3 h-3" /> : <ChevronDown className="w-3 h-3" />}
            </button>
            <button
              onClick={clearSkillOutputs}
              className="p-1.5 rounded-lg text-text-dim/40 hover:text-error hover:bg-error/10 transition-colors"
              title={t("common.clear", { defaultValue: "Clear" })}
            >
              <Trash2 className="w-3.5 h-3.5" />
            </button>
          </div>

          {/* Content */}
          {!collapsed && (
            <div className="max-h-48 sm:max-h-64 overflow-y-auto p-2 sm:p-3 space-y-2">
              {skillOutputs.map((output) => (
                <div
                  key={output.id}
                  className="flex items-start gap-2 p-2.5 sm:p-3 rounded-lg bg-main/50 border border-border-subtle/30 group"
                >
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2 mb-1">
                      <span className="text-[10px] font-bold text-brand uppercase tracking-wider">
                        {output.skillName}
                      </span>
                      {output.agentName && (
                        <span className="text-[9px] text-text-dim">
                          via {output.agentName}
                        </span>
                      )}
                      <span className="text-[9px] text-text-dim/40 ml-auto">
                        {new Date(output.timestamp).toLocaleTimeString()}
                      </span>
                    </div>
                    <p className="text-xs text-text-dim leading-relaxed whitespace-pre-wrap break-words">
                      {output.content}
                    </p>
                  </div>
                  <button
                    onClick={() => dismissSkillOutput(output.id)}
                    className="p-1 rounded text-text-dim/20 hover:text-text-dim opacity-0 group-hover:opacity-100 transition-[colors,opacity] shrink-0"
                  >
                    <X className="w-3 h-3" />
                  </button>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
