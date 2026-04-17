import { useQuery, useQueryClient } from "@tanstack/react-query";
import { formatDate } from "../lib/datetime";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  getSkillDetail,
  createSkill,
  reloadSkills,
  evolveUpdateSkill,
  evolvePatchSkill,
  evolveRollbackSkill,
  evolveDeleteSkill,
  evolveWriteFile,
  evolveRemoveFile,
  getSupportingFile,
  type ClawHubBrowseItem,
  type FangHubSkill,
  type HandDefinitionItem,
} from "../api";
import { useSkills, skillQueries } from "../lib/queries/skills";
import { skillKeys } from "../lib/queries/keys";
import { useHands } from "../lib/queries/hands";
import { useUninstallSkill, useClawHubInstall, useSkillHubInstall, useInstallSkill } from "../lib/mutations/skills";
import { CardSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { Input } from "../components/ui/Input";
import { Modal } from "../components/ui/Modal";
import { useUIStore } from "../lib/store";
import {
  Wrench, Search, CheckCircle2, X,
  Download, Trash2, Star, Loader2, Sparkles, Package,
  Code, GitBranch, Globe, Cloud, Monitor, Bot, Database,
  Briefcase, Shield, Terminal, Calendar, Store, Zap, RefreshCw,
  Plus, History, Eye, RotateCcw, FileText, Tag, Edit as EditIcon, Upload,
} from "lucide-react";

type ClawHubSkillWithStatus = ClawHubBrowseItem & { is_installed?: boolean };

type ViewMode = "installed" | "marketplace" | "skillhub" | "fanghub";
type MarketplaceSource = "clawhub" | "skillhub";

// Timezone-based routing: CN users see SkillHub, others see ClawHub
const CN_TIMEZONES = new Set([
  "Asia/Shanghai", "Asia/Chongqing", "Asia/Harbin",
  "Asia/Urumqi", "Asia/Kashgar",
]);
const USE_SKILLHUB = (() => {
  try { return CN_TIMEZONES.has(Intl.DateTimeFormat().resolvedOptions().timeZone); }
  catch { return false; }
})();

// Categories with icons and search keywords
const categories = [
  { id: "coding", nameKey: "skills.cat_coding", icon: Code, keyword: "python javascript code" },
  { id: "git", nameKey: "skills.cat_git", icon: GitBranch, keyword: "git github" },
  { id: "web", nameKey: "skills.cat_web", icon: Globe, keyword: "web frontend html css" },
  { id: "devops", nameKey: "skills.cat_devops", icon: Cloud, keyword: "devops cloud aws docker kubernetes" },
  { id: "browser", nameKey: "skills.cat_browser", icon: Monitor, keyword: "browser automation" },
  { id: "ai", nameKey: "skills.cat_ai", icon: Bot, keyword: "ai llm gpt openai" },
  { id: "data", nameKey: "skills.cat_data", icon: Database, keyword: "data analytics python" },
  { id: "productivity", nameKey: "skills.cat_productivity", icon: Briefcase, keyword: "productivity" },
  { id: "security", nameKey: "skills.cat_security", icon: Shield, keyword: "security" },
  { id: "cli", nameKey: "skills.cat_cli", icon: Terminal, keyword: "cli bash shell" },
];

function getCategoryIcon(category: string) {
  const icons: Record<string, React.ReactNode> = {
    coding: <Code className="w-4 h-4" />,
    git: <GitBranch className="w-4 h-4" />,
    web: <Globe className="w-4 h-4" />,
    devops: <Cloud className="w-4 h-4" />,
    browser: <Monitor className="w-4 h-4" />,
    ai: <Bot className="w-4 h-4" />,
    data: <Database className="w-4 h-4" />,
    productivity: <Briefcase className="w-4 h-4" />,
    security: <Shield className="w-4 h-4" />,
    cli: <Terminal className="w-4 h-4" />,
  };
  return icons[category] || <Sparkles className="w-4 h-4" />;
}

// Skill Card - FangHub registry skill
function FangHubSkillCard({ skill, pendingId, onInstall, t }: {
  skill: FangHubSkill;
  pendingId: string | null;
  onInstall: (name: string) => void;
  t: (key: string) => string;
}) {
  const isPending = pendingId === skill.name;
  return (
    <Card hover padding="none" className="flex flex-col overflow-hidden">
      <div className="h-1.5 bg-gradient-to-r from-brand via-brand/60 to-brand/30" />
      <div className="p-5 flex-1 flex flex-col">
        <div className="flex items-start justify-between gap-3 mb-3">
          <div className="flex items-center gap-3 min-w-0">
            <div className="w-10 h-10 rounded-lg flex items-center justify-center bg-gradient-to-br from-brand/10 to-brand/5 border border-brand/20">
              <Zap className="w-5 h-5 text-brand" />
            </div>
            <div className="min-w-0">
              <h3 className="font-bold text-sm truncate">{skill.name}</h3>
              {skill.version && (
                <span className="text-[10px] text-text-dim font-mono">{skill.version}</span>
              )}
            </div>
          </div>
          {skill.is_installed ? (
            <Badge variant="success"><CheckCircle2 className="w-3 h-3 mr-1" />{t("skills.installed")}</Badge>
          ) : (
            <Button variant="primary" size="sm" onClick={() => onInstall(skill.name)} disabled={!!pendingId}>
              {isPending ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Download className="w-3.5 h-3.5" />}
            </Button>
          )}
        </div>
        {skill.description && (
          <p className="text-xs text-text-dim line-clamp-2 mb-3">{skill.description}</p>
        )}
        {skill.tags && skill.tags.length > 0 && (
          <div className="flex flex-wrap gap-1 mt-auto">
            {skill.tags.slice(0, 3).map(tag => (
              <span key={tag} className="text-[10px] px-1.5 py-0.5 rounded-full bg-brand/8 text-brand/70 font-medium">{tag}</span>
            ))}
          </div>
        )}
      </div>
    </Card>
  );
}

// Skill Card - Installed
function InstalledSkillCard({ skill, onUninstall, onViewDetail, t }: {
  skill: { name: string; version?: string; description?: string; author?: string; tools_count?: number; tags?: string[] };
  onUninstall: (name: string) => void;
  onViewDetail: (name: string) => void;
  t: (key: string) => string;
}) {
  return (
    <Card hover padding="none" className="flex flex-col overflow-hidden group">
      <div className="h-1.5 bg-gradient-to-r from-success via-success/60 to-success/30" />
      <div className="p-5 flex-1 flex flex-col">
        <div className="flex items-start justify-between gap-3 mb-4">
          <div className="flex items-center gap-3 min-w-0">
            <div className="w-10 h-10 rounded-lg flex items-center justify-center text-xl bg-gradient-to-br from-success/10 to-success/5 border border-success/20">
              <Wrench className="w-5 h-5 text-success" />
            </div>
            <div className="min-w-0">
              <h2 className="text-base font-black truncate group-hover:text-success transition-colors">{skill.name}</h2>
              <p className="text-[10px] font-black uppercase tracking-widest text-text-dim/60 truncate">v{skill.version || "1.0.0"}</p>
            </div>
          </div>
          <Badge variant="success">{t("skills.installed")}</Badge>
        </div>
        <p className="text-xs text-text-dim line-clamp-2 italic mb-4 flex-1">{skill.description || "-"}</p>
        <div className="flex justify-between items-center text-[10px] font-bold text-text-dim uppercase mb-3">
          <span>{t("skills.author")}: {skill.author || t("common.unknown")}</span>
          <span>{t("skills.tools")}: {skill.tools_count || 0}</span>
        </div>
        {skill.tags && skill.tags.length > 0 && (
          <div className="flex flex-wrap gap-1 mb-3">
            {skill.tags.slice(0, 3).map(tag => (
              <span key={tag} className="px-1.5 py-0.5 text-[10px] rounded bg-surface-2 text-text-dim">{tag}</span>
            ))}
          </div>
        )}
        <div className="flex gap-2">
          <Button variant="ghost" className="flex-1" onClick={() => onViewDetail(skill.name)} leftIcon={<Eye className="w-4 h-4" />}>
            {t("common.detail")}
          </Button>
          <Button variant="ghost" className="flex-1 text-error hover:text-error" onClick={() => onUninstall(skill.name)} leftIcon={<Trash2 className="w-4 h-4" />}>
            {t("skills.uninstall")}
          </Button>
        </div>
      </div>
    </Card>
  );
}

// Create Skill Modal
function CreateSkillModal({ isOpen, onClose, onCreated, t }: {
  isOpen: boolean;
  onClose: () => void;
  onCreated: () => void;
  t: (key: string, opts?: any) => string;
}) {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [promptContext, setPromptContext] = useState("");
  const [tags, setTags] = useState("");
  const [error, setError] = useState("");
  const [creating, setCreating] = useState(false);

  // Track mounted state to prevent state updates after unmount
  const mountedRef = useRef(true);
  const abortControllerRef = useRef<AbortController | null>(null);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      // Cancel any in-flight request when component unmounts
      abortControllerRef.current?.abort();
    };
  }, []);

  // Reset form state when modal closes
  useEffect(() => {
    if (!isOpen) {
      setError("");
      setCreating(false);
    }
  }, [isOpen]);

  /// Map common API error messages to user-friendly localized strings
  const formatApiError = useCallback((e: any): string => {
    const msg = (e?.message || "").toLowerCase();
    if (msg.includes("already installed") || msg.includes("already exists")) {
      return t("skills.err_name_conflict", { defaultValue: "A skill with this name already exists. Please choose a different name." });
    }
    if (msg.includes("description too long")) {
      return t("skills.err_desc_too_long", { defaultValue: "Description is too long (max 1024 characters)." });
    }
    if (msg.includes("prompt context too large")) {
      return t("skills.err_prompt_too_large", { defaultValue: "Prompt context is too large (max 160,000 characters)." });
    }
    if (msg.includes("security") || msg.includes("blocked")) {
      return t("skills.err_security_blocked", { defaultValue: "Content was blocked by security scan. Please remove potentially dangerous patterns." });
    }
    if (msg.includes("invalid") && msg.includes("name")) {
      return t("skills.err_invalid_name", { defaultValue: "Invalid skill name. Use lowercase letters, numbers, hyphens, and underscores only." });
    }
    return e?.message || t("skills.err_create_failed", { defaultValue: "Failed to create skill. Please try again." });
  }, [t]);

  const handleCreate = async () => {
    setError("");
    if (!name.trim() || !description.trim()) {
      setError(t("skills.evo_fill_required", { defaultValue: "Name and description are required" }));
      return;
    }

    // Abort any previous in-flight request
    abortControllerRef.current?.abort();
    const controller = new AbortController();
    abortControllerRef.current = controller;

    setCreating(true);
    try {
      await createSkill({
        name: name.trim(),
        description: description.trim(),
        prompt_context: promptContext.trim(),
        tags: tags.split(",").map(t => t.trim()).filter(Boolean),
      });
      // Only update state if still mounted and not aborted
      if (mountedRef.current && !controller.signal.aborted) {
        onCreated();
        onClose();
        setName(""); setDescription(""); setPromptContext(""); setTags("");
      }
    } catch (e: any) {
      // Ignore abort errors
      if (e?.name === "AbortError") return;
      if (mountedRef.current && !controller.signal.aborted) {
        setError(formatApiError(e));
      }
    } finally {
      if (mountedRef.current && !controller.signal.aborted) {
        setCreating(false);
      }
    }
  };

  return (
    <Modal isOpen={isOpen} onClose={onClose} title={t("skills.evo_create_title", { defaultValue: "Create Skill" })} size="xl">
      <div className="space-y-4 p-1">
        <div>
          <label className="block text-xs font-bold uppercase text-text-dim mb-1">{t("common.name")}</label>
          <Input value={name} onChange={e => setName(e.target.value)} placeholder="my-skill-name" />
          <p className="text-[10px] text-text-dim mt-1">{t("skills.evo_name_hint", { defaultValue: "Lowercase, hyphens allowed (e.g., csv-analysis)" })}</p>
        </div>
        <div>
          <label className="block text-xs font-bold uppercase text-text-dim mb-1">{t("common.description")}</label>
          <Input value={description} onChange={e => setDescription(e.target.value)} placeholder={t("skills.evo_desc_placeholder", { defaultValue: "What this skill teaches agents to do" })} />
        </div>
        <div>
          <label className="block text-xs font-bold uppercase text-text-dim mb-1">{t("skills.evo_prompt_context", { defaultValue: "Prompt Context (Markdown)" })}</label>
          <textarea
            value={promptContext}
            onChange={e => setPromptContext(e.target.value)}
            className="w-full h-48 px-3 py-2 text-sm rounded-lg bg-surface-2 border border-border text-text-main resize-y font-mono"
            placeholder={t("skills.evo_prompt_placeholder", { defaultValue: "# Skill Instructions\n\nMarkdown instructions injected into the system prompt..." })}
          />
          <p className="text-[10px] text-text-dim mt-1">{promptContext.length.toLocaleString()} / 160,000</p>
        </div>
        <div>
          <label className="block text-xs font-bold uppercase text-text-dim mb-1">{t("skills.evo_tags", { defaultValue: "Tags (comma-separated)" })}</label>
          <Input value={tags} onChange={e => setTags(e.target.value)} placeholder="data, csv, analysis" />
        </div>
        {error && <p className="text-xs text-error">{error}</p>}
        <div className="flex justify-end gap-2 pt-2">
          <Button variant="ghost" onClick={onClose}>{t("common.cancel")}</Button>
          <Button onClick={handleCreate} disabled={creating} leftIcon={creating ? <Loader2 className="w-4 h-4 animate-spin" /> : <Plus className="w-4 h-4" />}>
            {creating ? t("common.creating", { defaultValue: "Creating..." }) : t("common.create")}
          </Button>
        </div>
      </div>
    </Modal>
  );
}

