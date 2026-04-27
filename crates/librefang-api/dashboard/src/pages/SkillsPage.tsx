import { useQuery } from "@tanstack/react-query";
import { formatDate } from "../lib/datetime";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  type ClawHubBrowseItem,
  type FangHubSkill,
  type HandDefinitionItem,
} from "../api";
import {
  useSkills,
  skillQueries,
  useSkillDetail,
  useSupportingFile,
} from "../lib/queries/skills";
import { useHands } from "../lib/queries/hands";
import {
  useUninstallSkill,
  useClawHubInstall,
  useClawHubCnInstall,
  useSkillHubInstall,
  useInstallSkill,
  useCreateSkill,
  useReloadSkills,
  useEvolveUpdateSkill,
  useEvolvePatchSkill,
  useEvolveRollbackSkill,
  useEvolveDeleteSkill,
  useEvolveWriteFile,
  useEvolveRemoveFile,
} from "../lib/mutations/skills";
import { CardSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { Input } from "../components/ui/Input";
import { Modal } from "../components/ui/Modal";
import { ConfirmDialog } from "../components/ui/ConfirmDialog";
import { PageHeader } from "../components/ui/PageHeader";
import { useUIStore } from "../lib/store";
import {
  Wrench,
  Search,
  CheckCircle2,
  X,
  Download,
  Trash2,
  Star,
  Loader2,
  Sparkles,
  Package,
  Code,
  GitBranch,
  Globe,
  Cloud,
  Monitor,
  Bot,
  Database,
  Briefcase,
  Shield,
  Terminal,
  Calendar,
  Store,
  Zap,
  Plus,
  History,
  Eye,
  RotateCcw,
  FileText,
  Tag,
  Edit as EditIcon,
  Upload,
} from "lucide-react";

// ─── Types ───────────────────────────────────────────────────────────────────

type ClawHubSkillWithStatus = ClawHubBrowseItem & { is_installed?: boolean };
type ViewMode = "installed" | "browse";
type MarketplaceSource = "fanghub" | "clawhub" | "clawhub-cn" | "skillhub";

// ─── Constants ───────────────────────────────────────────────────────────────

const CATEGORIES = [
  { id: "coding", nameKey: "skills.cat_coding", icon: <Code className="w-3.5 h-3.5" />, keyword: "python javascript code" },
  { id: "git", nameKey: "skills.cat_git", icon: <GitBranch className="w-3.5 h-3.5" />, keyword: "git github" },
  { id: "web", nameKey: "skills.cat_web", icon: <Globe className="w-3.5 h-3.5" />, keyword: "web frontend html css" },
  { id: "devops", nameKey: "skills.cat_devops", icon: <Cloud className="w-3.5 h-3.5" />, keyword: "devops cloud aws docker kubernetes" },
  { id: "browser", nameKey: "skills.cat_browser", icon: <Monitor className="w-3.5 h-3.5" />, keyword: "browser automation" },
  { id: "ai", nameKey: "skills.cat_ai", icon: <Bot className="w-3.5 h-3.5" />, keyword: "ai llm gpt openai" },
  { id: "data", nameKey: "skills.cat_data", icon: <Database className="w-3.5 h-3.5" />, keyword: "data analytics python" },
  { id: "productivity", nameKey: "skills.cat_productivity", icon: <Briefcase className="w-3.5 h-3.5" />, keyword: "productivity" },
  { id: "security", nameKey: "skills.cat_security", icon: <Shield className="w-3.5 h-3.5" />, keyword: "security" },
  { id: "cli", nameKey: "skills.cat_cli", icon: <Terminal className="w-3.5 h-3.5" />, keyword: "cli bash shell" },
] as const;

// ─── Helpers ─────────────────────────────────────────────────────────────────

function isRateLimitError(err: unknown): boolean {
  if (!err || typeof err !== "object") return false;
  const obj = err as Record<string, unknown>;
  const msg = String(obj.message ?? "").toLowerCase();
  return (
    msg.includes("429") ||
    msg.includes("rate limit") ||
    msg.includes("rate") ||
    obj.status === 429
  );
}

function filterByCategory<
  T extends { name: string; description?: string; tags?: string[] },
>(items: T[], category: string | null): T[] {
  if (!category) return items;
  const kws = (CATEGORIES.find((c) => c.id === category)?.keyword ?? "")
    .toLowerCase()
    .split(" ");
  return items.filter((s) =>
    kws.some(
      (kw) =>
        s.name.toLowerCase().includes(kw) ||
        s.description?.toLowerCase().includes(kw) ||
        s.tags?.some((tag) => tag.toLowerCase().includes(kw)),
    ),
  );
}

// ─── Grid skeleton ────────────────────────────────────────────────────────────

function SkillGridSkeleton({ count = 6 }: { count?: number }) {
  return (
    <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
      {Array.from({ length: count }, (_, i) => (
        <CardSkeleton key={i} />
      ))}
    </div>
  );
}

// ─── Unified Skill Card ───────────────────────────────────────────────────────

type SkillCardVariant = "installed" | "fanghub" | "marketplace";

interface SkillCardProps {
  name: string;
  version?: string;
  description?: string;
  author?: string;
  toolsCount?: number;
  tags?: string[];
  stars?: number;
  downloads?: number;
  updatedAt?: string;
  isInstalled?: boolean;
  variant: SkillCardVariant;
  installPending?: boolean;
  source?: MarketplaceSource;
  onInstall?: () => void;
  onUninstall?: () => void;
  onViewDetail?: () => void;
  t: (key: string, opts?: Record<string, unknown>) => string;
}

function SkillCard({
  name,
  version,
  description,
  author,
  toolsCount,
  tags,
  stars,
  downloads,
  updatedAt,
  isInstalled,
  variant,
  installPending,
  source,
  onInstall,
  onUninstall,
  onViewDetail,
  t,
}: SkillCardProps) {
  const isAccentSource = source === "skillhub" || source === "clawhub-cn";

  const accentClass =
    variant === "installed"
      ? "from-success via-success/60 to-success/30"
      : isAccentSource
        ? "from-accent via-accent/60 to-accent/30"
        : "from-brand via-brand/60 to-brand/30";

  const iconClass =
    variant === "installed"
      ? "bg-success/10 border-success/20 text-success"
      : isAccentSource
        ? "bg-accent/10 border-accent/20 text-accent"
        : "bg-primary/10 border-primary/20 text-primary";

  const hoverTextClass =
    variant === "installed"
      ? "group-hover:text-success"
      : isAccentSource
        ? "group-hover:text-accent"
        : "group-hover:text-primary";

  const icon =
    variant === "installed" ? (
      <Wrench className="w-4 h-4" />
    ) : variant === "fanghub" ? (
      <Zap className="w-4 h-4" />
    ) : source === "skillhub" ? (
      <Store className="w-4 h-4" />
    ) : source === "clawhub-cn" ? (
      <Globe className="w-4 h-4" />
    ) : (
      <Sparkles className="w-4 h-4" />
    );

  return (
    <Card
      hover
      padding="none"
      className={`flex flex-col overflow-hidden group ${onViewDetail ? "cursor-pointer" : ""}`}
      onClick={onViewDetail}
    >
      <div className={`h-1 bg-linear-to-r ${accentClass}`} />
      <div className="p-4 flex-1 flex flex-col gap-3">
        {/* Header */}
        <div className="flex items-start justify-between gap-2">
          <div className="flex items-center gap-2.5 min-w-0">
            <div
              className={`w-8 h-8 shrink-0 rounded-lg flex items-center justify-center border ${iconClass}`}
            >
              {icon}
            </div>
            <div className="min-w-0">
              <h3
                className={`font-bold text-sm truncate transition-colors ${hoverTextClass}`}
              >
                {name}
              </h3>
              <p className="text-[10px] text-text-dim font-mono">
                v{version ?? "1.0.0"}
              </p>
            </div>
          </div>
          {variant === "installed" && (
            <Badge variant="success">{t("skills.installed")}</Badge>
          )}
          {variant !== "installed" && isInstalled && (
            <Badge variant="success">
              <CheckCircle2 className="w-2.5 h-2.5" />
              {t("skills.installed")}
            </Badge>
          )}
        </div>

        {/* Description */}
        <p className="text-xs text-text-dim line-clamp-2 flex-1 italic">
          {description || "—"}
        </p>

        {/* Marketplace stats */}
        {(stars !== undefined || updatedAt) && (
          <div className="flex items-center gap-3 text-[10px] font-bold text-text-dim">
            {stars !== undefined ? (
              <>
                <span className="flex items-center gap-1">
                  <Star className="w-3 h-3 text-warning" />
                  {stars}
                </span>
                {downloads !== undefined && (
                  <span className="flex items-center gap-1">
                    <Download className="w-3 h-3" />
                    {downloads}
                  </span>
                )}
              </>
            ) : (
              <span className="flex items-center gap-1">
                <Calendar className="w-3 h-3" />
                {formatDate(updatedAt!)}
              </span>
            )}
          </div>
        )}

        {/* Installed meta */}
        {variant === "installed" && (author || toolsCount !== undefined) && (
          <div className="flex justify-between text-[10px] font-bold text-text-dim">
            {author && (
              <span>
                {t("skills.author")}: {author}
              </span>
            )}
            {toolsCount !== undefined && (
              <span>
                {t("skills.tools")}: {toolsCount}
              </span>
            )}
          </div>
        )}

        {/* Tags */}
        {tags && tags.length > 0 && (
          <div className="flex flex-wrap gap-1">
            {tags.slice(0, 3).map((tag) => (
              <span
                key={tag}
                className="px-1.5 py-0.5 text-[10px] rounded-full bg-surface-2 text-text-dim font-medium"
              >
                {tag}
              </span>
            ))}
          </div>
        )}

        {/* Actions */}
        {variant === "installed" ? (
          <div
            className="flex gap-2"
            onClick={(e) => e.stopPropagation()}
          >
            {onViewDetail && (
              <Button
                variant="ghost"
                size="sm"
                className="flex-1"
                onClick={onViewDetail}
                leftIcon={<Eye className="w-3.5 h-3.5" />}
              >
                {t("common.detail")}
              </Button>
            )}
            {onUninstall && (
              <Button
                variant="ghost"
                size="sm"
                className="flex-1 text-error hover:text-error"
                onClick={onUninstall}
                leftIcon={<Trash2 className="w-3.5 h-3.5" />}
              >
                {t("skills.uninstall")}
              </Button>
            )}
          </div>
        ) : isInstalled ? (
          <Button variant="secondary" size="sm" disabled className="w-full">
            <CheckCircle2 className="w-3.5 h-3.5 mr-1" />
            {t("skills.installed")}
          </Button>
        ) : onInstall ? (
          <div onClick={(e) => e.stopPropagation()}>
            <Button
              variant="primary"
              size="sm"
              className="w-full"
              onClick={onInstall}
              disabled={installPending}
              leftIcon={
                installPending ? (
                  <Loader2 className="w-3.5 h-3.5 animate-spin" />
                ) : (
                  <Download className="w-3.5 h-3.5" />
                )
              }
            >
              {installPending ? t("skills.installing") : t("skills.install")}
            </Button>
          </div>
        ) : null}
      </div>
    </Card>
  );
}

// ─── Category chips ───────────────────────────────────────────────────────────

function CategoryChips({
  selected,
  onChange,
  t,
}: {
  selected: string | null;
  onChange: (id: string | null) => void;
  t: (key: string) => string;
}) {
  return (
    <div className="flex flex-wrap gap-1.5">
      <button
        onClick={() => onChange(null)}
        className={`flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-bold transition-colors ${
          !selected
            ? "bg-primary text-white shadow-sm"
            : "bg-surface border border-border-subtle text-text-dim hover:text-text-main hover:border-primary/30"
        }`}
      >
        {t("common.all")}
      </button>
      {CATEGORIES.map((cat) => (
        <button
          key={cat.id}
          onClick={() => onChange(selected === cat.id ? null : cat.id)}
          className={`flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-bold transition-colors ${
            selected === cat.id
              ? "bg-primary text-white shadow-sm"
              : "bg-surface border border-border-subtle text-text-dim hover:text-text-main hover:border-primary/30"
          }`}
        >
          {cat.icon}
          {t(cat.nameKey)}
        </button>
      ))}
    </div>
  );
}

// ─── Marketplace detail modal ─────────────────────────────────────────────────

function MarketplaceDetailModal({
  skill,
  source,
  pendingId,
  onClose,
  onInstall,
  t,
}: {
  skill: ClawHubSkillWithStatus;
  source: MarketplaceSource;
  pendingId: string | null;
  onClose: () => void;
  onInstall: () => void;
  t: (key: string, opts?: Record<string, unknown>) => string;
}) {
  const isPending = pendingId === skill.slug;
  return (
    <Modal isOpen onClose={onClose} title={skill.name} size="md" variant="panel-right">
      <div className="p-5 space-y-4">
        <div className="p-4 rounded-xl bg-surface-2">
          <p className="text-sm text-text-dim leading-relaxed">{skill.description}</p>
        </div>

        <div className="flex items-center gap-5 text-xs font-bold text-text-dim">
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
          <div className="flex flex-wrap gap-1.5">
            {skill.tags.map((tag) => (
              <span
                key={tag}
                className={`px-2 py-1 rounded-lg text-xs font-bold ${
                  source === "skillhub"
                    ? "bg-accent/10 text-accent"
                    : "bg-primary/10 text-primary"
                }`}
              >
                {tag}
              </span>
            ))}
          </div>
        )}

        {skill.is_installed ? (
          <Button
            variant="secondary"
            className="w-full"
            disabled
            leftIcon={<CheckCircle2 className="w-4 h-4" />}
          >
            {t("skills.installed")}
          </Button>
        ) : (
          <Button
            variant="primary"
            className="w-full"
            onClick={onInstall}
            disabled={isPending}
            leftIcon={
              isPending ? (
                <Loader2 className="w-4 h-4 animate-spin" />
              ) : (
                <Download className="w-4 h-4" />
              )
            }
          >
            {isPending ? t("skills.installing") : t("skills.install")}
          </Button>
        )}
      </div>
    </Modal>
  );
}

