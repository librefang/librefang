import { useQuery } from "@tanstack/react-query";
import { formatDate } from "../lib/datetime";
import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
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
import { DrawerPanel } from "../components/ui/DrawerPanel";
import { ConfirmDialog } from "../components/ui/ConfirmDialog";
import { PageHeader } from "../components/ui/PageHeader";
import { PendingSkillsSection } from "../components/PendingSkillsSection";
import { useUIStore } from "../lib/store";
import {
  SkillHubBar,
  SkillHubHeadline,
  HubBadge,
  type HubFilter,
  type HubCounts,
  type HubHealthMap,
} from "../components/SkillHubBar";
import { getSkillHub } from "../lib/skillHubs";
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
  /** Optional hub origin badge rendered top-right (used by the unified
   *  "all hubs" view to make every card's source obvious). */
  hubBadge?: React.ReactNode;
  onInstall?: () => void;
  onUninstall?: () => void;
  onViewDetail?: () => void;
  t: (key: string, opts?: Record<string, unknown>) => string;
}

const SkillCard = React.memo(function SkillCard({
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
  hubBadge,
  onInstall,
  onUninstall,
  onViewDetail,
  t,
}: SkillCardProps) {
  // Card accent + icon style. Browse cards tint with the source hub's
  // brand color so a grid view of mixed hubs is colour-coded by origin
  // (matches the design canvas). Installed cards stay green.
  const hub = variant !== "installed" && source ? getSkillHub(source) : null;
  const iconClass =
    variant === "installed"
      ? "bg-success/10 border-success/20 text-success"
      : !hub
        ? "bg-brand/10 border-brand/20 text-brand"
        : "";
  const iconStyle = hub
    ? {
        background: `${hub.color}1a`,
        borderColor: `${hub.color}40`,
        color: hub.color,
      }
    : undefined;
  const hoverTextClass =
    variant === "installed"
      ? "group-hover:text-success"
      : "group-hover:text-brand";

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
      className={`relative flex flex-col overflow-hidden group ${onViewDetail ? "cursor-pointer" : ""}`}
      onClick={onViewDetail}
    >
      {/* Abs hub-source ribbon — top-right of the card so the origin is
       *  legible at a glance even when the card is in a dense grid. */}
      {hubBadge && (
        <div className="absolute top-2.5 right-2.5 pointer-events-none">
          {hubBadge}
        </div>
      )}
      {/* `accentClass` was an experiment with a 1px gradient bar at the
       *  top of every card; the canvas reference doesn't carry one and
       *  it visually fights the abs hub badge, so it's gone now. */}
      <div className="p-3.5 flex-1 flex flex-col gap-2.5">
        {/* Header — larger 38px icon, name row, author/version meta */}
        <div className="flex items-start gap-3 pr-20">
          <div
            className={`w-9 h-9 shrink-0 rounded-xl flex items-center justify-center border ${iconClass}`}
            style={iconStyle}
          >
            {icon}
          </div>
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-1.5 flex-wrap">
              <h3
                className={`font-bold text-sm truncate transition-colors ${hoverTextClass}`}
              >
                {name}
              </h3>
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
            <p className="text-[10.5px] text-text-dim font-mono mt-0.5 truncate">
              {[
                variant === "installed" ? "skill" : variant === "fanghub" ? "fanghub" : source,
                author,
                version ? `v${version}` : null,
              ]
                .filter(Boolean)
                .join(" · ")}
            </p>
          </div>
        </div>

        {/* Description */}
        <p className="text-xs text-text-dim line-clamp-2 flex-1 leading-relaxed">
          {description || "—"}
        </p>

        {/* Stats + inline Install button. Mirrors the design canvas where
         *  installs / rating sit on the same row as the install action so
         *  vertically dense card grids don't waste a whole row on stats. */}
        <div className="flex items-center gap-3 flex-wrap">
          {stars !== undefined && (
            <span className="flex items-center gap-1 text-[10.5px] font-mono text-text-dim/80">
              <Star className="w-3 h-3 text-warning" />
              {stars}
            </span>
          )}
          {downloads !== undefined && (
            <span className="flex items-center gap-1 text-[10.5px] font-mono text-text-dim/80">
              <Download className="w-3 h-3" />
              {downloads.toLocaleString()}
            </span>
          )}
          {variant === "installed" && toolsCount !== undefined && (
            <span className="flex items-center gap-1 text-[10.5px] font-mono text-text-dim/80">
              <Wrench className="w-3 h-3" />
              {toolsCount}
            </span>
          )}
          {updatedAt && (
            <span className="flex items-center gap-1 text-[10.5px] font-mono text-text-dim/80">
              <Calendar className="w-3 h-3" />
              {formatDate(updatedAt)}
            </span>
          )}
          <span className="flex-1" />
          {/* Inline action — Install (browse) or Uninstall (installed). */}
          {variant === "installed" && onUninstall ? (
            <div onClick={(e) => e.stopPropagation()}>
              <Button
                variant="ghost"
                size="sm"
                onClick={onUninstall}
                leftIcon={<Trash2 className="w-3.5 h-3.5" />}
                className="text-error hover:text-error"
              >
                {t("skills.uninstall")}
              </Button>
            </div>
          ) : variant !== "installed" && isInstalled ? (
            // Header already carries the installed badge — leave the
            // action slot empty so we don't double-stamp the card.
            null
          ) : variant !== "installed" && onInstall ? (
            <div onClick={(e) => e.stopPropagation()}>
              <Button
                variant="secondary"
                size="sm"
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

        {tags && tags.length > 0 && (
          <div className="flex flex-wrap gap-1">
            {tags.slice(0, 4).map((tag) => (
              <span
                key={tag}
                className="px-1.5 py-0.5 text-[10px] rounded font-mono text-text-dim/80 border border-border-subtle/60 bg-main/40"
              >
                {tag}
              </span>
            ))}
          </div>
        )}
      </div>
    </Card>
  );
});

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
            ? "bg-brand text-white shadow-sm"
            : "bg-surface border border-border-subtle text-text-dim hover:text-text-main hover:border-brand/30"
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
              ? "bg-brand text-white shadow-sm"
              : "bg-surface border border-border-subtle text-text-dim hover:text-text-main hover:border-brand/30"
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
    <DrawerPanel isOpen onClose={onClose} title={skill.name} size="md">
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
                    : "bg-brand/10 text-brand"
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
    </DrawerPanel>
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
    // Guard against double-submit: bail out if a creation is already in flight.
    if (creating || createSkillMutation.isPending) return;
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
        // Propagate unmount aborts down to the underlying fetch.
        signal: controller.signal,
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
    <DrawerPanel
      isOpen={isOpen}
      onClose={onClose}
      title={t("skills.evo_create_title", { defaultValue: "Create Skill" })}
      size="xl"
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
            disabled={creating || createSkillMutation.isPending}
            leftIcon={
              creating || createSkillMutation.isPending ? (
                <Loader2 className="w-4 h-4 animate-spin" />
              ) : (
                <Plus className="w-4 h-4" />
              )
            }
          >
            {creating || createSkillMutation.isPending
              ? t("common.creating", { defaultValue: "Creating..." })
              : t("common.create")}
          </Button>
        </div>
      </div>
    </DrawerPanel>
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
  addToast,
  t,
}: {
  skillName: string;
  onSubmit: (params: { path: string; content: string }) => void;
  onCancel: () => void;
  busy: boolean;
  addToast: (msg: string, type: "success" | "error" | "info") => void;
  t: (key: string, opts?: Record<string, unknown>) => string;
}) {
  const [subdir, setSubdir] = useState("references");
  const [filename, setFilename] = useState("");
  const [content, setContent] = useState("");
  const fileInputRef = useRef<HTMLInputElement | null>(null);

  const handleFilePick = async (file: File) => {
    if (file.size > 1024 * 1024) {
      addToast(
        t("skills.evo_file_too_large", { defaultValue: "File exceeds 1 MiB limit" }),
        "error",
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
          {error instanceof Error ? error.message : t("skills.evo_load_failed")}
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
  const [confirmAction, setConfirmAction] = useState<{
    title: string;
    message: string;
    onConfirm: () => void;
  } | null>(null);

  useEffect(() => {
    if (!isOpen) {
      setPane("none");
      setViewingFile(null);
      setConfirmAction(null);
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
      addToast(e instanceof Error ? e.message : t("skills.evo_action_failed"), "error");
    } finally {
      setBusy(false);
    }
  };

  const handleRollback = () => {
    if (!skillName) return;
    setConfirmAction({
      title: t("skills.evo_rollback", { defaultValue: "Rollback" }),
      message: t("skills.evo_rollback_confirm", {
        defaultValue:
          "Roll back to the previous version? This cannot be undone unless you patch again.",
      }),
      onConfirm: () => {
        setConfirmAction(null);
        void runMutation(
          () => rollbackMutation.mutateAsync({ name: skillName }),
          t("skills.evo_rolled_back", { defaultValue: "Skill rolled back" }),
        );
      },
    });
  };

  const handleRemoveFile = (path: string) => {
    if (!skillName) return;
    setConfirmAction({
      title: t("skills.evo_remove_file", { defaultValue: "Remove File" }),
      message: t("skills.evo_remove_file_confirm", {
        defaultValue: `Remove ${path}?`,
        path,
      }),
      onConfirm: () => {
        setConfirmAction(null);
        void runMutation(
          () => removeFileMutation.mutateAsync({ name: skillName, path }),
          t("skills.evo_file_removed", { defaultValue: "File removed" }),
        );
      },
    });
  };

  const handleDelete = () => {
    if (!skillName) return;
    setConfirmAction({
      title: t("skills.evo_delete", { defaultValue: "Delete" }),
      message: t("skills.evo_delete_confirm", {
        defaultValue: `Permanently delete ${skillName}? This cannot be undone.`,
        name: skillName,
      }),
      onConfirm: () => {
        const name = skillName;
        setConfirmAction(null);
        (async () => {
          setBusy(true);
          try {
            await deleteSkillMutation.mutateAsync({ name });
            addToast(
              t("skills.evo_deleted", { defaultValue: "Skill deleted" }),
              "success",
            );
            onClose();
          } catch (e: unknown) {
            addToast(e instanceof Error ? e.message : t("skills.evo_delete_failed"), "error");
          } finally {
            setBusy(false);
          }
        })();
      },
    });
  };

  return (
    <DrawerPanel
      isOpen={isOpen}
      onClose={onClose}
      title={detail?.name ?? skillName ?? ""}
      size="xl"
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
              addToast={addToast}
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
                            className="hover:text-brand"
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
      <ConfirmDialog
        isOpen={!!confirmAction}
        title={confirmAction?.title ?? ""}
        message={confirmAction?.message ?? ""}
        tone="destructive"
        onConfirm={() => confirmAction?.onConfirm()}
        onClose={() => setConfirmAction(null)}
      />
    </DrawerPanel>
  );
}

// ─── Main Page ────────────────────────────────────────────────────────────────

export function SkillsPage() {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);

  const [viewMode, setViewMode] = useState<ViewMode>("browse");
  /**
   * Which federated hub the browse grid pulls from. Defaults to
   * `"fanghub"` so the page lands on a populated grid (FangHub is the
   * always-warm local cache); switching to `"all"` aggregates every
   * configured hub but gates the remote ones behind a search keyword
   * to avoid wide network fan-outs on every page mount.
   */
  const [hubFilter, setHubFilter] = useState<HubFilter>("fanghub");
  const [selectedCategory, setSelectedCategory] = useState<string | null>(null);
  const [search, setSearch] = useState("");

  const isHubActive = useCallback(
    (id: MarketplaceSource) => hubFilter === "all" || hubFilter === id,
    [hubFilter],
  );

  const [uninstalling, setUninstalling] = useState<string | null>(null);
  const [detailsSkill, setDetailsSkill] = useState<ClawHubSkillWithStatus | null>(null);
  const [detailsSource, setDetailsSource] = useState<MarketplaceSource>("clawhub");
  const [detailsFangHub, setDetailsFangHub] = useState<FangHubSkill | null>(null);
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

  // Remote hub queries (clawhub / clawhub-cn) only fire when the user
  // narrows to that hub OR types a keyword in "all hubs" — otherwise
  // every Skills page mount would trigger a wide network search.
  const remoteEligible = viewMode === "browse" && (hubFilter !== "all" || !!keyword);

  const clawhubQuery = useQuery({
    ...skillQueries.clawhubSearch(keyword || "python"),
    enabled: remoteEligible && isHubActive("clawhub"),
  });

  const clawhubCnQuery = useQuery({
    ...skillQueries.clawhubCnSearch(keyword || "python"),
    enabled: remoteEligible && isHubActive("clawhub-cn"),
  });

  const skillhubBrowseQuery = useQuery({
    ...skillQueries.skillhubBrowse(),
    enabled: viewMode === "browse" && isHubActive("skillhub") && !keyword,
  });
  const skillhubSearchQuery = useQuery({
    ...skillQueries.skillhubSearch(keyword),
    enabled: viewMode === "browse" && isHubActive("skillhub") && !!keyword,
  });
  const activeSkillhubQuery = keyword ? skillhubSearchQuery : skillhubBrowseQuery;

  // FangHub is the LibreFang first-party registry — local cache, cheap
  // enough to keep enabled whenever browsing, regardless of hub filter.
  const fanghubQuery = useQuery({
    ...skillQueries.fanghubList(),
    enabled: viewMode === "browse" && isHubActive("fanghub"),
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

  const installedSlugSet = useMemo(() => {
    const set = new Set<string>();
    for (const s of installedSkills) {
      const src = s.source;
      const srcType = src?.type ?? "";
      const srcSlug = src?.slug;
      if (srcSlug) {
        if (srcType === "clawhub" || srcType === "clawhub-cn") {
          set.add(`clawhub:${srcSlug}`);
          set.add(`clawhub-cn:${srcSlug}`);
        } else {
          set.add(`${srcType}:${srcSlug}`);
        }
      }
      if (srcType === "" || srcType === "local") {
        set.add(`name:${s.name}`);
      }
    }
    return set;
  }, [installedSkills]);

  const isInstalledFromMarketplace = useCallback(
    (slug: string, src: MarketplaceSource) => {
      if (src === "clawhub" || src === "clawhub-cn") {
        return installedSlugSet.has(`clawhub:${slug}`) || installedSlugSet.has(`clawhub-cn:${slug}`) || installedSlugSet.has(`name:${slug}`);
      }
      return installedSlugSet.has(`${src}:${slug}`) || installedSlugSet.has(`name:${slug}`);
    },
    [installedSlugSet],
  );

  /** Items from a non-fanghub remote registry, normalized with
   *  `is_installed` and a `_hub` discriminator the unified card renderer
   *  reads. Filtering on text match here so the per-hub list and the
   *  merged "all" list stay in sync without repeating the predicate. */
  const buildRemoteItems = useCallback(
    (items: ClawHubBrowseItem[] | undefined, src: MarketplaceSource) =>
      (items ?? [])
        .map((s) => ({
          ...s,
          is_installed: isInstalledFromMarketplace(s.slug, src),
          _hub: src,
        }))
        .filter(
          (s) =>
            !search ||
            s.name.toLowerCase().includes(search.toLowerCase()) ||
            (s.description?.toLowerCase().includes(search.toLowerCase()) ?? false),
        ),
    [isInstalledFromMarketplace, search],
  );

  const fanghubItems = useMemo(
    () =>
      filterByCategory(fanghubQuery.data?.skills ?? [], selectedCategory).map((s) => ({
        ...s,
        _hub: "fanghub" as const,
      })),
    [fanghubQuery.data, selectedCategory],
  );
  const clawhubItems = useMemo(
    () => buildRemoteItems(clawhubQuery.data?.items, "clawhub"),
    [buildRemoteItems, clawhubQuery.data],
  );
  const clawhubCnItems = useMemo(
    () => buildRemoteItems(clawhubCnQuery.data?.items, "clawhub-cn"),
    [buildRemoteItems, clawhubCnQuery.data],
  );
  const skillhubItems = useMemo(
    () =>
      filterByCategory(
        buildRemoteItems(activeSkillhubQuery.data?.items, "skillhub"),
        selectedCategory,
      ),
    [buildRemoteItems, activeSkillhubQuery.data, selectedCategory],
  );

  /** What the grid actually renders, narrowed to the active hub or
   *  merged across all hubs when `hubFilter === "all"`. */
  const browseItems = useMemo(() => {
    if (hubFilter === "fanghub") return fanghubItems;
    if (hubFilter === "clawhub") return clawhubItems;
    if (hubFilter === "clawhub-cn") return clawhubCnItems;
    if (hubFilter === "skillhub") return skillhubItems;
    // "all" — merge. FangHub first (curated), then clawhub/clawhub-cn,
    // then skillhub. Only includes hubs that actually returned data this
    // render (clawhub etc. are search-gated).
    return [...fanghubItems, ...clawhubItems, ...clawhubCnItems, ...skillhubItems];
  }, [hubFilter, fanghubItems, clawhubItems, clawhubCnItems, skillhubItems]);

  const hubCounts: HubCounts = useMemo(
    () => ({
      fanghub: fanghubItems.length,
      clawhub: clawhubItems.length,
      "clawhub-cn": clawhubCnItems.length,
      skillhub: skillhubItems.length,
    }),
    [fanghubItems.length, clawhubItems.length, clawhubCnItems.length, skillhubItems.length],
  );

  const hubHealth: HubHealthMap = useMemo(() => {
    const flag = (q: { isFetching: boolean; isError: boolean }) =>
      q.isError ? ("down" as const) : q.isFetching ? ("checking" as const) : ("live" as const);
    return {
      fanghub: flag(fanghubQuery),
      clawhub: flag(clawhubQuery),
      "clawhub-cn": flag(clawhubCnQuery),
      skillhub: flag(activeSkillhubQuery),
    };
  }, [fanghubQuery, clawhubQuery, clawhubCnQuery, activeSkillhubQuery]);

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
    src: MarketplaceSource,
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
      addToast(e instanceof Error ? e.message : t("skills.reload_failed"), "error");
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
        helpText={t("skills.help")}
        isFetching={isAnyFetching}
        onRefresh={handleReload}
        actions={
          <>
            <span className="hidden sm:inline-block px-2.5 py-1 rounded-full border border-border-subtle bg-surface text-[10px] font-bold uppercase text-text-dim">
              {t("skills.installed_count", { count: installedSkills.length })}
            </span>
            <a
              href="https://librefang.ai/skills"
              target="_blank"
              rel="noopener noreferrer"
              className="hidden md:flex h-8 items-center gap-1.5 rounded-xl border border-border-subtle bg-surface px-3 text-xs font-bold text-text-dim hover:text-brand hover:border-brand/30 transition-colors"
            >
              <Globe className="h-3.5 w-3.5" />
              <span>{t("skills.browse_registry", { defaultValue: "Registry" })}</span>
            </a>
            <button
              className="flex h-8 items-center gap-1.5 rounded-xl border border-brand/30 bg-brand/10 px-3 text-xs font-bold text-brand hover:bg-brand/20 transition-colors"
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

      {/* Skill workshop pending review (#3328). Mounts above the
          installed/browse tab bar so a fresh capture is the first
          thing the operator sees on the Skills page; renders nothing
          while the queue is empty (the steady state for most
          operators) so it doesn't waste page space — see the
          early-return in `PendingSkillsSection`. */}
      <PendingSkillsSection />

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
              activeColor: "text-brand",
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

      {/* Federated hub source bar (browse only). Replaces the old
       *  4-option <select> with a horizontal pill row that exposes
       *  health, latency hint (via tooltip), and per-hub counts. */}
      {viewMode === "browse" && (
        <SkillHubBar
          hubFilter={hubFilter}
          onChange={setHubFilter}
          counts={hubCounts}
          health={hubHealth}
          totalCount={browseItems.length}
        />
      )}

      {/* When a single hub is selected, surface its domain + tagline so
       *  operators know exactly where they're searching. */}
      {viewMode === "browse" && hubFilter !== "all" && (
        <SkillHubHeadline hub={hubFilter} />
      )}

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

      {/* Search — applies to every hub. FangHub items get prefix-matched
       *  client-side, the remote hubs treat the keyword as their search
       *  query (gated so we don't fan it out across all hubs without a
       *  user-typed term). */}
      {viewMode === "browse" && (
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
        // Pick the "primary" query state for skeleton / error rendering.
        // For "all", showing a single skeleton when *any* hub is fetching
        // would be misleading; only block when nothing is rendered yet.
        const activeQuery =
          hubFilter === "clawhub" ? clawhubQuery :
          hubFilter === "clawhub-cn" ? clawhubCnQuery :
          hubFilter === "skillhub" ? activeSkillhubQuery :
          hubFilter === "fanghub" ? fanghubQuery :
          null;

        const isLoading = activeQuery
          ? activeQuery.isLoading
          : fanghubQuery.isLoading && browseItems.length === 0;
        const queryError = activeQuery?.error ?? null;

        return isLoading ? (
          <SkillGridSkeleton count={hubFilter === "fanghub" ? 4 : 6} />
        ) : queryError && isRateLimitError(queryError) ? (
          <EmptyState
            title={t("skills.rate_limited")}
            description={t("skills.rate_limited_desc")}
            icon={<Loader2 className="h-6 w-6 animate-spin" />}
          />
        ) : queryError ? (
          <EmptyState
            title={t("skills.load_error")}
            description={(queryError as Error).message}
            icon={<Search className="h-6 w-6" />}
          />
        ) : browseItems.length === 0 ? (
          <EmptyState
            title={
              hubFilter === "all" && !search
                ? t("skills.search_to_explore", {
                    defaultValue: "Type to search across hubs",
                  })
                : t("skills.no_results")
            }
            icon={<Search className="h-6 w-6" />}
          />
        ) : (
          <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
            {browseItems.map((entry) =>
              entry._hub === "fanghub" ? (
                <SkillCard
                  key={`fanghub:${entry.name}`}
                  variant="fanghub"
                  name={entry.name}
                  version={entry.version}
                  description={entry.description}
                  tags={entry.tags}
                  isInstalled={entry.is_installed}
                  installPending={installingId === entry.name}
                  source="fanghub"
                  hubBadge={<HubBadge hub="fanghub" />}
                  onInstall={() => handleInstall(entry.name, "fanghub")}
                  onViewDetail={() => setDetailsFangHub(entry as FangHubSkill)}
                  t={t}
                />
              ) : (
                <SkillCard
                  key={`${entry._hub}:${entry.slug}`}
                  variant="marketplace"
                  name={entry.name}
                  version={entry.version}
                  description={entry.description}
                  tags={entry.tags}
                  stars={entry.stars}
                  downloads={entry.downloads}
                  isInstalled={entry.is_installed}
                  installPending={installingId === entry.slug}
                  source={entry._hub}
                  hubBadge={<HubBadge hub={entry._hub} />}
                  onInstall={() => handleInstall(entry.slug, entry._hub)}
                  onViewDetail={() => {
                    setDetailsSkill(entry as ClawHubSkillWithStatus);
                    setDetailsSource(entry._hub);
                  }}
                  t={t}
                />
              ),
            )}
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

      {/* FangHub skill detail */}
      <DrawerPanel
        isOpen={!!detailsFangHub}
        onClose={() => setDetailsFangHub(null)}
        title={detailsFangHub?.name ?? ""}
        size="md"
      >
        {detailsFangHub && (
          <div className="p-5 space-y-4">
            <div className="flex items-start gap-3">
              <div className="w-12 h-12 rounded-xl bg-brand/10 flex items-center justify-center shrink-0 text-brand">
                <Zap className="w-5 h-5" />
              </div>
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2 flex-wrap">
                  <h2 className="text-lg font-black tracking-tight truncate">{detailsFangHub.name}</h2>
                  <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-main text-text-dim font-mono">v{detailsFangHub.version}</span>
                </div>
                {detailsFangHub.author && (
                  <p className="text-[11px] text-text-dim/70 mt-0.5">{detailsFangHub.author}</p>
                )}
              </div>
            </div>

            <p className="text-sm text-text-dim leading-relaxed whitespace-pre-wrap">
              {detailsFangHub.description}
            </p>

            {detailsFangHub.tags && detailsFangHub.tags.length > 0 && (
              <div className="flex flex-wrap gap-1.5">
                {detailsFangHub.tags.map((tag) => (
                  <span key={tag} className="px-2 py-1 rounded-lg text-xs font-bold bg-brand/10 text-brand">
                    {tag}
                  </span>
                ))}
              </div>
            )}

            {detailsFangHub.is_installed ? (
              <Button variant="secondary" className="w-full" disabled leftIcon={<CheckCircle2 className="w-4 h-4" />}>
                {t("skills.installed")}
              </Button>
            ) : (
              <Button
                variant="primary"
                className="w-full"
                disabled={installingId === detailsFangHub.name}
                onClick={() => {
                  if (detailsFangHub) handleInstall(detailsFangHub.name, "fanghub");
                }}
                leftIcon={installingId === detailsFangHub.name ? <Loader2 className="w-4 h-4 animate-spin" /> : <Download className="w-4 h-4" />}
              >
                {installingId === detailsFangHub.name ? t("skills.installing") : t("skills.install")}
              </Button>
            )}
          </div>
        )}
      </DrawerPanel>
    </div>
  );
}