// Skill Detail Modal with evolution history
// ── Evolve sub-panes (embedded inside SkillDetailModal) ─────────────
//
// These are thin, stateful forms. They don't own mutation state — the
// parent (SkillDetailModal) does, so sub-panes simply call onSubmit
// with the collected params and let the parent handle the API call,
// refetch, toasts, and error display.

function EvolveUpdatePane({ skillName, initialContent, onSubmit, onCancel, busy, t }: {
  skillName: string;
  initialContent: string;
  onSubmit: (params: { prompt_context: string; changelog: string }) => void;
  onCancel: () => void;
  busy: boolean;
  t: (key: string, opts?: any) => string;
}) {
  const [content, setContent] = useState(initialContent);
  const [changelog, setChangelog] = useState("");
  const dirty = content !== initialContent;
  return (
    <div className="rounded-lg border border-border bg-surface-1 p-3 space-y-2">
      <p className="text-xs font-bold uppercase text-text-dim">
        {t("skills.evo_update_title", { defaultValue: "Update {{name}}", name: skillName })}
      </p>
      <textarea
        value={content}
        onChange={(e) => setContent(e.target.value)}
        className="w-full h-64 px-3 py-2 text-sm rounded-lg bg-surface-2 border border-border text-text-main resize-y font-mono"
      />
      <p className="text-[10px] text-text-dim">{content.length.toLocaleString()} / 160,000</p>
      <Input
        value={changelog}
        onChange={(e) => setChangelog(e.target.value)}
        placeholder={t("skills.evo_changelog_placeholder", { defaultValue: "What changed and why" })}
      />
      <div className="flex justify-end gap-2">
        <Button variant="ghost" onClick={onCancel} disabled={busy}>{t("common.cancel")}</Button>
        <Button
          onClick={() => onSubmit({ prompt_context: content, changelog: changelog.trim() })}
          disabled={busy || !dirty || !changelog.trim()}
          leftIcon={busy ? <Loader2 className="w-4 h-4 animate-spin" /> : <EditIcon className="w-4 h-4" />}
        >
          {t("skills.evo_update", { defaultValue: "Update" })}
        </Button>
      </div>
    </div>
  );
}