// ─── Create Skill Modal ───────────────────────────────────────────────────────

function CreateSkillModal({
  isOpen,
  onClose,
  onCreated,
  t,
}: {
  isOpen: boolean;
  onClose: () => void;
  onCreated: () => void;
  t: (key: string, opts?: Record<string, unknown>) => string;
}) {
  const createSkillMutation = useCreateSkill();
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [promptContext, setPromptContext] = useState("");
  const [tags, setTags] = useState("");
  const [error, setError] = useState("");
  const [creating, setCreating] = useState(false);
  const mountedRef = useRef(true);
  const abortRef = useRef<AbortController | null>(null);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      abortRef.current?.abort();
    };
  }, []);

  useEffect(() => {
    if (!isOpen) {
      setError("");
      setCreating(false);
    }
  }, [isOpen]);

  const formatApiError = useCallback(
    (e: unknown): string => {
      const msg = (e instanceof Error ? e.message : String(e)).toLowerCase();
      if (msg.includes("already installed") || msg.includes("already exists"))
        return t("skills.err_name_conflict", {
          defaultValue: "A skill with this name already exists.",
        });
      if (msg.includes("description too long"))
        return t("skills.err_desc_too_long", {
          defaultValue: "Description is too long (max 1024 characters).",
        });
      if (msg.includes("prompt context too large"))
        return t("skills.err_prompt_too_large", {
          defaultValue: "Prompt context is too large (max 160,000 characters).",
        });
      if (msg.includes("security") || msg.includes("blocked"))
        return t("skills.err_security_blocked", {
          defaultValue: "Content was blocked by security scan.",
        });
      if (msg.includes("invalid") && msg.includes("name"))
        return t("skills.err_invalid_name", {
          defaultValue:
            "Invalid skill name. Use lowercase letters, numbers, hyphens only.",
        });
      return (
        (e instanceof Error ? e.message : String(e)) ||
        t("skills.err_create_failed", { defaultValue: "Failed to create skill." })
      );
    },
    [t],
  );

  const handleCreate = async () => {
    setError("");
    if (!name.trim() || !description.trim()) {
      setError(
        t("skills.evo_fill_required", {
          defaultValue: "Name and description are required",
        }),
      );
      return;
    }
    abortRef.current?.abort();
    const controller = new AbortController();
    abortRef.current = controller;
    setCreating(true);
    try {
      await createSkillMutation.mutateAsync({
        name: name.trim(),
        description: description.trim(),
        prompt_context: promptContext.trim(),
        tags: tags
          .split(",")
          .map((s) => s.trim())
          .filter(Boolean),
      });
      if (mountedRef.current && !controller.signal.aborted) {
        onCreated();
        onClose();
        setName("");
        setDescription("");
        setPromptContext("");
        setTags("");
      }
    } catch (e: unknown) {
      if (e instanceof DOMException && e.name === "AbortError") return;
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
    <Modal
      isOpen={isOpen}
      onClose={onClose}
      title={t("skills.evo_create_title", { defaultValue: "Create Skill" })}
      size="xl"
      variant="panel-right"
    >
      <div className="space-y-4 p-1">
        <div>
          <label className="block text-xs font-bold uppercase text-text-dim mb-1">
            {t("common.name")}
          </label>
          <Input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="my-skill-name"
          />
          <p className="text-[10px] text-text-dim mt-1">
            {t("skills.evo_name_hint", {
              defaultValue: "Lowercase, hyphens allowed (e.g., csv-analysis)",
            })}
          </p>
        </div>
        <div>
          <label className="block text-xs font-bold uppercase text-text-dim mb-1">
            {t("common.description")}
          </label>
          <Input
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            placeholder={t("skills.evo_desc_placeholder", {
              defaultValue: "What this skill teaches agents to do",
            })}
          />
        </div>
        <div>
          <label className="block text-xs font-bold uppercase text-text-dim mb-1">
            {t("skills.evo_prompt_context", {
              defaultValue: "Prompt Context (Markdown)",
            })}
          </label>
          <textarea
            value={promptContext}
            onChange={(e) => setPromptContext(e.target.value)}
            className="w-full h-48 px-3 py-2 text-sm rounded-lg bg-surface-2 border border-border text-text-main resize-y font-mono"
            placeholder={t("skills.evo_prompt_placeholder", {
              defaultValue:
                "# Skill Instructions\n\nMarkdown instructions injected into the system prompt...",
            })}
          />
          <p className="text-[10px] text-text-dim mt-1">
            {promptContext.length.toLocaleString()} / 160,000
          </p>
        </div>
        <div>
          <label className="block text-xs font-bold uppercase text-text-dim mb-1">
            {t("skills.evo_tags", { defaultValue: "Tags (comma-separated)" })}
          </label>
          <Input
            value={tags}
            onChange={(e) => setTags(e.target.value)}
            placeholder="data, csv, analysis"
          />
        </div>
        {error && <p className="text-xs text-error">{error}</p>}
        <div className="flex justify-end gap-2 pt-2">
          <Button variant="ghost" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button
            onClick={handleCreate}
            disabled={creating}
            leftIcon={
              creating ? (
                <Loader2 className="w-4 h-4 animate-spin" />
              ) : (
                <Plus className="w-4 h-4" />
              )
            }
          >
            {creating
              ? t("common.creating", { defaultValue: "Creating..." })
              : t("common.create")}
          </Button>
        </div>
      </div>
    </Modal>
  );
}

