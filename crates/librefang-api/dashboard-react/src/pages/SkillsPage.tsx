import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { listSkills, installSkill, uninstallSkill, clawhubSearch, clawhubBrowse, clawhubInstall, type ClawHubBrowseItem, type ClawHubSkillDetail } from "../api";
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
  Wrench, Search, CheckCircle2, XCircle, ChevronRight, X,
  Download, Trash2, Star, Tag, Loader2, Sparkles, Package, BookOpen,
  Code, GitBranch, Globe, Cloud, Monitor, Search as SearchIcon, Bot, Database,
  Briefcase, MessageCircle, Film, FileText, Shield, Terminal, TrendingUp, DollarSign, Home, FileCode
} from "lucide-react";

const REFRESH_MS = 30000;
const ITEMS_PER_PAGE = 6;

type ViewMode = "installed" | "marketplace";

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
function MarketplaceSkillCard({ skill, onInstall, pendingId, onViewDetails, t }: {
  skill: ClawHubBrowseItem;
  pendingId: string | null;
  onInstall: (slug: string) => void;
  onViewDetails: (skill: ClawHubBrowseItem) => void;
  t: (key: string) => string;
}) {
  return (
    <Card hover padding="none" className="flex flex-col overflow-hidden group">
      <div className="h-1.5 bg-gradient-to-r from-brand via-brand/60 to-brand/30" />
      <div className="p-5 flex-1 flex flex-col">
        <div className="flex items-start justify-between gap-3 mb-4">
          <div className="flex items-center gap-3 min-w-0">
            <div className="w-10 h-10 rounded-lg flex items-center justify-center text-xl bg-gradient-to-br from-brand/10 to-brand/5 border border-brand/20">
              <Sparkles className="w-5 h-5 text-brand" />
            </div>
            <div className="min-w-0">
              <h2 className="text-base font-black truncate group-hover:text-brand transition-colors">{skill.name}</h2>
              <p className="text-[10px] font-black uppercase tracking-widest text-text-dim/60 truncate">v{skill.version || "1.0.0"}</p>
            </div>
          </div>
          {skill.is_installed && <Badge variant="success">{t("skills.installed")}</Badge>}
        </div>
        <p className="text-xs text-text-dim line-clamp-2 italic mb-4 flex-1">{skill.description || "-"}</p>

        {/* Stats */}
        <div className="flex items-center gap-4 mb-4 text-[10px] font-bold text-text-dim">
          <span className="flex items-center gap-1">
            <Star className="w-3 h-3 text-warning" />
            {skill.stars}
          </span>
          <span className="flex items-center gap-1">
            <Download className="w-3 h-3" />
            {skill.downloads}
          </span>
        </div>

        {/* Actions */}
        <div className="flex gap-2 mt-auto">
          <Button variant="ghost" size="sm" className="flex-1" onClick={() => onViewDetails(skill)}>
            {t("common.details")}
            <ChevronRight className="w-3 h-3" />
          </Button>
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
              onClick={() => onInstall(skill.slug)}
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
function DetailsModal({ skill, onClose, onInstall, pendingId, t }: {
  skill: ClawHubBrowseItem;
  onClose: () => void;
  onInstall: () => void;
  pendingId: string | null;
  t: (key: string) => string;
}) {
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/50" onClick={onClose}>
      <div className="bg-surface rounded-2xl border border-border-subtle w-full max-w-lg shadow-2xl max-h-[90vh] overflow-y-auto" onClick={e => e.stopPropagation()}>
        <div className="h-2 bg-gradient-to-r from-brand via-brand/60 to-brand/30 rounded-t-2xl" />
        <div className="p-6 border-b border-border-subtle">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <div className="w-12 h-12 rounded-xl flex items-center justify-center text-2xl bg-brand/10 border border-brand/20">
                <Sparkles className="w-6 h-6 text-brand" />
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
            <span className="flex items-center gap-1">
              <Star className="w-4 h-4 text-warning" />
              {skill.stars} stars
            </span>
            <span className="flex items-center gap-1">
              <Download className="w-4 h-4" />
              {skill.downloads} downloads
            </span>
          </div>

          {skill.tags && skill.tags.length > 0 && (
            <div className="flex flex-wrap gap-2">
              {skill.tags.map(tag => (
                <span key={tag} className="px-2 py-1 rounded-lg bg-brand/10 text-brand text-xs font-bold">{tag}</span>
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
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50 backdrop-blur-sm" onClick={onClose}>
      <div className="bg-surface border border-border-subtle rounded-2xl w-full max-w-sm p-6 shadow-2xl" onClick={e => e.stopPropagation()}>
        <h3 className="text-lg font-black mb-2">{t("skills.uninstall_confirm_title")}</h3>
        <p className="text-sm text-text-dim mb-6">{t("skills.uninstall_confirm", { name: skillName })}</p>
        <div className="flex gap-3">
          <Button variant="secondary" className="flex-1" onClick={onClose}>{t("common.cancel")}</Button>
          <Button variant="primary" className="flex-1 !bg-error hover:!bg-error/90" onClick={onConfirm} disabled={isPending}>
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

  // View state
  const [viewMode, setViewMode] = useState<ViewMode>("marketplace");
  const [selectedCategory, setSelectedCategory] = useState<string>("coding");
  const [search, setSearch] = useState("");
  const [page, setPage] = useState(1);

  // Actions
  const [uninstalling, setUninstalling] = useState<string | null>(null);
  const [detailsSkill, setDetailsSkill] = useState<ClawHubBrowseItem | null>(null);
  const [installingId, setInstallingId] = useState<string | null>(null);

  // Get search keyword from category or use search input
  const searchKeyword = selectedCategory
    ? categories.find(c => c.id === selectedCategory)?.keyword || ""
    : search;

  // Queries
  const skillsQuery = useQuery({ queryKey: ["skills", "list"], queryFn: listSkills, refetchInterval: REFRESH_MS });

  // Use search API
  const searchQuery = useQuery({
    queryKey: ["clawhub", "search", searchKeyword],
    queryFn: () => clawhubSearch(searchKeyword || "python"),
    staleTime: 60000,
    enabled: viewMode === "marketplace" && !!searchKeyword,
  });

  const installedSkills = skillsQuery.data ?? [];
  const marketplaceSkills = searchQuery.data?.items ?? [];
  const isMarketplaceLoading = searchQuery.isLoading;
  const marketplaceError = searchQuery.error as any;
  const isRateLimited = marketplaceError?.message?.includes("429") || marketplaceError?.message?.includes("rate") || marketplaceError?.message?.includes("Rate limit") || marketplaceError?.status === 429;

  // Filter & paginate
  const filteredMarketplace = marketplaceSkills
    .map(s => ({
      ...s,
      is_installed: installedSkills.some(i => i.name === s.slug)
    }))
    .filter(s => !search || s.name.toLowerCase().includes(search.toLowerCase()) || s.description?.toLowerCase().includes(search.toLowerCase()));

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
    mutationKey: ["install", "skill"],
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
      addToast(msg.includes("abort") ? "Installation timed out" : msg, "error");
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

  const handleInstall = (slug: string) => {
    console.log("[Skills] Installing:", slug);
    setInstallingId(slug);
    installMutation.mutate({ slug }, {
      onSuccess: () => console.log("[Skills] Install success"),
      onError: (err) => console.log("[Skills] Install error:", err)
    });
  };

  const handleUninstall = (name: string) => {
    setUninstalling(name);
  };

  const confirmUninstall = () => {
    if (uninstalling) {
      uninstallMutation.mutate(uninstalling);
    }
  };

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("common.infrastructure")}
        title={t("skills.title")}
        subtitle={t("skills.subtitle")}
        isFetching={skillsQuery.isFetching || searchQuery.isFetching}
        onRefresh={() => { void skillsQuery.refetch(); void searchQuery.refetch(); }}
        icon={<Wrench className="h-4 w-4" />}
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
          className={`flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-bold transition-all ${
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
          onClick={() => { setViewMode("marketplace"); setPage(1); }}
          className={`flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-bold transition-all ${
            viewMode === "marketplace" ? "bg-surface text-brand shadow-sm" : "text-text-dim hover:text-text-main"
          }`}
        >
          <Sparkles className="w-4 h-4" />
          {t("skills.marketplace")}
        </button>
      </div>

      {/* Category Chips */}
      {viewMode === "marketplace" && (
        <div className="flex flex-wrap gap-2">
          {categories.map(cat => (
            <button
              key={cat.id}
              onClick={() => handleCategoryClick(cat.id)}
              className={`flex items-center gap-2 px-3 py-1.5 rounded-lg text-xs font-bold transition-all ${
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

      {/* Search */}
      {viewMode === "marketplace" && (
        <Input
          value={search}
          onChange={(e) => { setSearch(e.target.value); setSelectedCategory(null); setPage(1); }}
          placeholder={selectedCategory ? categories.find(c => c.id === selectedCategory)?.name + "..." : t("skills.search_placeholder")}
          leftIcon={<Search className="w-4 h-4" />}
          rightIcon={search ? (
            <button onClick={() => setSearch("")} className="hover:text-text-main">
              <X className="w-3 h-3" />
            </button>
          ) : undefined}
        />
      )}

      {/* Content */}
      {viewMode === "installed" ? (
        skillsQuery.isLoading ? (
          <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
            {[1, 2, 3, 4, 5, 6].map(i => <CardSkeleton key={i} />)}
          </div>
        ) : installedSkills.length === 0 ? (
          <EmptyState title={t("skills.no_skills")} icon={<Package className="h-6 w-6" />} />
        ) : (
          <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
            {installedSkills.map(s => (
              <InstalledSkillCard key={s.name} skill={s} onUninstall={handleUninstall} t={t} />
            ))}
          </div>
        )
      ) : (
        isMarketplaceLoading ? (
          <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
            {[1, 2, 3, 4, 5, 6].map(i => <CardSkeleton key={i} />)}
          </div>
        ) : isRateLimited ? (
          <EmptyState
            title={t("skills.rate_limited")}
            subtitle={t("skills.rate_limited_desc")}
            icon={<Loader2 className="h-6 w-6 animate-spin" />}
          />
        ) : marketplaceError ? (
          <EmptyState
            title={t("skills.load_error")}
            subtitle={marketplaceError.message || t("common.error")}
            icon={<Search className="h-6 w-6" />}
          />
        ) : filteredMarketplace.length === 0 ? (
          <EmptyState title={t("skills.no_results")} icon={<Search className="h-6 w-6" />} />
        ) : (
          <>
            <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
              {paginatedMarketplace.map(s => (
                <MarketplaceSkillCard
                  key={s.slug}
                  skill={s}
                  pendingId={installingId}
                  onInstall={handleInstall}
                  onViewDetails={setDetailsSkill}
                  t={t}
                />
              ))}
            </div>
            {totalPages > 1 && (
              <Pagination currentPage={page} totalPages={totalPages} onPageChange={setPage} />
            )}
          </>
        )
      )}

      {/* Details Modal */}
      {detailsSkill && (
        <DetailsModal
          skill={detailsSkill}
          onClose={() => setDetailsSkill(null)}
          onInstall={() => handleInstall(detailsSkill.slug)}
          pendingId={installingId}
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