function EvolvePatchPane({ skillName, onSubmit, onCancel, busy, t }: {
  skillName: string;
  onSubmit: (params: { old_string: string; new_string: string; changelog: string; replace_all: boolean }) => void;
  onCancel: () => void;
  busy: boolean;
  t: (key: string, opts?: any) => string;
}) {
  const [oldStr, setOldStr] = useState("");
  const [newStr, setNewStr] = useState("");
  const [changelog, setChangelog] = useState("");
  const [replaceAll, setReplaceAll] = useState(false);
  return (
    <div className="rounded-lg border border-border bg-surface-1 p-3 space-y-2">
      <p className="text-xs font-bold uppercase text-text-dim">
        {t("skills.evo_patch_title", { defaultValue: "Patch {{name}}", name: skillName })}
      </p>
      <div className="grid grid-cols-1 md:grid-cols-2 gap-2">
        <div>
          <label className="block text-[10px] font-bold uppercase text-text-dim mb-1">{t("skills.evo_patch_old", { defaultValue: "Find" })}</label>
          <textarea
            value={oldStr}
            onChange={(e) => setOldStr(e.target.value)}
            className="w-full h-40 px-2 py-1.5 text-xs rounded bg-surface-2 border border-border text-text-main resize-y font-mono"
          />
        </div>
        <div>
          <label className="block text-[10px] font-bold uppercase text-text-dim mb-1">{t("skills.evo_patch_new", { defaultValue: "Replace with" })}</label>
          <textarea
            value={newStr}
            onChange={(e) => setNewStr(e.target.value)}
            className="w-full h-40 px-2 py-1.5 text-xs rounded bg-surface-2 border border-border text-text-main resize-y font-mono"
          />
        </div>
      </div>
      <Input
        value={changelog}
        onChange={(e) => setChangelog(e.target.value)}
        placeholder={t("skills.evo_changelog_placeholder", { defaultValue: "What changed and why" })}
      />
      <label className="inline-flex items-center gap-2 text-xs text-text-dim">
        <input type="checkbox" checked={replaceAll} onChange={(e) => setReplaceAll(e.target.checked)} />
        {t("skills.evo_replace_all", { defaultValue: "Replace all occurrences" })}
      </label>
      <div className="flex justify-end gap-2">
        <Button variant="ghost" onClick={onCancel} disabled={busy}>{t("common.cancel")}</Button>
        <Button
          onClick={() => onSubmit({ old_string: oldStr, new_string: newStr, changelog: changelog.trim(), replace_all: replaceAll })}
          disabled={busy || !oldStr || !changelog.trim()}
          leftIcon={busy ? <Loader2 className="w-4 h-4 animate-spin" /> : <Code className="w-4 h-4" />}
        >
          {t("skills.evo_patch", { defaultValue: "Patch" })}
        </Button>
      </div>
    </div>
  );
}

function EvolveUploadPane({ skillName, onSubmit, onCancel, busy, t }: {
  skillName: string;
  onSubmit: (params: { path: string; content: string }) => void;
  onCancel: () => void;
  busy: boolean;
  t: (key: string, opts?: any) => string;
}) {
  const [subdir, setSubdir] = useState("references");
  const [filename, setFilename] = useState("");
  const [content, setContent] = useState("");
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const handleFilePick = async (file: File) => {
    // Read as text — supporting files are limited to 1 MiB per skill rules.
    if (file.size > 1024 * 1024) {
      alert(t("skills.evo_file_too_large", { defaultValue: "File exceeds 1 MiB limit" }));
      return;
    }
    const text = await file.text();
    setContent(text);
    if (!filename) setFilename(file.name);
  };
  const path = filename ? `${subdir}/${filename}` : "";
  return (
    <div className="rounded-lg border border-border bg-surface-1 p-3 space-y-2">
      <p className="text-xs font-bold uppercase text-text-dim">
        {t("skills.evo_upload_title", { defaultValue: "Add file to {{name}}", name: skillName })}
      </p>
      <div className="grid grid-cols-3 gap-2">
        <div>
          <label className="block text-[10px] font-bold uppercase text-text-dim mb-1">{t("skills.evo_folder", { defaultValue: "Folder" })}</label>
          <select
            value={subdir}
            onChange={(e) => setSubdir(e.target.value)}
            className="w-full px-2 py-1.5 text-xs rounded bg-surface-2 border border-border text-text-main"
          >
            <option value="references">references</option>
            <option value="templates">templates</option>
            <option value="scripts">scripts</option>
            <option value="assets">assets</option>
          </select>
        </div>
        <div className="col-span-2">
          <label className="block text-[10px] font-bold uppercase text-text-dim mb-1">{t("skills.evo_filename", { defaultValue: "Filename" })}</label>
          <Input value={filename} onChange={(e) => setFilename(e.target.value)} placeholder="example.md" />
        </div>
      </div>
      <div>
        <label className="block text-[10px] font-bold uppercase text-text-dim mb-1">{t("skills.evo_content", { defaultValue: "Content" })}</label>
        <textarea
          value={content}
          onChange={(e) => setContent(e.target.value)}
          className="w-full h-40 px-2 py-1.5 text-xs rounded bg-surface-2 border border-border text-text-main resize-y font-mono"
          placeholder={t("skills.evo_content_placeholder", { defaultValue: "Paste file content or load from disk below" })}
        />
        <div className="flex items-center gap-2 mt-1">
          <input
            ref={fileInputRef}
            type="file"
            className="hidden"
            onChange={(e) => { const f = e.target.files?.[0]; if (f) void handleFilePick(f); }}
          />
          <Button variant="ghost" onClick={() => fileInputRef.current?.click()} leftIcon={<Upload className="w-3 h-3" />}>
            {t("skills.evo_load_from_disk", { defaultValue: "Load from disk" })}
          </Button>
          <span className="text-[10px] text-text-dim">{content.length.toLocaleString()} chars</span>
        </div>
      </div>
      {path && <p className="text-[10px] text-text-dim font-mono">→ {path}</p>}
      <div className="flex justify-end gap-2">
        <Button variant="ghost" onClick={onCancel} disabled={busy}>{t("common.cancel")}</Button>
        <Button
          onClick={() => onSubmit({ path, content })}
          disabled={busy || !filename.trim() || !content}
          leftIcon={busy ? <Loader2 className="w-4 h-4 animate-spin" /> : <Upload className="w-4 h-4" />}
        >
          {t("skills.evo_upload", { defaultValue: "Upload" })}
        </Button>
      </div>
    </div>
  );
}

// Read-only viewer for a skill's supporting file. Fetches on demand so
// the rest of the detail modal doesn't pay for files the user never
// opens. Truncated responses are flagged so the user doesn't mistake a
// capped preview for complete content.
function SupportingFileViewer({ skillName, path, onClose, t }: {
  skillName: string;
  path: string;
  onClose: () => void;
  t: (key: string, opts?: any) => string;
}) {
  const { data, isLoading, error } = useQuery({
    queryKey: ["skill-file", skillName, path],
    queryFn: () => getSupportingFile(skillName, path),
  });
  return (
    <div className="rounded-lg border border-border bg-surface-1 p-3 space-y-2">
      <div className="flex items-center justify-between">
        <p className="text-xs font-bold uppercase text-text-dim font-mono">{path}</p>
        <button className="text-text-dim hover:text-text-main" onClick={onClose}><X className="w-4 h-4" /></button>
      </div>
      {isLoading && <div className="flex items-center justify-center py-6"><Loader2 className="w-5 h-5 animate-spin text-text-dim" /></div>}
      {error && <p className="text-xs text-error">{(error as any)?.message || "Failed to load file"}</p>}
      {data && (
        <>
          <pre className="max-h-80 overflow-auto whitespace-pre-wrap break-all text-[11px] bg-surface-2 p-2 rounded font-mono">{data.content}</pre>
          {data.truncated && (
            <p className="text-[10px] text-text-dim">
              {t("skills.evo_file_truncated", { defaultValue: "File truncated to 256 KiB preview" })}
            </p>
          )}
        </>
      )}
    </div>
  );
}

type EvolvePane = "none" | "update" | "patch" | "upload";