// ─── Evolve sub-panes ─────────────────────────────────────────────────────────

function EvolveUpdatePane({
  skillName,
  initialContent,
  onSubmit,
  onCancel,
  busy,
  t,
}: {
  skillName: string;
  initialContent: string;
  onSubmit: (params: { prompt_context: string; changelog: string }) => void;
  onCancel: () => void;
  busy: boolean;
  t: (key: string, opts?: Record<string, unknown>) => string;
}) {
  const [content, setContent] = useState(initialContent);
  const [changelog, setChangelog] = useState("");
  const dirty = content !== initialContent;
  return (
    <div className="rounded-lg border border-border bg-surface-1 p-3 space-y-2">
      <p className="text-xs font-bold uppercase text-text-dim">
        {t("skills.evo_update_title", {
          defaultValue: "Update {{name}}",
          name: skillName,
        })}
      </p>
      <textarea
        value={content}
        onChange={(e) => setContent(e.target.value)}
        className="w-full h-64 px-3 py-2 text-sm rounded-lg bg-surface-2 border border-border text-text-main resize-y font-mono"
      />
      <p className="text-[10px] text-text-dim">
        {content.length.toLocaleString()} / 160,000
      </p>
      <Input
        value={changelog}
        onChange={(e) => setChangelog(e.target.value)}
        placeholder={t("skills.evo_changelog_placeholder", {
          defaultValue: "What changed and why",
        })}
      />
      <div className="flex justify-end gap-2">
        <Button variant="ghost" onClick={onCancel} disabled={busy}>
          {t("common.cancel")}
        </Button>
        <Button
          onClick={() =>
            onSubmit({ prompt_context: content, changelog: changelog.trim() })
          }
          disabled={busy || !dirty || !changelog.trim()}
          leftIcon={
            busy ? (
              <Loader2 className="w-4 h-4 animate-spin" />
            ) : (
              <EditIcon className="w-4 h-4" />
            )
          }
        >
          {t("skills.evo_update", { defaultValue: "Update" })}
        </Button>
      </div>
    </div>
  );
}

