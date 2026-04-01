import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { formatDate } from "../lib/datetime";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { listSkills, uninstallSkill, clawhubSearch, clawhubInstall, clawhubGetSkill, skillhubSearch, skillhubBrowse, skillhubInstall, skillhubGetSkill, fanghubListSkills, installSkill, type ClawHubBrowseItem, type FangHubSkill } from "../api";
import { PageHeader } from "../components/ui/PageHeader";
import { CardSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { Input } from "../components/ui/Input";
import { useUIStore } from "../lib/store";
import {
  Wrench, Search, CheckCircle2, X,
  Download, Trash2, Star, Loader2, Sparkles, Package,
  Code, GitBranch, Globe, Cloud, Monitor, Bot, Database,
  Briefcase, Shield, Terminal, Calendar, Store, Zap,
} from "lucide-react";

type ClawHubSkillWithStatus = ClawHubBrowseItem & { is_installed?: boolean };

const REFRESH_MS = 30000;

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

export function SkillsPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const addToast = useUIStore((s) => s.addToast);

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

  // Get search keyword from category or use search input
  const searchKeyword = selectedCategory
    ? categories.find(c => c.id === selectedCategory)?.keyword || ""
    : search;

  // Queries
  const skillsQuery = useQuery({ queryKey: ["skills", "list"], queryFn: listSkills, refetchInterval: REFRESH_MS });

  // ClawHub search API
  const searchQuery = useQuery({
    queryKey: ["clawhub", "search", searchKeyword],
    queryFn: () => clawhubSearch(searchKeyword || "python"),
    staleTime: 60000,
    enabled: viewMode === "marketplace" && !!searchKeyword,
  });

  // Skillhub queries — category selection also drives search
  const skillhubKeyword = skillhubSearch_ || (selectedCategory ? categories.find(c => c.id === selectedCategory)?.keyword || "" : "");

  const skillhubBrowseQuery = useQuery({
    queryKey: ["skillhub", "browse"],
    queryFn: () => skillhubBrowse(),
    staleTime: 60000,
    enabled: viewMode === "skillhub" && !skillhubKeyword,
  });

  const skillhubSearchQuery = useQuery({
    queryKey: ["skillhub", "search", skillhubKeyword],
    queryFn: () => skillhubSearch(skillhubKeyword),
    staleTime: 60000,
    enabled: viewMode === "skillhub" && !!skillhubKeyword,
  });

  const activeSkillhubQuery = skillhubKeyword ? skillhubSearchQuery : skillhubBrowseQuery;

  // FangHub — official skills from local registry (~/.librefang/registry/skills)
  const fanghubQuery = useQuery({
    queryKey: ["fanghub", "list"],
    queryFn: fanghubListSkills,
    staleTime: 60000,
    enabled: viewMode === "fanghub",
  });

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
      queryClient.invalidateQueries({ queryKey: ["fanghub", "list"] });
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

  const fanghubInstallMutation = useMutation({
    mutationKey: ["install", "skill", "fanghub"],
    mutationFn: ({ name }: { name: string }) => installSkill(name),
    retry: 0,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["skills", "list"] });
      queryClient.invalidateQueries({ queryKey: ["fanghub", "list"] });
      addToast(t("common.success"), "success");
      setInstallingId(null);
    },
    onError: (error: any) => {
      const msg = error.message || t("common.error");
      addToast(msg.includes("abort") ? t("skills.install_timeout") : msg, "error");
      setInstallingId(null);
    }
  });

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
    if (source === "skillhub") {
      skillhubInstallMutation.mutate({ slug });
    } else {
      installMutation.mutate({ slug });
    }
  };

  const handleUninstall = (name: string) => setUninstalling(name);
  const confirmUninstall = () => { if (uninstalling) uninstallMutation.mutate(uninstalling); };
  const handleViewDetails = (skill: ClawHubSkillWithStatus, source: MarketplaceSource) => {
    setDetailsSkill(skill);
    setDetailsSource(source);
  };

  const isAnyFetching = skillsQuery.isFetching || searchQuery.isFetching
    || skillhubBrowseQuery.isFetching || skillhubSearchQuery.isFetching || fanghubQuery.isFetching;

  return (
    <div className="flex flex-col gap-4 transition-colors duration-300">
      <PageHeader
        badge={t("common.infrastructure")}
        title={t("skills.title")}
        subtitle={t("skills.subtitle")}
        isFetching={isAnyFetching}
        onRefresh={() => { void skillsQuery.refetch(); void searchQuery.refetch(); void activeSkillhubQuery.refetch(); }}
        icon={<Wrench className="h-4 w-4" />}
        helpText={t("skills.help")}
        actions={
          <div className="hidden rounded-full border border-border-subtle bg-surface px-3 py-1.5 text-[10px] font-bold uppercase text-text-dim sm:block">
            {t("skills.installed_count", { count: installedSkills.length })}
          </div>
        }
      />

      {/* View Toggle + Category Chips — one row */}
      <div className="flex items-center gap-2 flex-wrap">
        {/* Tabs */}
        <div className="flex gap-0.5 p-0.5 bg-main/30 rounded-lg shrink-0">
          <button
            onClick={() => { setViewMode("installed"); setSearch(""); setSkillhubSearch(""); setSelectedCategory(null); }}
            className={`flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-bold transition-colors ${
              viewMode === "installed" ? "bg-surface text-success shadow-sm" : "bg-surface-hover text-text-dim hover:text-text-main"
            }`}
          >
            <Package className="w-3.5 h-3.5" />
            {t("skills.installed")}
            <span className={`px-1.5 py-0.5 rounded-full text-[10px] ${viewMode === "installed" ? "bg-success/20 text-success" : "bg-border-subtle text-text-dim"}`}>
              {installedSkills.length}
            </span>
          </button>

          <button
            onClick={() => { setViewMode("fanghub"); setSearch(""); setSkillhubSearch(""); setSelectedCategory(null); }}
            className={`relative flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-bold transition-colors ${
              viewMode === "fanghub" ? "bg-surface text-brand shadow-sm" : "bg-surface-hover text-text-dim hover:text-text-main"
            }`}
          >
            <Zap className="w-3.5 h-3.5" />
            {t("skills.builtin")}
            <span className={`absolute top-0.5 right-1 text-[8px] font-black px-1 py-px rounded-full leading-none ${viewMode === "fanghub" ? "bg-brand text-white" : "bg-border-subtle text-text-dim"}`}>{t("skills.official")}</span>
          </button>

          {!USE_SKILLHUB && (
            <button
              onClick={() => { setViewMode("marketplace"); setSkillhubSearch(""); setSelectedCategory(null); }}
              className={`flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-bold transition-colors ${
                viewMode === "marketplace" ? "bg-surface text-brand shadow-sm" : "bg-surface-hover text-text-dim hover:text-text-main"
              }`}
            >
              <Sparkles className="w-3.5 h-3.5" />
              {t("skills.marketplace")}
            </button>
          )}

          {USE_SKILLHUB && (
            <button
              onClick={() => { setViewMode("skillhub"); setSearch(""); setSelectedCategory(null); }}
              className={`flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-bold transition-colors ${
                viewMode === "skillhub" ? "bg-surface text-accent shadow-sm" : "bg-surface-hover text-text-dim hover:text-text-main"
              }`}
            >
              <Store className="w-3.5 h-3.5" />
              {t("skills.skillhub")}
            </button>
          )}
        </div>

        {/* Category Chips */}
        {(viewMode === "marketplace" || viewMode === "skillhub" || viewMode === "fanghub") && (
          <div className="flex items-center gap-1 flex-wrap">
            <button
              onClick={() => { setSelectedCategory(null); }}
              className={`px-2.5 py-1 rounded-md text-xs font-bold transition-colors ${
                !selectedCategory ? "bg-brand text-white" : "text-text-dim hover:text-text-main border border-border-subtle"
              }`}
            >
              {t("common.all")}
            </button>
            {categories.map(cat => (
              <button
                key={cat.id}
                onClick={() => handleCategoryClick(cat.id)}
                className={`flex items-center gap-1 px-2.5 py-1 rounded-md text-xs font-bold transition-colors ${
                  selectedCategory === cat.id ? "bg-brand text-white" : "text-text-dim hover:text-text-main border border-border-subtle"
                }`}
              >
                {getCategoryIcon(cat.id)}
                {cat.name}
              </button>
            ))}
          </div>
        )}
      </div>

      {/* Search — ClawHub */}
      {viewMode === "marketplace" && (
        <Input
          value={search}
          onChange={(e) => { setSearch(e.target.value); setSelectedCategory(null); }}
          placeholder={selectedCategory ? categories.find(c => c.id === selectedCategory)?.name + "..." : t("skills.search_placeholder")}
          leftIcon={<Search className="w-4 h-4" />}
          rightIcon={search ? (
            <button onClick={() => setSearch("")} className="hover:text-text-main">
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
          <EmptyState title={t("skills.rate_limited")} description={t("skills.rate_limited_desc")} icon={<Loader2 className="h-6 w-6 animate-spin" />} />
        ) : marketplaceError ? (
          <EmptyState title={t("skills.load_error")} description={marketplaceError.message || t("common.error")} icon={<Search className="h-6 w-6" />} />
        ) : filteredMarketplace.length === 0 ? (
          <EmptyState title={t("skills.no_results")} icon={<Search className="h-6 w-6" />} />
        ) : (
          <div className="overflow-y-auto max-h-[600px] pr-1">
            <div className="grid gap-2 sm:gap-4 md:grid-cols-2 xl:grid-cols-3">
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
          <div className="grid gap-2 sm:gap-4 md:grid-cols-2 xl:grid-cols-3">
            {[1, 2, 3, 4, 5, 6].map(i => <CardSkeleton key={i} />)}
          </div>
        ) : isSkillhubRateLimited ? (
          <EmptyState title={t("skills.rate_limited")} description={t("skills.skillhub_rate_limited_desc")} icon={<Loader2 className="h-6 w-6 animate-spin" />} />
        ) : skillhubError ? (
          <EmptyState title={t("skills.load_error")} description={skillhubError.message || t("common.error")} icon={<Search className="h-6 w-6" />} />
        ) : filteredSkillhub.length === 0 ? (
          <EmptyState title={t("skills.no_results")} icon={<Search className="h-6 w-6" />} />
        ) : (
          <div className="overflow-y-auto max-h-[600px] pr-1">
            <div className="grid gap-2 sm:gap-4 md:grid-cols-2 xl:grid-cols-3">
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
          <CardSkeleton count={3} />
        ) : filteredFanghub.length === 0 ? (
          <EmptyState title={t("skills.no_results")} icon={<Zap className="h-6 w-6" />} />
        ) : (
          <div className="overflow-y-auto max-h-[600px] pr-1">
          <div className="grid gap-2 sm:gap-4 md:grid-cols-2 xl:grid-cols-3">
            {filteredFanghub.map((skill: FangHubSkill) => (
              <FangHubSkillCard
                key={skill.name}
                skill={skill}
                pendingId={installingId}
                onInstall={(name) => {
                  setInstallingId(name);
                  fanghubInstallMutation.mutate({ name });
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
    </div>
  );
}