function SkillDetailModal({ skillName, isOpen, onClose, t }: {
  skillName: string | null;
  isOpen: boolean;
  onClose: () => void;
  t: (key: string, opts?: any) => string;
}) {
  const queryClient = useQueryClient();
  const { addToast } = useUIStore();
  const { data: detail, isLoading, refetch } = useQuery({
    // Use the shared factory so mutations' `invalidateQueries(skillKeys.all)`
    // also refresh open detail modals. The old `['skill-detail', name]`
    // namespace lived outside `skillKeys.all` and was never invalidated, so
    // users would see stale metadata after install/update/delete.
    queryKey: skillKeys.detail(skillName ?? ""),
    queryFn: () => getSkillDetail(skillName!),
    enabled: isOpen && !!skillName,
  });

  const [pane, setPane] = useState<EvolvePane>("none");
  const [viewingFile, setViewingFile] = useState<string | null>(null);
  useEffect(() => {
    // Reset sub-pane every time the modal is reopened or the skill changes.
    if (!isOpen) { setPane("none"); setViewingFile(null); }
  }, [isOpen, skillName]);

  const [busy, setBusy] = useState(false);

  // ── mutation helpers ───────────────────────────────────────────────
  const runMutation = async <T,>(fn: () => Promise<T>, successMsg: string) => {
    if (!skillName) return;
    setBusy(true);
    try {
      await fn();
      await refetch();
      queryClient.invalidateQueries({ queryKey: ["skills"] });
      addToast(successMsg, "success");
      setPane("none");
    } catch (e: any) {
      addToast(e?.message || "Action failed", "error");
    } finally {
      setBusy(false);
    }
  };

  const handleRollback = () => {
    if (!skillName) return;
    if (!confirm(t("skills.evo_rollback_confirm", { defaultValue: "Roll back to the previous version? This cannot be undone unless you patch again." }))) return;
    void runMutation(
      () => evolveRollbackSkill(skillName),
      t("skills.evo_rolled_back", { defaultValue: "Skill rolled back" })
    );
  };

  const handleRemoveFile = (path: string) => {
    if (!skillName) return;
    if (!confirm(t("skills.evo_remove_file_confirm", { defaultValue: `Remove ${path}?`, path }))) return;
    void runMutation(
      () => evolveRemoveFile(skillName, path),
      t("skills.evo_file_removed", { defaultValue: "File removed" })
    );
  };

  const handleDelete = () => {
    if (!skillName) return;
    // Delete goes through the evolve/delete path which enforces source
    // === Local/Native — safer than the Uninstall button which removes
    // any source. Two confirmations because this is destructive.
    if (!confirm(t("skills.evo_delete_confirm", { defaultValue: `Permanently delete ${skillName}? This cannot be undone.`, name: skillName }))) return;
    (async () => {
      if (!skillName) return;
      setBusy(true);
      try {
        await evolveDeleteSkill(skillName);
        queryClient.invalidateQueries({ queryKey: ["skills"] });
        addToast(t("skills.evo_deleted", { defaultValue: "Skill deleted" }), "success");
        onClose();
      } catch (e: any) {
        addToast(e?.message || "Delete failed", "error");
      } finally {
        setBusy(false);
      }
    })();
  };

  return (
    <Modal isOpen={isOpen} onClose={onClose} title={detail?.name || skillName || ""} size="xl">
      {isLoading ? (
        <div className="flex items-center justify-center py-12"><Loader2 className="w-6 h-6 animate-spin text-text-dim" /></div>
      ) : detail ? (
        <div className="space-y-5 p-1">
          {/* Header */}
          <div>
            <p className="text-sm text-text-dim italic">{detail.description}</p>
            <div className="flex flex-wrap gap-2 mt-2">
              <Badge variant="default">v{detail.version}</Badge>
              <Badge variant="default">{detail.runtime}</Badge>
              {detail.tags.map(tag => (
                <Badge key={tag} variant="default"><Tag className="w-3 h-3 mr-1" />{tag}</Badge>
              ))}
            </div>
          </div>

          {/* Evolve actions */}
          <div className="flex flex-wrap gap-2">
            <Button variant="ghost" onClick={() => setPane(pane === "update" ? "none" : "update")} leftIcon={<EditIcon className="w-4 h-4" />} disabled={busy}>
              {t("skills.evo_update", { defaultValue: "Update" })}
            </Button>
            <Button variant="ghost" onClick={() => setPane(pane === "patch" ? "none" : "patch")} leftIcon={<Code className="w-4 h-4" />} disabled={busy}>
              {t("skills.evo_patch", { defaultValue: "Patch" })}
            </Button>
            <Button variant="ghost" onClick={() => setPane(pane === "upload" ? "none" : "upload")} leftIcon={<Upload className="w-4 h-4" />} disabled={busy}>
              {t("skills.evo_add_file", { defaultValue: "Add File" })}
            </Button>
            <Button
              variant="ghost"
              onClick={handleRollback}
              leftIcon={<RotateCcw className="w-4 h-4" />}
              disabled={busy || detail.evolution.versions.length < 1}
              title={detail.evolution.versions.length < 1 ? t("skills.evo_no_rollback", { defaultValue: "No prior version to roll back to" }) : ""}
            >
              {t("skills.evo_rollback", { defaultValue: "Rollback" })}
            </Button>
            <Button
              variant="ghost"
              className="text-error hover:text-error ml-auto"
              onClick={handleDelete}
              leftIcon={<Trash2 className="w-4 h-4" />}
              disabled={busy}
              title={t("skills.evo_delete_title", { defaultValue: "Delete this agent-evolved skill" })}
            >
              {t("skills.evo_delete", { defaultValue: "Delete" })}
            </Button>
          </div>

          {/* Embedded edit panes */}
          {pane === "update" && skillName && (
            <EvolveUpdatePane
              skillName={skillName}
              initialContent={detail.prompt_context || ""}
              onSubmit={(params) => runMutation(
                () => evolveUpdateSkill(skillName, params),
                t("skills.evo_updated", { defaultValue: "Skill updated" })
              )}
              onCancel={() => setPane("none")}
              busy={busy}
              t={t}
            />
          )}
          {pane === "patch" && skillName && (
            <EvolvePatchPane
              skillName={skillName}
              onSubmit={(params) => runMutation(
                () => evolvePatchSkill(skillName, params),
                t("skills.evo_patched", { defaultValue: "Skill patched" })
              )}
              onCancel={() => setPane("none")}
              busy={busy}
              t={t}
            />
          )}
          {pane === "upload" && skillName && (
            <EvolveUploadPane
              skillName={skillName}
              onSubmit={(params) => runMutation(
                () => evolveWriteFile(skillName, params),
                t("skills.evo_file_uploaded", { defaultValue: "File uploaded" })
              )}
              onCancel={() => setPane("none")}
              busy={busy}
              t={t}
            />
          )}

          {/* Inline supporting-file viewer. Open on click, close via X. */}
          {viewingFile && skillName && (
            <SupportingFileViewer
              skillName={skillName}
              path={viewingFile}
              onClose={() => setViewingFile(null)}
              t={t}
            />
          )}

          {/* Stats */}
          <div className="grid grid-cols-3 gap-3">
            <div className="p-3 rounded-lg bg-surface-2 text-center">
              <p className="text-2xl font-black">{detail.tools.length}</p>
              <p className="text-[10px] font-bold uppercase text-text-dim">{t("skills.tools")}</p>
            </div>
            <div className="p-3 rounded-lg bg-surface-2 text-center">
              <p className="text-2xl font-black">{detail.evolution.use_count}</p>
              <p className="text-[10px] font-bold uppercase text-text-dim">{t("skills.evo_uses", { defaultValue: "Uses" })}</p>
            </div>
            <div className="p-3 rounded-lg bg-surface-2 text-center">
              <p className="text-2xl font-black">{detail.evolution.evolution_count}</p>
              <p className="text-[10px] font-bold uppercase text-text-dim">{t("skills.evo_evolutions", { defaultValue: "Evolutions" })}</p>
            </div>
          </div>

          {/* Tools */}
          {detail.tools.length > 0 && (
            <div>
              <h3 className="text-xs font-bold uppercase text-text-dim mb-2"><Wrench className="w-3 h-3 inline mr-1" />{t("skills.tools")}</h3>
              <div className="space-y-1">
                {detail.tools.map(tool => (
                  <div key={tool.name} className="px-3 py-2 rounded bg-surface-2 text-xs">
                    <span className="font-mono font-bold">{tool.name}</span>
                    <span className="text-text-dim ml-2">{tool.description}</span>
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* Linked Files */}
          {Object.keys(detail.linked_files).length > 0 && (
            <div>
              <h3 className="text-xs font-bold uppercase text-text-dim mb-2"><FileText className="w-3 h-3 inline mr-1" />{t("skills.evo_files", { defaultValue: "Supporting Files" })}</h3>
              {Object.entries(detail.linked_files).map(([dir, files]) => (
                <div key={dir} className="mb-2">
                  <p className="text-[10px] font-bold uppercase text-text-dim mb-1">{dir}/</p>
                  <div className="flex flex-wrap gap-1">
                    {files.map(f => {
                      const rel = `${dir}/${f}`;
                      return (
                        <span key={f} className="inline-flex items-center gap-1 px-2 py-0.5 rounded bg-surface-2 text-xs font-mono group">
                          <button
                            className="hover:text-brand"
                            onClick={() => setViewingFile(rel)}
                            title={t("skills.evo_view_file", { defaultValue: "View file" })}
                          >
                            {f}
                          </button>
                          <button
                            className="opacity-0 group-hover:opacity-100 transition-opacity text-error hover:text-error-dim"
                            onClick={() => handleRemoveFile(rel)}
                            title={t("skills.evo_remove_file", { defaultValue: "Remove file" })}
                            disabled={busy}
                          >
                            <X className="w-3 h-3" />
                          </button>
                        </span>
                      );
                    })}
                  </div>
                </div>
              ))}
            </div>
          )}

          {/* Version History */}
          {detail.evolution.versions.length > 0 && (
            <div>
              <h3 className="text-xs font-bold uppercase text-text-dim mb-2"><History className="w-3 h-3 inline mr-1" />{t("skills.evo_history", { defaultValue: "Version History" })}</h3>
              <div className="space-y-2 max-h-48 overflow-y-auto">
                {[...detail.evolution.versions].reverse().map((v, i) => (
                  <div key={i} className="flex items-start gap-3 px-3 py-2 rounded bg-surface-2 text-xs">
                    <Badge variant={i === 0 ? "success" : "default"}>v{v.version}</Badge>
                    <div className="flex-1 min-w-0">
                      <p className="text-text-main">{v.changelog}</p>
                      <p className="text-[10px] text-text-dim mt-0.5">
                        {new Date(v.timestamp).toLocaleString()}
                        {v.author && <span className="ml-2 font-mono">· {v.author}</span>}
                      </p>
                    </div>
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* Meta */}
          <div className="text-[10px] text-text-dim space-y-0.5 pt-2 border-t border-border">
            <p>{t("skills.author")}: {detail.author || "-"}</p>
            <p>{t("skills.evo_prompt_size", { defaultValue: "Prompt context" })}: {detail.prompt_context_length.toLocaleString()} chars</p>
            <p className="font-mono truncate">{detail.path}</p>
          </div>
        </div>
      ) : (
        <p className="text-sm text-text-dim py-8 text-center">{t("skills.evo_not_found", { defaultValue: "Skill not found" })}</p>
      )}
    </Modal>
  );
}

// Marketplace Skill Card
function MarketplaceSkillCard({ skill, onInstall, pendingId, onViewDetails, source = "clawhub", t }: {
  skill: ClawHubSkillWithStatus;
  pendingId: string | null;
  onInstall: (slug: string) => void;
  onViewDetails: (skill: ClawHubSkillWithStatus) => void;
  source?: MarketplaceSource;
  t: (key: string) => string;
}) {
  return (
    <Card hover padding="none" className="flex flex-col overflow-hidden group cursor-pointer" onClick={() => onViewDetails(skill)}>
      <div className={`h-1.5 bg-gradient-to-r ${source === "skillhub" ? "from-accent via-accent/60 to-accent/30" : "from-brand via-brand/60 to-brand/30"}`} />
      <div className="p-5 flex-1 flex flex-col">
        <div className="flex items-start justify-between gap-3 mb-4">
          <div className="flex items-center gap-3 min-w-0">
            <div className={`w-10 h-10 rounded-lg flex items-center justify-center text-xl bg-gradient-to-br ${source === "skillhub" ? "from-accent/10 to-accent/5 border border-accent/20" : "from-brand/10 to-brand/5 border border-brand/20"}`}>
              {source === "skillhub"
                ? <Store className="w-5 h-5 text-accent" />
                : <Sparkles className="w-5 h-5 text-brand" />}
            </div>
            <div className="min-w-0">
              <h2 className={`text-base font-black truncate transition-colors ${source === "skillhub" ? "group-hover:text-accent" : "group-hover:text-brand"}`}>{skill.name}</h2>
              <p className="text-[10px] font-black uppercase tracking-widest text-text-dim/60 truncate">v{skill.version || "1.0.0"}</p>
            </div>
          </div>
          {skill.is_installed && <Badge variant="success">{t("skills.installed")}</Badge>}
        </div>
        <p className="text-xs text-text-dim line-clamp-2 italic mb-4 flex-1">{skill.description || "-"}</p>

        {/* Stats */}
        <div className="flex items-center gap-4 mb-4 text-[10px] font-bold text-text-dim">
          {skill.stars !== undefined ? (
            <>
              <span className="flex items-center gap-1">
                <Star className="w-3 h-3 text-warning" />
                {skill.stars}
              </span>
              <span className="flex items-center gap-1">
                <Download className="w-3 h-3" />
                {skill.downloads}
              </span>
            </>
          ) : skill.updated_at ? (
            <span className="flex items-center gap-1 text-text-dim">
              <Calendar className="w-3 h-3" />
              {formatDate(skill.updated_at)}
            </span>
          ) : null}
        </div>

        {/* Actions */}
        <div className="flex gap-2 mt-auto" onClick={e => e.stopPropagation()}>
          {skill.is_installed ? (
            <Button variant="secondary" size="sm" className="flex-1" disabled>
              <CheckCircle2 className="w-3 h-3" />
              {t("skills.installed")}
            </Button>
          ) : (
            <Button
              variant="primary"
              size="sm"
              className="flex-1"
              onClick={(e) => { e.stopPropagation(); onInstall(skill.slug); }}
              disabled={pendingId === skill.slug}
              leftIcon={pendingId === skill.slug ? <Loader2 className="w-3 h-3 animate-spin" /> : <Download className="w-3 h-3" />}
            >
              {pendingId === skill.slug ? t("skills.installing") : t("skills.install")}
            </Button>
          )}
        </div>
      </div>
    </Card>
  );
}

// Details Modal
function DetailsModal({ skill, onClose, onInstall, pendingId, source = "clawhub", t }: {
  skill: ClawHubSkillWithStatus;
  onClose: () => void;
  onInstall: () => void;
  pendingId: string | null;
  source?: MarketplaceSource;
  t: (key: string) => string;
}) {
  return (
    <div className="fixed inset-0 z-50 flex items-end sm:items-center justify-center p-0 sm:p-4 bg-black/50 backdrop-blur-sm" onClick={onClose}>
      <div className="bg-surface rounded-2xl border border-border-subtle w-full sm:max-w-lg shadow-2xl rounded-t-2xl sm:rounded-2xl max-h-[90vh] overflow-y-auto animate-fade-in-scale" onClick={e => e.stopPropagation()}>
        <div className={`h-2 bg-gradient-to-r rounded-t-2xl ${source === "skillhub" ? "from-accent via-accent/60 to-accent/30" : "from-brand via-brand/60 to-brand/30"}`} />
        <div className="p-6 border-b border-border-subtle">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <div className={`w-12 h-12 rounded-xl flex items-center justify-center text-2xl ${source === "skillhub" ? "bg-accent/10 border border-accent/20" : "bg-brand/10 border border-brand/20"}`}>
                {source === "skillhub"
                  ? <Store className="w-6 h-6 text-accent" />
                  : <Sparkles className="w-6 h-6 text-brand" />}
              </div>
              <div>
                <h2 className="text-xl font-black">{skill.name}</h2>
                <p className="text-xs font-black uppercase tracking-widest text-text-dim/60">v{skill.version || "1.0.0"}</p>
              </div>
            </div>
            <button onClick={onClose} className="p-2 hover:bg-main/30 rounded-lg transition-colors" aria-label={t("common.close")}>
              <X className="w-5 h-5 text-text-dim" />
            </button>
          </div>
        </div>

        <div className="p-6 space-y-4">
          <div className="p-4 rounded-xl bg-main/30">
            <p className="text-sm text-text-dim">{skill.description}</p>
          </div>

          <div className="flex items-center gap-6 text-xs font-bold text-text-dim">
            {skill.stars !== undefined ? (
              <>
                <span className="flex items-center gap-1">
                  <Star className="w-4 h-4 text-warning" />
                  {skill.stars} {t("skills.stars_count")}
                </span>
                <span className="flex items-center gap-1">
                  <Download className="w-4 h-4" />
                  {skill.downloads} {t("skills.downloads_count")}
                </span>
              </>
            ) : skill.updated_at ? (
              <span className="flex items-center gap-1">
                <Calendar className="w-4 h-4" />
                {formatDate(skill.updated_at)}
              </span>
            ) : null}
          </div>

          {skill.tags && skill.tags.length > 0 && (
            <div className="flex flex-wrap gap-2">
              {skill.tags.map(tag => (
                <span key={tag} className={`px-2 py-1 rounded-lg text-xs font-bold ${source === "skillhub" ? "bg-accent/10 text-accent" : "bg-brand/10 text-brand"}`}>{tag}</span>
              ))}
            </div>
          )}

          <div className="flex gap-2 pt-2">
            {skill.is_installed ? (
              <Button variant="secondary" className="flex-1" disabled leftIcon={<CheckCircle2 className="w-4 h-4" />}>
                {t("skills.installed")}
              </Button>
            ) : (
              <Button
                variant="primary"
                className="flex-1"
                onClick={onInstall}
                disabled={pendingId === skill.slug}
                leftIcon={pendingId === skill.slug ? <Loader2 className="w-4 h-4 animate-spin" /> : <Download className="w-4 h-4" />}
              >
                {pendingId === skill.slug ? t("skills.installing") : t("skills.install")}
              </Button>
            )}
          </div>
        </div>

        <div className="p-4 border-t border-border-subtle flex justify-end">
          <Button variant="ghost" onClick={onClose}>{t("common.close")}</Button>
        </div>
      </div>
    </div>
  );
}

// Uninstall Dialog
function UninstallDialog({ skillName, onClose, onConfirm, isPending }: {
  skillName: string;
  onClose: () => void;
  onConfirm: () => void;
  isPending: boolean;
}) {
  const { t } = useTranslation();

  return (
    <div className="fixed inset-0 bg-black/50 flex items-end sm:items-center justify-center z-50 backdrop-blur-sm" onClick={onClose}>
      <div className="bg-surface border border-border-subtle rounded-2xl w-full sm:max-w-sm p-4 sm:p-6 rounded-t-2xl sm:rounded-2xl shadow-2xl animate-fade-in-scale" onClick={e => e.stopPropagation()}>
        <h3 className="text-lg font-black mb-2">{t("skills.uninstall_confirm_title")}</h3>
        <p className="text-sm text-text-dim mb-6">{t("skills.uninstall_confirm", { name: skillName })}</p>
        <div className="flex gap-3">
          <Button variant="secondary" className="flex-1" onClick={onClose}>{t("common.cancel")}</Button>
          <Button variant="primary" className="flex-1 bg-error! hover:bg-error/90!" onClick={onConfirm} disabled={isPending}>
            {isPending ? "..." : t("common.confirm")}
          </Button>
        </div>
      </div>
    </div>
  );
}

export function SkillsPage() {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);
  const queryClient = useQueryClient();

  // View state — default to the region-appropriate marketplace
  const [viewMode, setViewMode] = useState<ViewMode>("fanghub");
  const [selectedCategory, setSelectedCategory] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [skillhubSearch_, setSkillhubSearch] = useState("");

  // Actions
  const [uninstalling, setUninstalling] = useState<string | null>(null);
  const [detailsSkill, setDetailsSkill] = useState<ClawHubSkillWithStatus | null>(null);
  const [detailsSource, setDetailsSource] = useState<MarketplaceSource>("clawhub");
  const [installingId, setInstallingId] = useState<string | null>(null);
  const [targetHand, setTargetHand] = useState<string>("");

  // Skill evolution state
  const [showCreateModal, setShowCreateModal] = useState(false);
  const [detailSkillName, setDetailSkillName] = useState<string | null>(null);

  const handsQuery = useHands();
  const hands = handsQuery.data ?? [];

  // Get search keyword from category or use search input
  const searchKeyword = selectedCategory
    ? categories.find(c => c.id === selectedCategory)?.keyword || ""
    : search;

  // Queries
  const skillsQuery = useSkills();

  // ClawHub search — always runs in marketplace mode; falls back to "python"
  const effectiveKeyword = searchKeyword || "python";
  const searchQuery = useQuery({
    ...skillQueries.clawhubSearch(effectiveKeyword),
    enabled: viewMode === "marketplace",
  });

  // Skillhub queries — category selection also drives search.
  // Browse only fires when no keyword; search only fires when keyword present.
  const skillhubKeyword = skillhubSearch_ || (selectedCategory ? categories.find(c => c.id === selectedCategory)?.keyword || "" : "");

  const skillhubBrowseQuery = useQuery({
    ...skillQueries.skillhubBrowse(),
    enabled: viewMode === "skillhub" && !skillhubKeyword,
  });
  const skillhubSearchQuery = useQuery({
    ...skillQueries.skillhubSearch(skillhubKeyword),
    enabled: viewMode === "skillhub" && !!skillhubKeyword,
  });

  const activeSkillhubQuery = skillhubKeyword ? skillhubSearchQuery : skillhubBrowseQuery;

  // FangHub — official skills from local registry (~/.librefang/registry/skills)
  const fanghubQuery = useQuery({
    ...skillQueries.fanghubList(),
    enabled: viewMode === "fanghub",
  });

  // Detail query — conditionally fetches from skillhub or clawhub via factory keys
  const clawhubDetailQuery = useQuery({
    ...skillQueries.clawhubSkill(detailsSkill?.slug ?? ""),
    enabled: !!detailsSkill?.slug && detailsSource === "clawhub",
  });
  const skillhubDetailQuery = useQuery({
    ...skillQueries.skillhubSkill(detailsSkill?.slug ?? ""),
    enabled: !!detailsSkill?.slug && detailsSource === "skillhub",
  });
  const detailQuery = detailsSource === "skillhub" ? skillhubDetailQuery : clawhubDetailQuery;

  const skillWithDetails = detailQuery.data && detailsSkill
    ? {
        ...detailsSkill,
        ...detailQuery.data,
        is_installed: detailQuery.data.is_installed ?? detailQuery.data.installed,
      } as ClawHubSkillWithStatus
    : detailsSkill;

  const installedSkills = skillsQuery.data ?? [];
  const isInstalledFromMarketplace = (slug: string, source: MarketplaceSource) =>
    installedSkills.some((skill) => skill.source?.type === source && skill.source?.slug === slug);

  const marketplaceSkills = searchQuery.data?.items ?? [];
  const isMarketplaceLoading = searchQuery.isLoading;
  const marketplaceError = searchQuery.error as any;
  const isRateLimited = marketplaceError?.message?.includes("429") || marketplaceError?.message?.includes("rate") || marketplaceError?.message?.includes("Rate limit") || marketplaceError?.status === 429;

  const filteredMarketplace = useMemo(
    () => marketplaceSkills
      .map(s => ({ ...s, is_installed: isInstalledFromMarketplace(s.slug, "clawhub") }))
      .filter(s => !search || s.name.toLowerCase().includes(search.toLowerCase()) || s.description?.toLowerCase().includes(search.toLowerCase())),
    [marketplaceSkills, installedSkills, search],
  );
  const skillhubSkills = activeSkillhubQuery.data?.items ?? [];
  const isSkillhubLoading = activeSkillhubQuery.isLoading;
  const skillhubError = activeSkillhubQuery.error as any;
  const isSkillhubRateLimited = skillhubError?.message?.includes("429") || skillhubError?.message?.includes("rate") || skillhubError?.message?.includes("Rate limit") || skillhubError?.status === 429;

  const filteredSkillhub = useMemo(() => {
    const all = skillhubSkills.map(s => ({ ...s, is_installed: isInstalledFromMarketplace(s.slug, "skillhub") }));
    if (!selectedCategory) return all;
    const kws = (categories.find(c => c.id === selectedCategory)?.keyword || "").toLowerCase().split(" ");
    return all.filter(s =>
      kws.some(kw =>
        s.name.toLowerCase().includes(kw) ||
        s.description?.toLowerCase().includes(kw) ||
        s.tags?.some((tag: string) => tag.toLowerCase().includes(kw))
      )
    );
  }, [skillhubSkills, installedSkills, selectedCategory]);
  const fanghubSkills = fanghubQuery.data?.skills ?? [];
  const filteredFanghub = useMemo(() => {
    if (!selectedCategory) return fanghubSkills;
    const keyword = categories.find(c => c.id === selectedCategory)?.keyword || "";
    const kws = keyword.toLowerCase().split(" ");
    return fanghubSkills.filter(s =>
      kws.some(kw =>
        s.name.toLowerCase().includes(kw) ||
        s.description?.toLowerCase().includes(kw) ||
        s.tags?.some(tag => tag.toLowerCase().includes(kw))
      )
    );
  }, [fanghubSkills, selectedCategory]);

  // Mutations
  const uninstallMutation = useUninstallSkill();
  const installMutation = useClawHubInstall();
  const skillhubInstallMutation = useSkillHubInstall();
  const fanghubInstallMutation = useInstallSkill();

  const handleCategoryClick = (categoryId: string) => {
    if (selectedCategory === categoryId) {
      setSelectedCategory(null);
    } else {
      setSelectedCategory(categoryId);
      setSearch("");
      setSkillhubSearch("");
    }
  };

  const handleInstall = (slug: string, source: MarketplaceSource = "clawhub") => {
    setInstallingId(slug);
    const hand = targetHand || undefined;
    const opts = {
      onSuccess: () => {
        addToast(t("common.success"), "success");
        setInstallingId(null);
        setDetailsSkill(null);
      },
      onError: (error: any) => {
        const msg = error.message || t("common.error");
        addToast(msg.includes("abort") ? t("skills.install_timeout") : msg, "error");
        setInstallingId(null);
      },
    };
    if (source === "skillhub") {
      skillhubInstallMutation.mutate({ slug, hand }, opts);
    } else {
      installMutation.mutate({ slug, hand }, opts);
    }
  };

  const handleUninstall = (name: string) => setUninstalling(name);
  const confirmUninstall = () => {
    if (uninstalling) {
      uninstallMutation.mutate(uninstalling, {
        onSuccess: () => {
          addToast(t("common.success"), "success");
          setUninstalling(null);
        },
      });
    }
  };
  const handleViewDetails = (skill: ClawHubSkillWithStatus, source: MarketplaceSource) => {
    setDetailsSkill(skill);
    setDetailsSource(source);
  };

  const isAnyFetching = skillsQuery.isFetching || searchQuery.isFetching
    || skillhubBrowseQuery.isFetching || skillhubSearchQuery.isFetching || fanghubQuery.isFetching;

  return (
    <div className="flex flex-col gap-4 transition-colors duration-300">
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2 min-w-0">
          <div className="p-1.5 rounded-lg bg-brand/10 text-brand shrink-0"><Wrench className="h-4 w-4" /></div>
          <div className="min-w-0">
            <h1 className="text-base font-extrabold tracking-tight">{t("skills.title")}</h1>
            <p className="text-[11px] text-text-dim hidden sm:block">{t("skills.subtitle")}</p>
          </div>
        </div>
        <div className="flex items-center gap-2 shrink-0">
          <span className="hidden sm:inline-block px-2.5 py-1 rounded-full border border-border-subtle bg-surface text-[10px] font-bold uppercase text-text-dim">
            {t("skills.installed_count", { count: installedSkills.length })}
          </span>
          <a
            href="https://librefang.ai/skills"
            target="_blank"
            rel="noopener noreferrer"
            className="hidden md:flex h-8 items-center gap-1.5 rounded-xl border border-border-subtle bg-surface px-3 text-xs font-bold text-text-dim hover:text-brand hover:border-brand/30 transition-colors"
            title={t("skills.browse_registry_title", { defaultValue: "Browse the full skill registry on librefang.ai" })}
          >
            <Globe className="h-3.5 w-3.5" />
            <span>{t("skills.browse_registry", { defaultValue: "Registry" })}</span>
          </a>
          <button
            className="flex h-8 items-center gap-1.5 rounded-xl border border-brand/30 bg-brand/10 px-3 text-xs font-bold text-brand hover:bg-brand/20 transition-colors"
            onClick={() => setShowCreateModal(true)}
          >
            <Plus className="h-3.5 w-3.5" />
            <span className="hidden sm:inline">{t("skills.evo_create", { defaultValue: "Create Skill" })}</span>
          </button>
          <button
            className="flex h-8 items-center gap-1.5 rounded-xl border border-brand/30 bg-brand/10 px-3 text-xs font-bold text-brand hover:bg-brand/20 transition-colors"
            onClick={() => setShowCreateModal(true)}
          >
            <Plus className="h-3.5 w-3.5" />
            <span className="hidden sm:inline">{t("skills.evo_create", { defaultValue: "Create Skill" })}</span>
          </button>
          <button
            className="flex h-8 items-center gap-1.5 rounded-xl border border-border-subtle bg-surface px-3 text-xs font-bold text-text-dim hover:text-brand hover:border-brand/30 transition-colors"
            onClick={async () => {
              // Rescan the skills directory on disk before refetching the list.
              // The kernel holds a cached registry — a plain query refetch only
              // re-reads that cache. A full reload picks up skills created by
              // the CLI, the agent evolve tools, or direct FS edits while the
              // dashboard was open.
              try {
                const res = await reloadSkills();
                addToast(
                  t("skills.reloaded", { defaultValue: "Rescanned skills directory ({{count}} loaded)", count: (res as any).count ?? 0 }),
                  "success",
                );
              } catch (e: any) {
                addToast(e?.message || "Reload failed", "error");
              }
              void skillsQuery.refetch();
              void searchQuery.refetch();
              void activeSkillhubQuery.refetch();
            }}
          >
            <RefreshCw className={`h-3.5 w-3.5 ${isAnyFetching ? "animate-spin" : ""}`} />
            <span className="hidden sm:inline">{t("skills.reload_from_disk", { defaultValue: "Reload" })}</span>
          </button>
        </div>
      </div>

      {/* View Toggle */}
      <div className="flex gap-1 p-1 bg-main/30 rounded-xl w-fit">
        <button
          onClick={() => { setViewMode("installed"); setSearch(""); setSkillhubSearch(""); setSelectedCategory(null); }}
          className={`flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-bold transition-colors ${
            viewMode === "installed" ? "bg-surface text-success shadow-sm" : "bg-surface-hover text-text-dim hover:text-text-main"
          }`}
        >
          <Package className="w-4 h-4" />
          {t("skills.installed")}
          <span className={`ml-1 px-1.5 py-0.5 rounded-full text-[10px] ${viewMode === "installed" ? "bg-success/20 text-success" : "bg-border-subtle text-text-dim"}`}>
            {installedSkills.length}
          </span>
        </button>

        <button
          onClick={() => { setViewMode("fanghub"); setSearch(""); setSkillhubSearch(""); setSelectedCategory(null); }}
          className={`relative flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-bold transition-colors ${
            viewMode === "fanghub" ? "bg-surface text-brand shadow-sm" : "bg-surface-hover text-text-dim hover:text-text-main"
          }`}
        >
          <Zap className="w-4 h-4" />
          {t("skills.builtin")}
          <span className={`absolute top-0.5 right-1 text-[8px] font-black px-1 py-px rounded-full leading-none ${viewMode === "fanghub" ? "bg-brand text-white" : "bg-border-subtle text-text-dim"}`}>{t("skills.official")}</span>
        </button>

        {!USE_SKILLHUB && (
          <button
            onClick={() => { setViewMode("marketplace"); setSkillhubSearch(""); setSelectedCategory(null); }}
            className={`flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-bold transition-colors ${
              viewMode === "marketplace" ? "bg-surface text-brand shadow-sm" : "bg-surface-hover text-text-dim hover:text-text-main"
            }`}
          >
            <Sparkles className="w-4 h-4" />
            {t("skills.marketplace")}
          </button>
        )}

        {USE_SKILLHUB && (
          <button
            onClick={() => { setViewMode("skillhub"); setSearch(""); setSelectedCategory(null); }}
            className={`flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-bold transition-colors ${
              viewMode === "skillhub" ? "bg-surface text-accent shadow-sm" : "bg-surface-hover text-text-dim hover:text-text-main"
            }`}
          >
            <Store className="w-4 h-4" />
            {t("skills.skillhub")}
          </button>
        )}
      </div>

      {/* Install Target: Global or Hand */}
      {viewMode !== "installed" && hands.length > 0 && (
        <div className="flex items-center gap-2">
          <span className="text-[11px] font-bold text-text-dim">{t("skills.install_to")}:</span>
          <select
            value={targetHand}
            onChange={(e) => setTargetHand(e.target.value)}
            className="rounded-lg border border-border-subtle bg-surface px-3 py-1.5 text-xs font-bold text-text-main"
          >
            <option value="">{t("skills.global")}</option>
            {hands.map((h: HandDefinitionItem) => (
              <option key={h.id} value={h.id}>{h.name || h.id}</option>
            ))}
          </select>
        </div>
      )}

      {/* Category Chips */}
      {(viewMode === "marketplace" || viewMode === "skillhub" || viewMode === "fanghub") && (
        <div className="flex flex-wrap gap-1.5 sm:gap-2">
          <button
            onClick={() => { setSelectedCategory(null); }}
            className={`flex items-center gap-2 px-3 py-1.5 rounded-lg text-xs font-bold transition-colors ${
              !selectedCategory ? "bg-brand text-white shadow-md" : "bg-main/50 text-text-dim hover:bg-main hover:text-text-main border border-border-subtle"
            }`}
          >
            {t("common.all")}
          </button>
          {categories.map(cat => (
            <button
              key={cat.id}
              onClick={() => handleCategoryClick(cat.id)}
              className={`flex items-center gap-2 px-3 py-1.5 rounded-lg text-xs font-bold transition-colors ${
                selectedCategory === cat.id ? "bg-brand text-white shadow-md" : "bg-main/50 text-text-dim hover:bg-main hover:text-text-main border border-border-subtle"
              }`}
            >
              {getCategoryIcon(cat.id)}
              {t(cat.nameKey)}
            </button>
          ))}
        </div>
      )}

      {/* Search — ClawHub */}
      {viewMode === "marketplace" && (
        <Input
          value={search}
          onChange={(e) => { setSearch(e.target.value); setSelectedCategory(null); }}
          placeholder={selectedCategory ? t(categories.find(c => c.id === selectedCategory)?.nameKey ?? "") + "..." : t("skills.search_placeholder")}
          leftIcon={<Search className="w-4 h-4" />}
          rightIcon={search ? (
            <button onClick={() => setSearch("")} className="hover:text-text-main" aria-label={t("common.clear_search", { defaultValue: "Clear search" })}>
              <X className="w-3 h-3" />
            </button>
          ) : undefined}
        />
      )}

      {/* Search — Skillhub */}
      {viewMode === "skillhub" && (
        <Input
          value={skillhubSearch_}
          onChange={(e) => { setSkillhubSearch(e.target.value); }}
          placeholder={t("skills.skillhub_search_placeholder")}
          leftIcon={<Search className="w-4 h-4" />}
          rightIcon={skillhubSearch_ ? (
            <button onClick={() => setSkillhubSearch("")} className="hover:text-text-main">
              <X className="w-3 h-3" />
            </button>
          ) : undefined}
        />
      )}

      {/* Content */}
      {viewMode === "installed" ? (
        skillsQuery.isLoading ? (
          <div className="grid gap-2 sm:gap-4 sm:grid-cols-2 md:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5 2xl:grid-cols-6">
            {[1, 2, 3, 4, 5, 6].map(i => <CardSkeleton key={i} />)}
          </div>
        ) : installedSkills.length === 0 ? (
          <EmptyState title={t("skills.no_skills")} icon={<Package className="h-6 w-6" />} />
        ) : (
          <div className="grid gap-2 sm:gap-4 sm:grid-cols-2 md:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5 2xl:grid-cols-6">
            {installedSkills.map(s => (
              <InstalledSkillCard key={s.name} skill={s} onUninstall={handleUninstall} onViewDetail={setDetailSkillName} t={t} />
            ))}
          </div>
        )
      ) : viewMode === "marketplace" ? (
        isMarketplaceLoading ? (
          <div className="grid gap-2 sm:gap-4 sm:grid-cols-2 md:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5 2xl:grid-cols-6">
            {[1, 2, 3, 4, 5, 6].map(i => <CardSkeleton key={i} />)}
          </div>
        ) : isRateLimited ? (
          <EmptyState title={t("skills.rate_limited")} description={t("skills.rate_limited_desc")} icon={<Loader2 className="h-6 w-6 animate-spin" />} />
        ) : marketplaceError ? (
          <EmptyState title={t("skills.load_error")} description={marketplaceError.message || t("common.error")} icon={<Search className="h-6 w-6" />} />
        ) : filteredMarketplace.length === 0 ? (
          <EmptyState title={t("skills.no_results")} description={search ? t("skills.try_different_search", { defaultValue: "Try a different search term." }) : t("skills.browse_unavailable", { defaultValue: "Browse is temporarily unavailable. Try searching above." })} icon={<Search className="h-6 w-6" />} />
        ) : (
          <div>
            <div className="grid gap-2 sm:gap-4 sm:grid-cols-2 md:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5 2xl:grid-cols-6">
              {filteredMarketplace.map(s => (
                <MarketplaceSkillCard key={s.slug} skill={s} pendingId={installingId}
                  onInstall={(slug) => handleInstall(slug, "clawhub")}
                  onViewDetails={(sk) => handleViewDetails(sk, "clawhub")}
                  source="clawhub" t={t} />
              ))}
            </div>
          </div>
        )
      ) : viewMode === "skillhub" ? (
        isSkillhubLoading ? (
          <div className="grid gap-2 sm:gap-4 sm:grid-cols-2 md:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5 2xl:grid-cols-6">
            {[1, 2, 3, 4, 5, 6].map(i => <CardSkeleton key={i} />)}
          </div>
        ) : isSkillhubRateLimited ? (
          <EmptyState title={t("skills.rate_limited")} description={t("skills.skillhub_rate_limited_desc")} icon={<Loader2 className="h-6 w-6 animate-spin" />} />
        ) : skillhubError ? (
          <EmptyState title={t("skills.load_error")} description={skillhubError.message || t("common.error")} icon={<Search className="h-6 w-6" />} />
        ) : filteredSkillhub.length === 0 ? (
          <EmptyState title={t("skills.no_results")} icon={<Search className="h-6 w-6" />} />
        ) : (
          <div>
            <div className="grid gap-2 sm:gap-4 sm:grid-cols-2 md:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5 2xl:grid-cols-6">
              {filteredSkillhub.map(s => (
                <MarketplaceSkillCard key={s.slug} skill={s} pendingId={installingId}
                  onInstall={(slug) => handleInstall(slug, "skillhub")}
                  onViewDetails={(sk) => handleViewDetails(sk, "skillhub")}
                  source="skillhub" t={t} />
              ))}
            </div>
          </div>
        )
      ) : (
        /* viewMode === "fanghub" — official LibreFang registry skills */
        fanghubQuery.isLoading ? (
          <div className="grid gap-2 sm:gap-4 sm:grid-cols-2 md:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5 2xl:grid-cols-6">{[1, 2, 3].map(i => <CardSkeleton key={i} />)}</div>
        ) : filteredFanghub.length === 0 ? (
          <EmptyState title={t("skills.no_results")} icon={<Zap className="h-6 w-6" />} />
        ) : (
          <div>
          <div className="grid gap-2 sm:gap-4 sm:grid-cols-2 md:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5 2xl:grid-cols-6">
            {filteredFanghub.map((skill: FangHubSkill) => (
              <FangHubSkillCard
                key={skill.name}
                skill={skill}
                pendingId={installingId}
                onInstall={(name) => {
                  setInstallingId(name);
                  fanghubInstallMutation.mutate(
                    { name, hand: targetHand || undefined },
                    {
                      onSuccess: () => {
                        addToast(t("common.success"), "success");
                        setInstallingId(null);
                      },
                      onError: (error: any) => {
                        const msg = error.message || t("common.error");
                        addToast(msg.includes("abort") ? t("skills.install_timeout") : msg, "error");
                        setInstallingId(null);
                      },
                    },
                  );
                }}
                t={t}
              />
            ))}
          </div>
          </div>
        )
      )}

      {/* Details Modal */}
      {detailsSkill && skillWithDetails && (
        <DetailsModal
          skill={skillWithDetails}
          onClose={() => setDetailsSkill(null)}
          onInstall={() => handleInstall(detailsSkill.slug, detailsSource)}
          pendingId={installingId}
          source={detailsSource}
          t={t}
        />
      )}

      {/* Uninstall Dialog */}
      {uninstalling && (
        <UninstallDialog
          skillName={uninstalling}
          onClose={() => setUninstalling(null)}
          onConfirm={confirmUninstall}
          isPending={uninstallMutation.isPending}
        />
      )}

      {/* Create Skill Modal */}
      <CreateSkillModal
        isOpen={showCreateModal}
        onClose={() => setShowCreateModal(false)}
        onCreated={() => {
          queryClient.invalidateQueries({ queryKey: ["skills"] });
          addToast(t("skills.evo_created", { defaultValue: "Skill created successfully" }), "success");
        }}
        t={t}
      />

      {/* Skill Detail Modal */}
      <SkillDetailModal
        skillName={detailSkillName}
        isOpen={!!detailSkillName}
        onClose={() => setDetailSkillName(null)}
        t={t}
      />
    </div>
  );
}