function EvolvePatchPane({
  skillName,
  onSubmit,
  onCancel,
  busy,
  t,
}: {
  skillName: string;
  onSubmit: (params: {
    old_string: string;
    new_string: string;
    changelog: string;
    replace_all: boolean;
  }) => void;
  onCancel: () => void;
  busy: boolean;
  t: (key: string, opts?: Record<string, unknown>) => string;
}) {
  const [oldStr, setOldStr] = useState("");
  const [newStr, setNewStr] = useState("");
  const [changelog, setChangelog] = useState("");
  const [replaceAll, setReplaceAll] = useState(false);
  return (
    <div className="rounded-lg border border-border bg-surface-1 p-3 space-y-2">
      <p className="text-xs font-bold uppercase text-text-dim">
        {t("skills.evo_patch_title", {
          defaultValue: "Patch {{name}}",
          name: skillName,
        })}
      </p>
      <div className="grid grid-cols-1 md:grid-cols-2 gap-2">
        <div>
          <label className="block text-[10px] font-bold uppercase text-text-dim mb-1">
            {t("skills.evo_patch_old", { defaultValue: "Find" })}
          </label>
          <textarea
            value={oldStr}
            onChange={(e) => setOldStr(e.target.value)}
            className="w-full h-40 px-2 py-1.5 text-xs rounded bg-surface-2 border border-border text-text-main resize-y font-mono"
          />
        </div>
        <div>
          <label className="block text-[10px] font-bold uppercase text-text-dim mb-1">
            {t("skills.evo_patch_new", { defaultValue: "Replace with" })}
          </label>
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
        placeholder={t("skills.evo_changelog_placeholder", {
          defaultValue: "What changed and why",
        })}
      />
      <label className="inline-flex items-center gap-2 text-xs text-text-dim">
        <input
          type="checkbox"
          checked={replaceAll}
          onChange={(e) => setReplaceAll(e.target.checked)}
        />
        {t("skills.evo_replace_all", {
          defaultValue: "Replace all occurrences",
        })}
      </label>
      <div className="flex justify-end gap-2">
        <Button variant="ghost" onClick={onCancel} disabled={busy}>
          {t("common.cancel")}
        </Button>
        <Button
          onClick={() =>
            onSubmit({
              old_string: oldStr,
              new_string: newStr,
              changelog: changelog.trim(),
              replace_all: replaceAll,
            })
          }
          disabled={busy || !oldStr || !changelog.trim()}
          leftIcon={
            busy ? (
              <Loader2 className="w-4 h-4 animate-spin" />
            ) : (
              <Code className="w-4 h-4" />
            )
          }
        >
          {t("skills.evo_patch", { defaultValue: "Patch" })}
        </Button>
      </div>
    </div>
  );
}

