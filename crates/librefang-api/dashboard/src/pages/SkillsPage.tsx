import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { formatDate } from "../lib/datetime";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { listSkills, uninstallSkill, clawhubSearch, clawhubInstall, clawhubGetSkill, skillhubSearch, skillhubBrowse, skillhubInstall, skillhubGetSkill, listTools, type ClawHubBrowseItem } from "../api";
import { PageHeader } from "../components/ui/PageHeader";
import { CardSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { Input } from "../components/ui/Input";
import { Pagination } from "../components/ui/Pagination";
import { useUIStore } from "../lib/store";
import {
  Wrench, Search, CheckCircle2, X,
  Download, Trash2, Star, Loader2, Sparkles, Package,
  Code, GitBranch, Globe, Cloud, Monitor, Bot, Database,
  Briefcase, Shield, Terminal, Calendar, Store
} from "lucide-react";

type ClawHubSkillWithStatus = ClawHubBrowseItem & { is_installed?: boolean };

const REFRESH_MS = 30000;
const ITEMS_PER_PAGE = 6;

type ViewMode = "installed" | "marketplace" | "builtin";
type MarketplaceSource = "clawhub" | "skillhub";

// Categories with icons and search keywords
const categories = [
  { id: "coding", name: "Coding", icon: Code, keyword: "python javascript code" },
  { id: "git", name: "Git", icon: GitBranch, keyword: "git github" },
  { id: "web", name: "Web", icon: Globe, keyword: "web frontend html css" },
  { id: "devops", name: "DevOps", icon: Cloud, keyword: "devops cloud aws docker kubernetes" },
  { id: "browser", name: "Browser", icon: Monitor, keyword: "browser automation" },
  { id: "ai", name: "AI", icon: Bot, keyword: "ai llm gpt openai" },
  { id: "data", name: "Data", icon: Database, keyword: "data analytics python" },
  { id: "productivity", name: "Productivity", icon: Briefcase, keyword: "productivity" },
  { id: "security", name: "Security", icon: Shield, keyword: "security" },
  { id: "cli", name: "CLI", icon: Terminal, keyword: "cli bash shell" },
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

// Skill Card - Installed
function InstalledSkillCard({ skill, onUninstall, t }: {
  skill: { name: string; version?: string; description?: string; author?: string; tools_count?: number };
  onUninstall: (name: string) => void;
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
        <div className="flex justify-between items-center text-[10px] font-bold text-text-dim uppercase mb-4">
          <span>{t("skills.author")}: {skill.author || t("common.unknown")}</span>
          <span>{t("skills.tools")}: {skill.tools_count || 0}</span>
        </div>
        <Button variant="ghost" className="w-full text-error hover:text-error" onClick={() => onUninstall(skill.name)} leftIcon={<Trash2 className="w-4 h-4" />}>
          {t("skills.uninstall")}
        </Button>
      </div>
    </Card>
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
            <button onClick={onClose} className="p-2 hover:bg-main/30 rounded-lg transition-colors">
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

const CN_TIMEZONES = new Set([
  "Asia/Shanghai", "Asia/Chongqing", "Asia/Harbin",
  "Asia/Urumqi", "Asia/Kashgar",
]);

function isChineseTimezone(): boolean {
  try {
    return CN_TIMEZONES.has(Intl.DateTimeFormat().resolvedOptions().timeZone);
  } catch {
    return false;
  }
}

const USE_SKILLHUB = isChineseTimezone();

export function SkillsPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const addToast = useUIStore((s) => s.addToast);

  // View state
  const [viewMode, setViewMode] = useState<ViewMode>("marketplace");
  const [selectedCategory, setSelectedCategory] = useState<string | null>(categories[0]?.id || null);
  const [search, setSearch] = useState("");
  const [page, setPage] = useState(1);

  // Actions
  const [builtinSearch, setBuiltinSearch] = useState("");
  const [uninstalling, setUninstalling] = useState<string | null>(null);
  const [detailsSkill, setDetailsSkill] = useState<ClawHubSkillWithStatus | null>(null);
  const [detailsSource, setDetailsSource] = useState<MarketplaceSource>("clawhub");
  const [installingId, setInstallingId] = useState<string | null>(null);

  // Get search keyword from category or use search input (ClawHub only)
  const searchKeyword = selectedCategory
    ? categories.find(c => c.id === selectedCategory)?.keyword || ""
    : search;

  // Queries
  const skillsQuery = useQuery({ queryKey: ["skills", "list"], queryFn: listSkills, refetchInterval: REFRESH_MS });
  const builtinToolsQuery = useQuery({ queryKey: ["tools"], queryFn: listTools, enabled: viewMode === "builtin" });
  const builtinTools: any[] = builtinToolsQuery.data ?? [];
  const filteredBuiltin = useMemo(
    () => builtinTools.filter((tool) =>
      !builtinSearch ||
      (tool.name || "").toLowerCase().includes(builtinSearch.toLowerCase()) ||
      (tool.description || "").toLowerCase().includes(builtinSearch.toLowerCase())
    ),
    [builtinTools, builtinSearch]
  );

  // ClawHub (non-CN)
  const searchQuery = useQuery({
    queryKey: ["clawhub", "search", searchKeyword],
    queryFn: () => clawhubSearch(searchKeyword || "python"),
    staleTime: 60000,
    enabled: viewMode === "marketplace" && !USE_SKILLHUB && !!searchKeyword,
  });

  // SkillHub (CN)
  const skillhubBrowseQuery = useQuery({
    queryKey: ["skillhub", "browse"],
    queryFn: () => skillhubBrowse(),
    staleTime: 60000,
    enabled: viewMode === "marketplace" && USE_SKILLHUB && !search,
  });
  const skillhubSearchQuery = useQuery({
    queryKey: ["skillhub", "search", search],
    queryFn: () => skillhubSearch(search),
    staleTime: 60000,
    enabled: viewMode === "marketplace" && USE_SKILLHUB && !!search,
  });
  const activeSkillhubQuery = search ? skillhubSearchQuery : skillhubBrowseQuery;

  const detailQuery = useQuery({
    queryKey: [detailsSource, "skill", detailsSkill?.slug],
    queryFn: () => {
      if (!detailsSkill?.slug) return Promise.resolve(null);
      return detailsSource === "skillhub"
        ? skillhubGetSkill(detailsSkill.slug)
        : clawhubGetSkill(detailsSkill.slug);
    },
    enabled: !!detailsSkill?.slug,
  });

  // Merge detail data with skill
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

  // Marketplace data — routed by timezone
  const marketplaceSource: MarketplaceSource = USE_SKILLHUB ? "skillhub" : "clawhub";
  const rawMarketplaceItems = USE_SKILLHUB
    ? (activeSkillhubQuery.data?.items ?? [])
    : (searchQuery.data?.items ?? []);
  const isMarketplaceLoading = USE_SKILLHUB ? activeSkillhubQuery.isLoading : searchQuery.isLoading;
  const marketplaceError = (USE_SKILLHUB ? activeSkillhubQuery.error : searchQuery.error) as any;
  const isRateLimited = marketplaceError?.message?.includes("429")
    || marketplaceError?.message?.includes("rate")
    || marketplaceError?.message?.includes("Rate limit")
    || marketplaceError?.status === 429;

  const filteredMarketplace = useMemo(
    () => rawMarketplaceItems
      .map((s: any) => ({ ...s, is_installed: isInstalledFromMarketplace(s.slug, marketplaceSource) }))
      .filter((s: any) => !search
        || s.name.toLowerCase().includes(search.toLowerCase())
        || s.description?.toLowerCase().includes(search.toLowerCase())),
    [rawMarketplaceItems, installedSkills, search],
  );
  const totalPages = Math.ceil(filteredMarketplace.length / ITEMS_PER_PAGE);
  const paginatedMarketplace = filteredMarketplace.slice((page - 1) * ITEMS_PER_PAGE, page * ITEMS_PER_PAGE);

  // Mutations
  const uninstallMutation = useMutation({
    mutationKey: ["uninstall", "skill"],
    mutationFn: uninstallSkill,
    retry: 0,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["skills", "list"] });
      addToast(t("common.success"), "success");
      setUninstalling(null);
    }
  });

  const installMutation = useMutation({
    mutationKey: ["install", "skill", "clawhub"],
    mutationFn: ({ slug }: { slug: string }) => clawhubInstall(slug),
    retry: 0,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["skills", "list"] });
      addToast(t("common.success"), "success");
      setInstallingId(null);
      setDetailsSkill(null);
    },
    onError: (error: any) => {
      const msg = error.message || t("common.error");
      addToast(msg.includes("abort") ? t("skills.install_timeout") : msg, "error");
      setInstallingId(null);
    }
  });

  const skillhubInstallMutation = useMutation({
    mutationKey: ["install", "skill", "skillhub"],
    mutationFn: ({ slug }: { slug: string }) => skillhubInstall(slug),
    retry: 0,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["skills", "list"] });
      addToast(t("common.success"), "success");
      setInstallingId(null);
      setDetailsSkill(null);
    },
    onError: (error: any) => {
      const msg = error.message || t("common.error");
      addToast(msg.includes("abort") ? t("skills.install_timeout") : msg, "error");
      setInstallingId(null);
    }
  });

  const handleCategoryClick = (categoryId: string) => {
    if (selectedCategory === categoryId) {
      setSelectedCategory(null); // Deselect
    } else {
      setSelectedCategory(categoryId);
      setSearch("");
    }
    setPage(1);
  };

  const handleInstall = (slug: string, source: MarketplaceSource = "clawhub") => {
    setInstallingId(slug);
    if (source === "skillhub") {
      skillhubInstallMutation.mutate({ slug });
    } else {
      installMutation.mutate({ slug });
    }
  };

  const handleUninstall = (name: string) => {
    setUninstalling(name);
  };

  const confirmUninstall = () => {
    if (uninstalling) {
      uninstallMutation.mutate(uninstalling);
    }
  };

  const handleViewDetails = (skill: ClawHubSkillWithStatus, source: MarketplaceSource) => {
    setDetailsSkill(skill);
    setDetailsSource(source);
  };

  const isAnyFetching = skillsQuery.isFetching
    || searchQuery.isFetching
    || skillhubBrowseQuery.isFetching
    || skillhubSearchQuery.isFetching;

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("common.infrastructure")}
        title={t("skills.title")}
        subtitle={t("skills.subtitle")}
        isFetching={isAnyFetching}
        onRefresh={() => { void skillsQuery.refetch(); void searchQuery.refetch(); void skillhubBrowseQuery.refetch(); void skillhubSearchQuery.refetch(); }}
        icon={<Wrench className="h-4 w-4" />}
        helpText={t("skills.help")}
        actions={
          <div className="hidden rounded-full border border-border-subtle bg-surface px-3 py-1.5 text-[10px] font-bold uppercase text-text-dim sm:block">
            {t("skills.installed_count", { count: installedSkills.length })}
          </div>
        }
      />

      {/* View Toggle */}
      <div className="flex gap-1 p-1 bg-main/30 rounded-xl w-fit">
        <button
          onClick={() => { setViewMode("installed"); setPage(1); setSearch(""); setSelectedCategory(null); }}
          className={`flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-bold transition-colors ${
            viewMode === "installed" ? "bg-surface text-success shadow-sm" : "text-text-dim hover:text-text-main"
          }`}
        >
          <Package className="w-4 h-4" />
          {t("skills.installed")}
          <span className={`ml-1 px-1.5 py-0.5 rounded-full text-[10px] ${viewMode === "installed" ? "bg-success/20 text-success" : "bg-border-subtle text-text-dim"}`}>
            {installedSkills.length}
          </span>
        </button>
        <button
          onClick={() => { setViewMode("marketplace"); setPage(1); setSearch(""); setSelectedCategory(USE_SKILLHUB ? null : (categories[0]?.id || null)); }}
          className={`flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-bold transition-colors ${
            viewMode === "marketplace" ? "bg-surface text-brand shadow-sm" : "text-text-dim hover:text-text-main"
          }`}
        >
          {USE_SKILLHUB ? <Store className="w-4 h-4" /> : <Sparkles className="w-4 h-4" />}
          {USE_SKILLHUB ? t("skills.skillhub") : t("skills.marketplace")}
        </button>
        <button
          onClick={() => { setViewMode("builtin"); setPage(1); setSearch(""); setSelectedCategory(null); }}
          className={`flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-bold transition-colors ${
            viewMode === "builtin" ? "bg-surface text-text shadow-sm" : "text-text-dim hover:text-text-main"
          }`}
        >
          <Wrench className="w-4 h-4" />
          {t("skills.builtin")}
        </button>
      </div>

      {/* Category Chips — ClawHub only (non-CN) */}
      {viewMode === "marketplace" && !USE_SKILLHUB && (
        <div className="flex flex-wrap gap-1.5 sm:gap-2">
          {categories.map(cat => (
            <button
              key={cat.id}
              onClick={() => handleCategoryClick(cat.id)}
              className={`flex items-center gap-2 px-3 py-1.5 rounded-lg text-xs font-bold transition-colors ${
                selectedCategory === cat.id
                  ? "bg-brand text-white shadow-md"
                  : "bg-main/50 text-text-dim hover:bg-main hover:text-text-main border border-border-subtle"
              }`}
            >
              {getCategoryIcon(cat.id)}
              {cat.name}
            </button>
          ))}
        </div>
      )}

      {/* Search — Marketplace */}
      {viewMode === "marketplace" && (
        <Input
          value={search}
          onChange={(e) => { setSearch(e.target.value); setSelectedCategory(null); setPage(1); }}
          placeholder={
            USE_SKILLHUB
              ? t("skills.skillhub_search_placeholder")
              : (selectedCategory ? categories.find(c => c.id === selectedCategory)?.name + "..." : t("skills.search_placeholder"))
          }
          leftIcon={<Search className="w-4 h-4" />}
          rightIcon={search ? (
            <button onClick={() => setSearch("")} className="hover:text-text-main">
              <X className="w-3 h-3" />
            </button>
          ) : undefined}
        />
      )}

      {/* Search — Built-in */}
      {viewMode === "builtin" && (
        <Input
          value={builtinSearch}
          onChange={(e) => setBuiltinSearch(e.target.value)}
          placeholder={t("settings.tools_search")}
          leftIcon={<Search className="w-4 h-4" />}
          rightIcon={builtinSearch ? (
            <button onClick={() => setBuiltinSearch("")} className="hover:text-text-main">
              <X className="w-3 h-3" />
            </button>
          ) : undefined}
        />
      )}

      {/* Content */}
      {viewMode === "installed" ? (
        skillsQuery.isLoading ? (
          <div className="grid gap-2 sm:gap-4 md:grid-cols-2 xl:grid-cols-3">
            {[1, 2, 3, 4, 5, 6].map(i => <CardSkeleton key={i} />)}
          </div>
        ) : installedSkills.length === 0 ? (
          <EmptyState title={t("skills.no_skills")} icon={<Package className="h-6 w-6" />} />
        ) : (
          <div className="grid gap-2 sm:gap-4 md:grid-cols-2 xl:grid-cols-3">
            {installedSkills.map(s => (
              <InstalledSkillCard key={s.name} skill={s} onUninstall={handleUninstall} t={t} />
            ))}
          </div>
        )
      ) : viewMode === "marketplace" ? (
        isMarketplaceLoading ? (
          <div className="grid gap-2 sm:gap-4 md:grid-cols-2 xl:grid-cols-3">
            {[1, 2, 3, 4, 5, 6].map(i => <CardSkeleton key={i} />)}
          </div>
        ) : isRateLimited ? (
          <EmptyState
            title={t("skills.rate_limited")}
            description={USE_SKILLHUB ? t("skills.skillhub_rate_limited_desc") : t("skills.rate_limited_desc")}
            icon={<Loader2 className="h-6 w-6 animate-spin" />}
          />
        ) : marketplaceError ? (
          <EmptyState
            title={t("skills.load_error")}
            description={marketplaceError.message || t("common.error")}
            icon={<Search className="h-6 w-6" />}
          />
        ) : filteredMarketplace.length === 0 ? (
          <EmptyState title={t("skills.no_results")} icon={<Search className="h-6 w-6" />} />
        ) : (
          <>
            <div className="grid gap-2 sm:gap-4 md:grid-cols-2 xl:grid-cols-3">
              {paginatedMarketplace.map((s: any) => (
                <MarketplaceSkillCard
                  key={s.slug}
                  skill={s}
                  pendingId={installingId}
                  onInstall={(slug) => handleInstall(slug, marketplaceSource)}
                  onViewDetails={(sk) => handleViewDetails(sk, marketplaceSource)}
                  source={marketplaceSource}
                  t={t}
                />
              ))}
            </div>
            {totalPages > 1 && (
              <Pagination currentPage={page} totalPages={totalPages} onPageChange={setPage} />
            )}
          </>
        )
      ) : null}

      {/* Built-in Tools */}
      {viewMode === "builtin" && (
        builtinToolsQuery.isLoading ? (
          <div className="grid gap-2 sm:gap-3 sm:grid-cols-2 lg:grid-cols-3">
            {[1,2,3,4,5,6].map(i => <CardSkeleton key={i} />)}
          </div>
        ) : filteredBuiltin.length === 0 ? (
          <EmptyState title={t("common.no_data")} icon={<Wrench className="h-6 w-6" />} />
        ) : (
          <div className="grid gap-2 sm:gap-3 sm:grid-cols-2 lg:grid-cols-3">
            {filteredBuiltin.map((tool: any, i: number) => (
              <div
                key={tool.name || i}
                className="flex items-start gap-3 p-4 rounded-2xl border border-border-subtle bg-surface hover:border-brand/20 hover:bg-brand/[0.02] transition-colors duration-200 cursor-default group"
              >
                <div className="w-9 h-9 rounded-xl bg-brand/8 flex items-center justify-center shrink-0 ring-1 ring-brand/10 group-hover:ring-brand/20 transition-all duration-200">
                  <Wrench className="w-4 h-4 text-brand/50 group-hover:text-brand/70 transition-colors duration-200" />
                </div>
                <div className="min-w-0 flex-1 pt-0.5">
                  <div className="flex items-center gap-1.5 flex-wrap">
                    <p className="text-sm font-bold truncate group-hover:text-brand transition-colors duration-200">
                      {tool.name || tool.id}
                    </p>
                    {tool.source && (
                      <span className="text-[9px] px-1.5 py-px rounded-full bg-main border border-border-subtle text-text-dim font-medium shrink-0">
                        {tool.source}
                      </span>
                    )}
                  </div>
                  {tool.description && (
                    <p className="text-xs text-text-dim line-clamp-2 mt-0.5 leading-relaxed">
                      {tool.description}
                    </p>
                  )}
                </div>
              </div>
            ))}
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
    </div>
  );
}