function EvolveUploadPane({
  skillName,
  onSubmit,
  onCancel,
  busy,
  t,
}: {
  skillName: string;
  onSubmit: (params: { path: string; content: string }) => void;
  onCancel: () => void;
  busy: boolean;
  t: (key: string, opts?: Record<string, unknown>) => string;
}) {
  const [subdir, setSubdir] = useState("references");
  const [filename, setFilename] = useState("");
  const [content, setContent] = useState("");
  const fileInputRef = useRef<HTMLInputElement | null>(null);

  const handleFilePick = async (file: File) => {
    if (file.size > 1024 * 1024) {
      alert(
        t("skills.evo_file_too_large", { defaultValue: "File exceeds 1 MiB limit" }),
      );
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
        {t("skills.evo_upload_title", {
          defaultValue: "Add file to {{name}}",
          name: skillName,
        })}
      </p>
      <div className="grid grid-cols-3 gap-2">
        <div>
          <label className="block text-[10px] font-bold uppercase text-text-dim mb-1">
            {t("skills.evo_folder", { defaultValue: "Folder" })}
          </label>
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
          <label className="block text-[10px] font-bold uppercase text-text-dim mb-1">
            {t("skills.evo_filename", { defaultValue: "Filename" })}
          </label>
          <Input
            value={filename}
            onChange={(e) => setFilename(e.target.value)}
            placeholder="example.md"
          />
        </div>
      </div>
      <div>
        <label className="block text-[10px] font-bold uppercase text-text-dim mb-1">
          {t("skills.evo_content", { defaultValue: "Content" })}
        </label>
        <textarea
          value={content}
          onChange={(e) => setContent(e.target.value)}
          className="w-full h-40 px-2 py-1.5 text-xs rounded bg-surface-2 border border-border text-text-main resize-y font-mono"
          placeholder={t("skills.evo_content_placeholder", {
            defaultValue: "Paste file content or load from disk below",
          })}
        />
        <div className="flex items-center gap-2 mt-1">
          <input
            ref={fileInputRef}
            type="file"
            className="hidden"
            onChange={(e) => {
              const f = e.target.files?.[0];
              if (f) void handleFilePick(f);
            }}
          />
          <Button
            variant="ghost"
            onClick={() => fileInputRef.current?.click()}
            leftIcon={<Upload className="w-3 h-3" />}
          >
            {t("skills.evo_load_from_disk", { defaultValue: "Load from disk" })}
          </Button>
          <span className="text-[10px] text-text-dim">
            {content.length.toLocaleString()} chars
          </span>
        </div>
      </div>
      {path && (
        <p className="text-[10px] text-text-dim font-mono">→ {path}</p>
      )}
      <div className="flex justify-end gap-2">
        <Button variant="ghost" onClick={onCancel} disabled={busy}>
          {t("common.cancel")}
        </Button>
        <Button
          onClick={() => onSubmit({ path, content })}
          disabled={busy || !filename.trim() || !content}
          leftIcon={
            busy ? (
              <Loader2 className="w-4 h-4 animate-spin" />
            ) : (
              <Upload className="w-4 h-4" />
            )
          }
        >
          {t("skills.evo_upload", { defaultValue: "Upload" })}
        </Button>
      </div>
    </div>
  );
}

function SupportingFileViewer({
  skillName,
  path,
  onClose,
  t,
}: {
  skillName: string;
  path: string;
  onClose: () => void;
  t: (key: string, opts?: Record<string, unknown>) => string;
}) {
  const { data, isLoading, error } = useSupportingFile(skillName, path, {
    enabled: !!skillName && !!path,
  });
  return (
    <div className="rounded-lg border border-border bg-surface-1 p-3 space-y-2">
      <div className="flex items-center justify-between">
        <p className="text-xs font-bold text-text-dim font-mono">{path}</p>
        <button className="text-text-dim hover:text-text-main" onClick={onClose}>
          <X className="w-4 h-4" />
        </button>
      </div>
      {isLoading && (
        <div className="flex items-center justify-center py-6">
          <Loader2 className="w-5 h-5 animate-spin text-text-dim" />
        </div>
      )}
      {error && (
        <p className="text-xs text-error">
          {error instanceof Error ? error.message : "Failed to load file"}
        </p>
      )}
      {data && (
        <>
          <pre className="max-h-80 overflow-auto whitespace-pre-wrap break-all text-[11px] bg-surface-2 p-2 rounded font-mono">
            {data.content}
          </pre>
          {data.truncated && (
            <p className="text-[10px] text-text-dim">
              {t("skills.evo_file_truncated", {
                defaultValue: "File truncated to 256 KiB preview",
              })}
            </p>
          )}
        </>
      )}
    </div>
  );
}

// ─── Skill Detail Modal (installed skills + evolve) ───────────────────────────

type EvolvePane = "none" | "update" | "patch" | "upload";

function SkillDetailModal({
  skillName,
  isOpen,
  onClose,
  t,
}: {
  skillName: string | null;
  isOpen: boolean;
  onClose: () => void;
  t: (key: string, opts?: Record<string, unknown>) => string;
}) {
  const { addToast } = useUIStore();
  const { data: detail, isLoading, refetch } = useSkillDetail(skillName ?? "", {
    enabled: isOpen && !!skillName,
  });

  const rollbackMutation = useEvolveRollbackSkill();
  const removeFileMutation = useEvolveRemoveFile();
  const deleteSkillMutation = useEvolveDeleteSkill();
  const updateSkillMutation = useEvolveUpdateSkill();
  const patchSkillMutation = useEvolvePatchSkill();
  const writeFileMutation = useEvolveWriteFile();

  const [pane, setPane] = useState<EvolvePane>("none");
  const [viewingFile, setViewingFile] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!isOpen) {
      setPane("none");
      setViewingFile(null);
    }
  }, [isOpen, skillName]);

  const runMutation = async <T,>(fn: () => Promise<T>, successMsg: string) => {
    if (!skillName) return;
    setBusy(true);
    try {
      await fn();
      await refetch();
      addToast(successMsg, "success");
      setPane("none");
    } catch (e: unknown) {
      addToast(e instanceof Error ? e.message : "Action failed", "error");
    } finally {
      setBusy(false);
    }
  };

  const handleRollback = () => {
    if (!skillName) return;
    if (
      !confirm(
        t("skills.evo_rollback_confirm", {
          defaultValue:
            "Roll back to the previous version? This cannot be undone unless you patch again.",
        }),
      )
    )
      return;
    void runMutation(
      () => rollbackMutation.mutateAsync({ name: skillName }),
      t("skills.evo_rolled_back", { defaultValue: "Skill rolled back" }),
    );
  };

  const handleRemoveFile = (path: string) => {
    if (!skillName) return;
    if (
      !confirm(
        t("skills.evo_remove_file_confirm", {
          defaultValue: `Remove ${path}?`,
          path,
        }),
      )
    )
      return;
    void runMutation(
      () => removeFileMutation.mutateAsync({ name: skillName, path }),
      t("skills.evo_file_removed", { defaultValue: "File removed" }),
    );
  };

  const handleDelete = () => {
    if (!skillName) return;
    if (
      !confirm(
        t("skills.evo_delete_confirm", {
          defaultValue: `Permanently delete ${skillName}? This cannot be undone.`,
          name: skillName,
        }),
      )
    )
      return;
    (async () => {
      setBusy(true);
      try {
        await deleteSkillMutation.mutateAsync({ name: skillName });
        addToast(
          t("skills.evo_deleted", { defaultValue: "Skill deleted" }),
          "success",
        );
        onClose();
      } catch (e: unknown) {
        addToast(e instanceof Error ? e.message : "Delete failed", "error");
      } finally {
        setBusy(false);
      }
    })();
  };

  return (
    <Modal
      isOpen={isOpen}
      onClose={onClose}
      title={detail?.name ?? skillName ?? ""}
      size="xl"
      variant="drawer-right"
    >
      {isLoading ? (
        <div className="flex items-center justify-center py-12">
          <Loader2 className="w-6 h-6 animate-spin text-text-dim" />
        </div>
      ) : detail ? (
        <div className="space-y-5 p-1">
          {/* Meta */}
          <div>
            <p className="text-sm text-text-dim italic">{detail.description}</p>
            <div className="flex flex-wrap gap-1.5 mt-2">
              <Badge variant="default">v{detail.version}</Badge>
              <Badge variant="default">{detail.runtime}</Badge>
              {detail.tags.map((tag) => (
                <Badge key={tag} variant="default">
                  <Tag className="w-3 h-3" />
                  {tag}
                </Badge>
              ))}
            </div>
          </div>

          {/* Evolve toolbar */}
          <div className="flex flex-wrap gap-2">
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setPane(pane === "update" ? "none" : "update")}
              leftIcon={<EditIcon className="w-3.5 h-3.5" />}
              disabled={busy}
            >
              {t("skills.evo_update", { defaultValue: "Update" })}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setPane(pane === "patch" ? "none" : "patch")}
              leftIcon={<Code className="w-3.5 h-3.5" />}
              disabled={busy}
            >
              {t("skills.evo_patch", { defaultValue: "Patch" })}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setPane(pane === "upload" ? "none" : "upload")}
              leftIcon={<Upload className="w-3.5 h-3.5" />}
              disabled={busy}
            >
              {t("skills.evo_add_file", { defaultValue: "Add File" })}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={handleRollback}
              leftIcon={<RotateCcw className="w-3.5 h-3.5" />}
              disabled={busy || detail.evolution.versions.length < 1}
              title={
                detail.evolution.versions.length < 1
                  ? t("skills.evo_no_rollback", {
                      defaultValue: "No prior version to roll back to",
                    })
                  : ""
              }
            >
              {t("skills.evo_rollback", { defaultValue: "Rollback" })}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              className="text-error hover:text-error ml-auto"
              onClick={handleDelete}
              leftIcon={<Trash2 className="w-3.5 h-3.5" />}
              disabled={busy}
            >
              {t("skills.evo_delete", { defaultValue: "Delete" })}
            </Button>
          </div>

          {/* Inline panes */}
          {pane === "update" && skillName && (
            <EvolveUpdatePane
              skillName={skillName}
              initialContent={detail.prompt_context ?? ""}
              onSubmit={(params) =>
                runMutation(
                  () =>
                    updateSkillMutation.mutateAsync({ name: skillName, params }),
                  t("skills.evo_updated", { defaultValue: "Skill updated" }),
                )
              }
              onCancel={() => setPane("none")}
              busy={busy}
              t={t}
            />
          )}
          {pane === "patch" && skillName && (
            <EvolvePatchPane
              skillName={skillName}
              onSubmit={(params) =>
                runMutation(
                  () =>
                    patchSkillMutation.mutateAsync({ name: skillName, params }),
                  t("skills.evo_patched", { defaultValue: "Skill patched" }),
                )
              }
              onCancel={() => setPane("none")}
              busy={busy}
              t={t}
            />
          )}
          {pane === "upload" && skillName && (
            <EvolveUploadPane
              skillName={skillName}
              onSubmit={(params) =>
                runMutation(
                  () =>
                    writeFileMutation.mutateAsync({ name: skillName, params }),
                  t("skills.evo_file_uploaded", {
                    defaultValue: "File uploaded",
                  }),
                )
              }
              onCancel={() => setPane("none")}
              busy={busy}
              t={t}
            />
          )}

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
              <p className="text-[10px] font-bold uppercase text-text-dim">
                {t("skills.tools")}
              </p>
            </div>
            <div className="p-3 rounded-lg bg-surface-2 text-center">
              <p className="text-2xl font-black">{detail.evolution.use_count}</p>
              <p className="text-[10px] font-bold uppercase text-text-dim">
                {t("skills.evo_uses", { defaultValue: "Uses" })}
              </p>
            </div>
            <div className="p-3 rounded-lg bg-surface-2 text-center">
              <p className="text-2xl font-black">
                {detail.evolution.evolution_count}
              </p>
              <p className="text-[10px] font-bold uppercase text-text-dim">
                {t("skills.evo_evolutions", { defaultValue: "Evolutions" })}
              </p>
            </div>
          </div>

          {/* Tools list */}
          {detail.tools.length > 0 && (
            <div>
              <h3 className="text-xs font-bold uppercase text-text-dim mb-2">
                <Wrench className="w-3 h-3 inline mr-1" />
                {t("skills.tools")}
              </h3>
              <div className="space-y-1">
                {detail.tools.map((tool) => (
                  <div
                    key={tool.name}
                    className="px-3 py-2 rounded bg-surface-2 text-xs"
                  >
                    <span className="font-mono font-bold">{tool.name}</span>
                    <span className="text-text-dim ml-2">{tool.description}</span>
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* Supporting files */}
          {Object.keys(detail.linked_files).length > 0 && (
            <div>
              <h3 className="text-xs font-bold uppercase text-text-dim mb-2">
                <FileText className="w-3 h-3 inline mr-1" />
                {t("skills.evo_files", { defaultValue: "Supporting Files" })}
              </h3>
              {Object.entries(detail.linked_files).map(([dir, files]) => (
                <div key={dir} className="mb-2">
                  <p className="text-[10px] font-bold uppercase text-text-dim mb-1">
                    {dir}/
                  </p>
                  <div className="flex flex-wrap gap-1">
                    {files.map((f) => {
                      const rel = `${dir}/${f}`;
                      return (
                        <span
                          key={f}
                          className="inline-flex items-center gap-1 px-2 py-0.5 rounded bg-surface-2 text-xs font-mono group"
                        >
                          <button
                            className="hover:text-primary"
                            onClick={() => setViewingFile(rel)}
                          >
                            {f}
                          </button>
                          <button
                            className="opacity-0 group-hover:opacity-100 transition-opacity text-error"
                            onClick={() => handleRemoveFile(rel)}
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

          {/* Version history */}
          {detail.evolution.versions.length > 0 && (
            <div>
              <h3 className="text-xs font-bold uppercase text-text-dim mb-2">
                <History className="w-3 h-3 inline mr-1" />
                {t("skills.evo_history", { defaultValue: "Version History" })}
              </h3>
              <div className="space-y-2 max-h-48 overflow-y-auto">
                {[...detail.evolution.versions].reverse().map((v, i) => (
                  <div
                    key={i}
                    className="flex items-start gap-3 px-3 py-2 rounded bg-surface-2 text-xs"
                  >
                    <Badge variant={i === 0 ? "success" : "default"}>
                      v{v.version}
                    </Badge>
                    <div className="flex-1 min-w-0">
                      <p className="text-text-main">{v.changelog}</p>
                      <p className="text-[10px] text-text-dim mt-0.5">
                        {new Date(v.timestamp).toLocaleString()}
                        {v.author && (
                          <span className="ml-2 font-mono">· {v.author}</span>
                        )}
                      </p>
                    </div>
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* Footer meta */}
          <div className="text-[10px] text-text-dim space-y-0.5 pt-2 border-t border-border">
            <p>
              {t("skills.author")}: {detail.author || "—"}
            </p>
            <p>
              {t("skills.evo_prompt_size", { defaultValue: "Prompt context" })}:{" "}
              {detail.prompt_context_length.toLocaleString()} chars
            </p>
            <p className="font-mono truncate">{detail.path}</p>
          </div>
        </div>
      ) : (
        <p className="text-sm text-text-dim py-8 text-center">
          {t("skills.evo_not_found", { defaultValue: "Skill not found" })}
        </p>
      )}
    </Modal>
  );
}

// ─── Main Page ────────────────────────────────────────────────────────────────

export function SkillsPage() {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);

  const [viewMode, setViewMode] = useState<ViewMode>("browse");
  const [source, setSource] = useState<MarketplaceSource>("fanghub");
  const [selectedCategory, setSelectedCategory] = useState<string | null>(null);
  const [search, setSearch] = useState("");

  const [uninstalling, setUninstalling] = useState<string | null>(null);
  const [detailsSkill, setDetailsSkill] = useState<ClawHubSkillWithStatus | null>(null);
  const [detailsSource, setDetailsSource] = useState<MarketplaceSource>("clawhub");
  const [installingId, setInstallingId] = useState<string | null>(null);
  const [targetHand, setTargetHand] = useState("");
  const [showCreateModal, setShowCreateModal] = useState(false);
  const [detailSkillName, setDetailSkillName] = useState<string | null>(null);

  const reloadSkillsMutation = useReloadSkills();
  const handsQuery = useHands();
  const hands = handsQuery.data ?? [];

  // ── Queries ──────────────────────────────────────────────────────────────

  const skillsQuery = useSkills();
  const installedSkills = skillsQuery.data ?? [];

  const keyword = selectedCategory
    ? (CATEGORIES.find((c) => c.id === selectedCategory)?.keyword ?? "")
    : search;

  const clawhubQuery = useQuery({
    ...skillQueries.clawhubSearch(keyword || "python"),
    enabled: viewMode === "browse" && source === "clawhub",
  });

  const clawhubCnQuery = useQuery({
    ...skillQueries.clawhubCnSearch(keyword || "python"),
    enabled: viewMode === "browse" && source === "clawhub-cn",
  });

  const skillhubBrowseQuery = useQuery({
    ...skillQueries.skillhubBrowse(),
    enabled: viewMode === "browse" && source === "skillhub" && !keyword,
  });
  const skillhubSearchQuery = useQuery({
    ...skillQueries.skillhubSearch(keyword),
    enabled: viewMode === "browse" && source === "skillhub" && !!keyword,
  });
  const activeSkillhubQuery = keyword ? skillhubSearchQuery : skillhubBrowseQuery;

  const fanghubQuery = useQuery({
    ...skillQueries.fanghubList(),
    enabled: viewMode === "browse" && source === "fanghub",
  });

  const clawhubDetailQuery = useQuery({
    ...skillQueries.clawhubSkill(detailsSkill?.slug ?? ""),
    enabled: !!detailsSkill?.slug && detailsSource === "clawhub",
  });
  const clawhubCnDetailQuery = useQuery({
    ...skillQueries.clawhubCnSkill(detailsSkill?.slug ?? ""),
    enabled: !!detailsSkill?.slug && detailsSource === "clawhub-cn",
  });
  const skillhubDetailQuery = useQuery({
    ...skillQueries.skillhubSkill(detailsSkill?.slug ?? ""),
    enabled: !!detailsSkill?.slug && detailsSource === "skillhub",
  });
  const detailQuery =
    detailsSource === "skillhub"
      ? skillhubDetailQuery
      : detailsSource === "clawhub-cn"
        ? clawhubCnDetailQuery
        : clawhubDetailQuery;

  const skillWithDetails =
    detailQuery.data && detailsSkill
      ? ({
          ...detailsSkill,
          ...detailQuery.data,
          is_installed:
            detailQuery.data.is_installed ?? detailQuery.data.installed,
        } as ClawHubSkillWithStatus)
      : detailsSkill;

  // ── Filtered data ─────────────────────────────────────────────────────────

  const isInstalledFromMarketplace = useCallback(
    (slug: string, src: MarketplaceSource) => {
      // clawhub and clawhub-cn share the same slug namespace (same content)
      const matchTypes =
        src === "clawhub" || src === "clawhub-cn"
          ? ["clawhub", "clawhub-cn"]
          : [src];
      return installedSkills.some(
        (s) => matchTypes.includes(s.source?.type ?? "") && s.source?.slug === slug,
      );
    },
    [installedSkills],
  );

  const browseItems = useMemo(() => {
    if (source === "fanghub") return filterByCategory(fanghubQuery.data?.skills ?? [], selectedCategory);
    if (source === "clawhub") {
      return (clawhubQuery.data?.items ?? [])
        .map((s) => ({ ...s, is_installed: isInstalledFromMarketplace(s.slug, "clawhub") }))
        .filter((s) => !search || s.name.toLowerCase().includes(search.toLowerCase()) || s.description?.toLowerCase().includes(search.toLowerCase()));
    }
    if (source === "clawhub-cn") {
      return (clawhubCnQuery.data?.items ?? [])
        .map((s) => ({ ...s, is_installed: isInstalledFromMarketplace(s.slug, "clawhub-cn") }))
        .filter((s) => !search || s.name.toLowerCase().includes(search.toLowerCase()) || s.description?.toLowerCase().includes(search.toLowerCase()));
    }
    // skillhub
    return filterByCategory(
      (activeSkillhubQuery.data?.items ?? []).map((s) => ({
        ...s,
        is_installed: isInstalledFromMarketplace(s.slug, "skillhub"),
      })),
      selectedCategory,
    );
  }, [source, fanghubQuery.data, clawhubQuery.data, clawhubCnQuery.data, activeSkillhubQuery.data, isInstalledFromMarketplace, search, selectedCategory]);

  const isAnyFetching =
    skillsQuery.isFetching ||
    clawhubQuery.isFetching ||
    clawhubCnQuery.isFetching ||
    skillhubBrowseQuery.isFetching ||
    skillhubSearchQuery.isFetching ||
    fanghubQuery.isFetching;

  // ── Mutations ─────────────────────────────────────────────────────────────

  const uninstallMutation = useUninstallSkill();
  const installMutation = useClawHubInstall();
  const clawhubCnInstallMutation = useClawHubCnInstall();
  const skillhubInstallMutation = useSkillHubInstall();
  const fanghubInstallMutation = useInstallSkill();

  const handleInstall = (
    slug: string,
    src: MarketplaceSource | "fanghub" = source,
  ) => {
    setInstallingId(slug);
    const hand = targetHand || undefined;
    const opts = {
      onSuccess: () => {
        addToast(t("common.success"), "success");
        setInstallingId(null);
        setDetailsSkill(null);
      },
      onError: (error: unknown) => {
        const msg = error instanceof Error ? error.message : String(error);
        addToast(
          msg.includes("abort") ? t("skills.install_timeout") : msg,
          "error",
        );
        setInstallingId(null);
      },
    };
    if (src === "skillhub")
      skillhubInstallMutation.mutate({ slug, hand }, opts);
    else if (src === "clawhub-cn")
      clawhubCnInstallMutation.mutate({ slug, hand }, opts);
    else if (src === "fanghub")
      fanghubInstallMutation.mutate({ name: slug, hand }, opts);
    else installMutation.mutate({ slug, hand }, opts);
  };

  const handleReload = async () => {
    try {
      const res = await reloadSkillsMutation.mutateAsync();
      addToast(
        t("skills.reloaded", {
          defaultValue: "Rescanned skills directory ({{count}} loaded)",
          count: (res as { count?: number }).count ?? 0,
        }),
        "success",
      );
    } catch (e: unknown) {
      addToast(e instanceof Error ? e.message : "Reload failed", "error");
    }
    void skillsQuery.refetch();
    void clawhubQuery.refetch();
    void clawhubCnQuery.refetch();
    void activeSkillhubQuery.refetch();
  };

  // ── Tab helpers ───────────────────────────────────────────────────────────

  const switchTab = (mode: ViewMode) => {
    setViewMode(mode);
    setSearch("");
    setSelectedCategory(null);
  };

  const showCategories = viewMode === "browse";

  // ── Render ────────────────────────────────────────────────────────────────

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        icon={<Wrench className="h-4 w-4" />}
        badge=""
        title={t("skills.title")}
        subtitle={t("skills.subtitle")}
        isFetching={isAnyFetching}
        onRefresh={handleReload}
        actions={
          <>
            <span className="hidden sm:inline-block px-2.5 py-1 rounded-full border border-border-subtle bg-surface text-[10px] font-bold uppercase text-text-dim">
              {t("skills.installed_count", { count: installedSkills.length })}
            </span>
            <select
              value={source}
              onChange={(e) => {
                setSource(e.target.value as MarketplaceSource);
                setSearch("");
                setSelectedCategory(null);
                setViewMode("browse");
              }}
              disabled={viewMode === "installed"}
              className="h-8 rounded-xl border border-border-subtle bg-surface px-2 text-xs font-bold text-text-main cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed"
            >
              <option value="fanghub">{t("skills.source_fanghub", { defaultValue: "FangHub" })}</option>
              <option value="clawhub">{t("skills.source_clawhub", { defaultValue: "ClawHub" })}</option>
              <option value="clawhub-cn">{t("skills.source_clawhub_cn", { defaultValue: "ClawHub CN" })}</option>
              <option value="skillhub">{t("skills.source_skillhub", { defaultValue: "SkillHub" })}</option>
            </select>
            <a
              href="https://librefang.ai/skills"
              target="_blank"
              rel="noopener noreferrer"
              className="hidden md:flex h-8 items-center gap-1.5 rounded-xl border border-border-subtle bg-surface px-3 text-xs font-bold text-text-dim hover:text-primary hover:border-primary/30 transition-colors"
            >
              <Globe className="h-3.5 w-3.5" />
              <span>{t("skills.browse_registry", { defaultValue: "Registry" })}</span>
            </a>
            <button
              className="flex h-8 items-center gap-1.5 rounded-xl border border-primary/30 bg-primary/10 px-3 text-xs font-bold text-primary hover:bg-primary/20 transition-colors"
              onClick={() => setShowCreateModal(true)}
            >
              <Plus className="h-3.5 w-3.5" />
              <span className="hidden sm:inline">
                {t("skills.evo_create", { defaultValue: "Create Skill" })}
              </span>
            </button>
          </>
        }
      />

      {/* Tab bar */}
      <div className="flex gap-1 p-1 bg-surface rounded-xl border border-border-subtle w-fit">
        {(
          [
            {
              mode: "installed" as const,
              icon: <Package className="w-4 h-4" />,
              label: t("skills.installed"),
              count: installedSkills.length,
              activeColor: "text-success",
            },
            {
              mode: "browse" as const,
              icon: <Sparkles className="w-4 h-4" />,
              label: t("skills.browse", { defaultValue: "Browse" }),
              activeColor: "text-primary",
            },
          ]
        ).map((tab) => {
          const active = viewMode === tab.mode;
          return (
            <button
              key={tab.mode}
              onClick={() => switchTab(tab.mode)}
              className={`relative flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-bold transition-colors ${
                active
                  ? `bg-surface-hover ${tab.activeColor} shadow-sm`
                  : "text-text-dim hover:text-text-main"
              }`}
            >
              {tab.icon}
              {tab.label}
              {"count" in tab && (
                <span
                  className={`ml-0.5 px-1.5 py-0.5 rounded-full text-[10px] ${
                    active
                      ? "bg-success/20 text-success"
                      : "bg-border-subtle text-text-dim"
                  }`}
                >
                  {tab.count}
                </span>
              )}
            </button>
          );
        })}
      </div>

      {/* Hand target selector */}
      {viewMode === "browse" && hands.length > 0 && (
        <div className="flex items-center gap-2">
          <span className="text-[11px] font-bold text-text-dim">
            {t("skills.install_to")}:
          </span>
          <select
            value={targetHand}
            onChange={(e) => setTargetHand(e.target.value)}
            className="rounded-lg border border-border-subtle bg-surface px-3 py-1.5 text-xs font-bold text-text-main"
          >
            <option value="">{t("skills.global")}</option>
            {hands.map((h: HandDefinitionItem) => (
              <option key={h.id} value={h.id}>
                {h.name || h.id}
              </option>
            ))}
          </select>
        </div>
      )}

      {/* Category chips */}
      {showCategories && (
        <CategoryChips
          selected={selectedCategory}
          onChange={(id) => {
            setSelectedCategory(id);
            setSearch("");
          }}
          t={t}
        />
      )}

      {/* Search */}
      {viewMode === "browse" && source !== "fanghub" && (
        <Input
          value={search}
          onChange={(e) => { setSearch(e.target.value); setSelectedCategory(null); }}
          placeholder={t("skills.search_placeholder")}
          leftIcon={<Search className="w-4 h-4" />}
          rightIcon={
            search ? (
              <button onClick={() => setSearch("")} className="hover:text-text-main">
                <X className="w-3 h-3" />
              </button>
            ) : undefined
          }
        />
      )}

      {/* ── Installed ── */}
      {viewMode === "installed" &&
        (skillsQuery.isLoading ? (
          <SkillGridSkeleton />
        ) : installedSkills.length === 0 ? (
          <EmptyState
            title={t("skills.no_skills")}
            icon={<Package className="h-6 w-6" />}
          />
        ) : (
          <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
            {installedSkills.map((s) => (
              <SkillCard
                key={s.name}
                variant="installed"
                name={s.name}
                version={s.version}
                description={s.description}
                author={s.author}
                toolsCount={s.tools_count}
                tags={s.tags}
                onUninstall={() => setUninstalling(s.name)}
                onViewDetail={() => setDetailSkillName(s.name)}
                t={t}
              />
            ))}
          </div>
        ))}

      {/* ── Browse ── */}
      {viewMode === "browse" && (() => {
        const activeQuery =
          source === "clawhub" ? clawhubQuery :
          source === "clawhub-cn" ? clawhubCnQuery :
          source === "skillhub" ? activeSkillhubQuery :
          fanghubQuery;

        return activeQuery.isLoading ? (
          <SkillGridSkeleton count={source === "fanghub" ? 4 : 6} />
        ) : isRateLimitError(activeQuery.error) ? (
          <EmptyState
            title={t("skills.rate_limited")}
            description={t("skills.rate_limited_desc")}
            icon={<Loader2 className="h-6 w-6 animate-spin" />}
          />
        ) : activeQuery.error ? (
          <EmptyState
            title={t("skills.load_error")}
            description={(activeQuery.error as Error).message}
            icon={<Search className="h-6 w-6" />}
          />
        ) : browseItems.length === 0 ? (
          <EmptyState
            title={t("skills.no_results")}
            icon={<Search className="h-6 w-6" />}
          />
        ) : (
          <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
            {source === "fanghub"
              ? (browseItems as FangHubSkill[]).map((skill) => (
                  <SkillCard
                    key={skill.name}
                    variant="fanghub"
                    name={skill.name}
                    version={skill.version}
                    description={skill.description}
                    tags={skill.tags}
                    isInstalled={skill.is_installed}
                    installPending={installingId === skill.name}
                    onInstall={() => handleInstall(skill.name, "fanghub")}
                    t={t}
                  />
                ))
              : (browseItems as ClawHubSkillWithStatus[]).map((s) => (
                  <SkillCard
                    key={s.slug}
                    variant="marketplace"
                    name={s.name}
                    version={s.version}
                    description={s.description}
                    tags={s.tags}
                    stars={s.stars}
                    downloads={s.downloads}
                    isInstalled={s.is_installed}
                    installPending={installingId === s.slug}
                    source={source as MarketplaceSource}
                    onInstall={() => handleInstall(s.slug, source)}
                    onViewDetail={() => { setDetailsSkill(s); setDetailsSource(source as MarketplaceSource); }}
                    t={t}
                  />
                ))
            }
          </div>
        );
      })()}

      {/* Marketplace detail modal */}
      {detailsSkill && skillWithDetails && (
        <MarketplaceDetailModal
          skill={skillWithDetails}
          source={detailsSource}
          pendingId={installingId}
          onClose={() => setDetailsSkill(null)}
          onInstall={() => handleInstall(detailsSkill.slug, detailsSource)}
          t={t}
        />
      )}

      {/* Uninstall confirmation */}
      <ConfirmDialog
        isOpen={!!uninstalling}
        title={t("skills.uninstall_confirm_title")}
        message={t("skills.uninstall_confirm", { name: uninstalling ?? "" })}
        tone="destructive"
        onConfirm={() => {
          if (uninstalling) {
            uninstallMutation.mutate(uninstalling, {
              onSuccess: () => {
                addToast(t("common.success"), "success");
                setUninstalling(null);
              },
            });
          }
        }}
        onClose={() => setUninstalling(null)}
      />

      {/* Create skill modal */}
      <CreateSkillModal
        isOpen={showCreateModal}
        onClose={() => setShowCreateModal(false)}
        onCreated={() =>
          addToast(
            t("skills.evo_created", { defaultValue: "Skill created successfully" }),
            "success",
          )
        }
        t={t}
      />

      {/* Installed skill detail + evolve modal */}
      <SkillDetailModal
        skillName={detailSkillName}
        isOpen={!!detailSkillName}
        onClose={() => setDetailSkillName(null)}
        t={t}
      />
    </div>
  );
}
