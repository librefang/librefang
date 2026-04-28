import { formatBytes } from "../lib/format";
import { useState, useCallback, useEffect, useRef, startTransition } from "react";
import { useTranslation } from "react-i18next";
import type { PluginItem, RegistryEntry, RegistryPluginListing } from "../api";
import { usePlugins, usePluginRegistries } from "../lib/queries/plugins";
import {
  useInstallPlugin,
  useUninstallPlugin,
  useScaffoldPlugin,
  useInstallPluginDeps,
} from "../lib/mutations/plugins";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { PageHeader } from "../components/ui/PageHeader";
import { ListSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { DrawerPanel } from "../components/ui/DrawerPanel";
import { useUIStore } from "../lib/store";
import { useCreateShortcut } from "../lib/useCreateShortcut";
import {
  Puzzle, Plus, Download, Trash2, Package, FolderOpen,
  GitBranch, Loader2, Check, AlertCircle, FileCode
} from "lucide-react";
import { StaggerList } from "../components/ui/StaggerList";
import { AnimatePresence, motion } from "motion/react";
import { tabContent } from "../lib/motion";

const EMPTY_PLUGINS: PluginItem[] = [];
const EMPTY_REGISTRIES: RegistryEntry[] = [];

function getErrorMessage(e: unknown): string | null {
  if (e instanceof Error) {
    return e.message.trim() || null;
  }

  if (typeof e === "string") {
    return e.trim() || null;
  }

  return null;
}

export function PluginsPage() {
  const { t } = useTranslation();
  const [tab, setTab] = useState<"installed" | "registry">("installed");
  const [showInstall, setShowInstall] = useState(false);
  const [showScaffold, setShowScaffold] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  const [installingName, setInstallingName] = useState<string | null>(null);
  const [detailsPlugin, setDetailsPlugin] = useState<PluginItem | null>(null);
  const [detailsRegistryPlugin, setDetailsRegistryPlugin] = useState<{ rp: RegistryPluginListing; repo: string } | null>(null);
  const openInstall = useCallback(() => setShowInstall(true), []);
  useCreateShortcut(openInstall);

  // Install form
  const [installSource, setInstallSource] = useState<"registry" | "local" | "git">("registry");
  const [installName, setInstallName] = useState("");
  const [installPath, setInstallPath] = useState("");
  const [installUrl, setInstallUrl] = useState("");
  const [installBranch, setInstallBranch] = useState("");
  const [installRepo, setInstallRepo] = useState("");

  // Scaffold form
  const [scaffoldName, setScaffoldName] = useState("");
  const [scaffoldDesc, setScaffoldDesc] = useState("");
  const [scaffoldRuntime, setScaffoldRuntime] = useState("python");

  const pluginsQuery = usePlugins();
  const registriesQuery = usePluginRegistries({ enabled: tab === "registry" });

  const addToast = useUIStore((s) => s.addToast);
  const installMutation = useInstallPlugin();
  const uninstallMutation = useUninstallPlugin();
  const scaffoldMutation = useScaffoldPlugin();
  const depsMutation = useInstallPluginDeps();

  const plugins = pluginsQuery.data?.plugins ?? EMPTY_PLUGINS;

  // First-time visitors with nothing installed land on the marketplace
  // tab — installing from the registry is the obvious next step. Fires
  // once per mount; if the user manually flips back to "Installed", the
  // next refetch doesn't override.
  const autoSwitchedRef = useRef(false);
  useEffect(() => {
    if (autoSwitchedRef.current) return;
    if (!pluginsQuery.isSuccess) return;
    autoSwitchedRef.current = true;
    if (plugins.length === 0) setTab("registry");
  }, [pluginsQuery.isSuccess, plugins.length]);
  const registries = registriesQuery.data?.registries ?? EMPTY_REGISTRIES;

  const onRefresh = useCallback(() => {
    pluginsQuery.refetch();
    registriesQuery.refetch();
  }, [pluginsQuery.refetch, registriesQuery.refetch]);

  const resetInstallForm = useCallback(() => {
    setInstallName(""); setInstallPath(""); setInstallUrl(""); setInstallBranch(""); setInstallRepo("");
  }, []);

  const onInstallSuccess = useCallback(() => {
    setShowInstall(false);
    resetInstallForm();
    addToast(t("plugins.install_success", { defaultValue: "Plugin installed" }), "success");
  }, [addToast, resetInstallForm, t]);
  const onInstallError = useCallback((e: unknown) => {
    addToast(getErrorMessage(e) || t("plugins.install_failed", { defaultValue: "Install failed" }), "error");
  }, [addToast, t]);

  const handleInstall = useCallback(() => {
    const payload = {
      registry: { source: "registry" as const, name: installName, github_repo: installRepo || undefined },
      local: { source: "local" as const, path: installPath },
      git: { source: "git" as const, url: installUrl, branch: installBranch || undefined },
    };
    installMutation.mutate(payload[installSource], {
      onSuccess: onInstallSuccess,
      onError: onInstallError,
      onSettled: () => setInstallingName(null),
    });
  }, [installSource, installName, installRepo, installPath, installUrl, installBranch, installMutation.mutate, onInstallSuccess, onInstallError]);

  const handleRegistryInstall = useCallback((name: string, repo: string) => {
    setInstallingName(`${repo}:${name}`);
    installMutation.mutate(
      { source: "registry", name, github_repo: repo },
      {
        onSuccess: () => addToast(t("plugins.install_success", { defaultValue: "Plugin installed" }), "success"),
        onError: (e: unknown) => addToast(getErrorMessage(e) || t("plugins.install_failed", { defaultValue: "Install failed" }), "error"),
        onSettled: () => setInstallingName(null),
      },
    );
  }, [installMutation.mutate, addToast, t]);

  const handleDelete = useCallback((name: string) => {
    if (confirmDelete !== name) { setConfirmDelete(name); return; }
    uninstallMutation.mutate(name, {
      onSuccess: () => {
        setConfirmDelete(null);
        addToast(t("plugins.uninstall_success", { defaultValue: "Plugin removed" }), "success");
      },
      onError: (e: unknown) => addToast(getErrorMessage(e) || t("plugins.uninstall_failed", { defaultValue: "Uninstall failed" }), "error"),
    });
  }, [confirmDelete, uninstallMutation.mutate, addToast, t]);

  const inputClass = "w-full rounded-xl border border-border-subtle bg-main px-3 py-2 text-sm outline-none focus:border-brand focus:ring-1 focus:ring-brand/20";

  return (
    <div className="flex flex-col gap-6">
      <PageHeader
        badge={t("plugins.section")}
        title={t("plugins.title")}
        subtitle={t("plugins.subtitle")}
        isFetching={pluginsQuery.isFetching}
        onRefresh={onRefresh}
        icon={<Puzzle className="h-4 w-4" />}
        helpText={t("plugins.help")}
        actions={
          <div className="flex gap-2">
            <Button variant="secondary" onClick={() => setShowScaffold(true)}>
              <FileCode className="h-4 w-4" />
              <span className="hidden sm:inline">{t("plugins.new_plugin")}</span>
            </Button>
          </div>
        }
      />

      {/* Tabs */}
      <div className="flex gap-4 border-b border-border-subtle">
        <button onClick={() => startTransition(() => setTab("installed"))}
          className={`pb-2 text-sm font-bold transition-colors ${tab === "installed" ? "text-brand border-b-2 border-brand" : "text-text-dim hover:text-text"}`}>
          <Package className="w-4 h-4 inline mr-1.5" />
          {t("plugins.installed_tab")}
          {plugins.length > 0 && <Badge variant="default" className="ml-2">{plugins.length}</Badge>}
        </button>
        <button onClick={() => startTransition(() => setTab("registry"))}
          className={`pb-2 text-sm font-bold transition-colors ${tab === "registry" ? "text-brand border-b-2 border-brand" : "text-text-dim hover:text-text"}`}>
          <FolderOpen className="w-4 h-4 inline mr-1.5" />
          {t("plugins.registry_tab")}
        </button>
      </div>

      {/* Installed / Registry Tab content */}
      <AnimatePresence mode="wait">
      <motion.div key={tab} variants={tabContent} initial="initial" animate="animate" exit="exit">
      {tab === "installed" && (
        <div>
          {pluginsQuery.isLoading ? (
            <ListSkeleton rows={3} />
          ) : plugins.length === 0 ? (
            <EmptyState
              icon={<Puzzle className="w-7 h-7" />}
              title={t("plugins.no_plugins")}
              description={t("plugins.no_plugins_desc")}
            />
          ) : (
            <StaggerList className="space-y-2">
              {plugins.map(p => (
                <div
                  key={p.name}
                  className="flex items-center gap-3 p-3 sm:p-4 rounded-xl sm:rounded-2xl border border-border-subtle bg-surface hover:border-brand/30 transition-colors cursor-pointer"
                  role="button"
                  tabIndex={0}
                  onClick={() => setDetailsPlugin(p)}
                  onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); setDetailsPlugin(p); } }}
                >
                  <div className="w-9 h-9 sm:w-10 sm:h-10 rounded-lg sm:rounded-xl bg-brand/10 flex items-center justify-center shrink-0">
                    <Puzzle className="w-4 h-4 sm:w-5 sm:h-5 text-brand" />
                  </div>
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-1.5 sm:gap-2 flex-wrap">
                      <h3 className="text-xs sm:text-sm font-bold">{p.name}</h3>
                      <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-main text-text-dim font-mono">{p.version}</span>
                      {p.hooks?.ingest && <Badge variant="brand">ingest</Badge>}
                      {p.hooks?.after_turn && <Badge variant="brand">after_turn</Badge>}
                      {!p.hooks_valid && <Badge variant="error">invalid</Badge>}
                    </div>
                    <p className="text-[10px] text-text-dim mt-0.5">{p.description || "-"}</p>
                    <div className="flex items-center gap-3 mt-1 text-[9px] text-text-dim/50">
                      {p.author && <span>{p.author}</span>}
                      <span>{formatBytes(p.size_bytes)}</span>
                    </div>
                  </div>
                  <div className="flex items-center gap-1 shrink-0" onClick={e => e.stopPropagation()}>
                    <Button variant="secondary" size="sm"
                      onClick={() => depsMutation.mutate(p.name, {
                        onSuccess: () => addToast(t("plugins.deps_installed", { defaultValue: "Dependencies installed" }), "success"),
                        onError: (e: unknown) => addToast(getErrorMessage(e) || t("plugins.deps_failed", { defaultValue: "Dependency install failed" }), "error"),
                      })}
                      disabled={depsMutation.isPending}>
                      {depsMutation.isPending ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Download className="w-3.5 h-3.5" />}
                      <span className="hidden sm:inline">{t("plugins.deps")}</span>
                    </Button>
                    {confirmDelete === p.name ? (
                      <div className="flex items-center gap-1">
                        <button onClick={() => handleDelete(p.name)} className="px-2 py-1 rounded-lg bg-error text-white text-[10px] font-bold">{t("common.confirm")}</button>
                        <button onClick={() => setConfirmDelete(null)} className="px-2 py-1 rounded-lg bg-main text-text-dim text-[10px] font-bold">{t("common.cancel")}</button>
                      </div>
                    ) : (
                      <button onClick={() => handleDelete(p.name)} className="p-2 rounded-lg text-text-dim/30 hover:text-error hover:bg-error/10 transition-colors" aria-label={t("common.delete")}>
                        <Trash2 className="w-3.5 h-3.5" />
                      </button>
                    )}
                  </div>
                </div>
              ))}
            </StaggerList>
          )}
        </div>
      )}

      {/* Registry Tab */}
      {tab === "registry" && (
        <div>
          {registriesQuery.isLoading ? (
            <div className="flex items-center gap-2 text-text-dim text-sm py-8 justify-center">
              <Loader2 className="w-4 h-4 animate-spin" /> {t("plugins.loading_registries")}
            </div>
          ) : registries.length === 0 ? (
            <EmptyState
              icon={<Puzzle className="w-7 h-7" />}
              title={t("plugins.no_registries")}
            />
          ) : (
            <div className="space-y-8">
              {registries.map(reg => (
                <div key={reg.name}>
                  <div className="flex items-center gap-2 mb-3 flex-wrap">
                    <h3 className="text-sm font-bold">{reg.name}</h3>
                    <a
                      href={`https://github.com/${reg.github_repo}`}
                      target="_blank"
                      rel="noreferrer"
                      className="text-[10px] text-text-dim font-mono hover:text-brand transition-colors"
                    >
                      {reg.github_repo}
                    </a>
                    {reg.plugins.length > 0 && (
                      <Badge variant="default">{reg.plugins.length}</Badge>
                    )}
                    {reg.error && <Badge variant="error">{reg.error}</Badge>}
                  </div>
                  {reg.plugins.length === 0 ? (
                    <p className="text-xs text-text-dim italic">{t("plugins.no_available")}</p>
                  ) : (
                    <StaggerList className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
                      {reg.plugins.map(rp => (
                        <Card
                          key={rp.name}
                          padding="md"
                          className="flex flex-col gap-3 hover:border-brand/30 transition-colors cursor-pointer"
                          onClick={() => setDetailsRegistryPlugin({ rp, repo: reg.github_repo })}
                        >
                          <div className="flex items-start gap-3">
                            <div className="w-9 h-9 rounded-xl bg-brand/10 flex items-center justify-center shrink-0">
                              <Puzzle className="w-4 h-4 text-brand" />
                            </div>
                            <div className="min-w-0 flex-1">
                              <div className="flex items-center gap-1.5 flex-wrap">
                                <h4 className="text-sm font-bold truncate">{rp.name}</h4>
                                {rp.version && (
                                  <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-main text-text-dim font-mono">
                                    {rp.version}
                                  </span>
                                )}
                              </div>
                              {rp.author && (
                                <p className="text-[10px] text-text-dim/70 mt-0.5 truncate">
                                  {rp.author}
                                </p>
                              )}
                            </div>
                          </div>
                          <p
                            className="text-xs text-text-dim leading-relaxed overflow-hidden line-clamp-2"
                            style={{ minHeight: "2.25rem" }}
                          >
                            {rp.description || t("plugins.no_description", { defaultValue: "No description provided." })}
                          </p>
                          <div className="flex items-center justify-between gap-2 mt-auto">
                            <div className="flex items-center gap-1 flex-wrap min-w-0">
                              {(rp.hooks ?? []).slice(0, 3).map(h => (
                                <Badge key={h} variant="brand">{h}</Badge>
                              ))}
                              {(rp.hooks?.length ?? 0) > 3 && (
                                <span className="text-[9px] text-text-dim">+{(rp.hooks?.length ?? 0) - 3}</span>
                              )}
                            </div>
                            {rp.installed ? (
                              <Badge variant="success">
                                <Check className="w-3 h-3 mr-1" />
                                {t("plugins.installed")}
                              </Badge>
                            ) : (
                              <Button
                                variant="primary"
                                size="sm"
                                onClick={(e) => { e.stopPropagation(); handleRegistryInstall(rp.name, reg.github_repo); }}
                                disabled={installingName === `${reg.github_repo}:${rp.name}`}
                              >
                                {installingName === `${reg.github_repo}:${rp.name}`
                                  ? <Loader2 className="w-3.5 h-3.5 animate-spin" />
                                  : <Download className="w-3.5 h-3.5 mr-1" />}
                                {t("plugins.install")}
                              </Button>
                            )}
                          </div>
                        </Card>
                      ))}
                    </StaggerList>
                  )}
                </div>
              ))}
            </div>
          )}
        </div>
      )}
      </motion.div>
      </AnimatePresence>

      {/* Install Modal */}
      <DrawerPanel isOpen={showInstall} onClose={() => setShowInstall(false)} title={t("plugins.install_title")} size="md">
        <div className="p-5 space-y-4">
              {/* Source Tabs */}
              <div>
                <span className="text-[10px] font-bold text-text-dim uppercase">{t("plugins.source")}</span>
                <div className="flex gap-2 mt-1">
                  {(["registry", "local", "git"] as const).map(s => (
                    <button key={s} onClick={() => setInstallSource(s)}
                      className={`px-3 py-1.5 rounded-lg text-xs font-bold transition-colors ${installSource === s ? "bg-brand text-white" : "bg-main text-text-dim hover:text-text"}`}>
                      {s === "registry" && <FolderOpen className="w-3 h-3 inline mr-1" />}
                      {s === "local" && <Package className="w-3 h-3 inline mr-1" />}
                      {s === "git" && <GitBranch className="w-3 h-3 inline mr-1" />}
                      {t(`plugins.source_${s}`)}
                    </button>
                  ))}
                </div>
              </div>

              {installSource === "registry" && (
                <>
                  <div>
                    <label htmlFor="install-plugin-name" className="text-[10px] font-bold text-text-dim uppercase">{t("plugins.plugin_name")}</label>
                    <input id="install-plugin-name" value={installName} onChange={e => setInstallName(e.target.value)} className={inputClass} placeholder="e.g. echo-memory" />
                  </div>
                  <div>
                    <label htmlFor="install-plugin-repo" className="text-[10px] font-bold text-text-dim uppercase">{t("plugins.registry_optional")}</label>
                    <input id="install-plugin-repo" value={installRepo} onChange={e => setInstallRepo(e.target.value)} className={inputClass} placeholder={t("plugins.registry_placeholder")} />
                  </div>
                </>
              )}
              {installSource === "local" && (
                <div>
                  <label htmlFor="install-plugin-path" className="text-[10px] font-bold text-text-dim uppercase">{t("plugins.path")}</label>
                  <input id="install-plugin-path" value={installPath} onChange={e => setInstallPath(e.target.value)} className={inputClass} placeholder="/path/to/plugin" />
                </div>
              )}
              {installSource === "git" && (
                <>
                  <div>
                    <label htmlFor="install-plugin-url" className="text-[10px] font-bold text-text-dim uppercase">{t("plugins.url")}</label>
                    <input id="install-plugin-url" value={installUrl} onChange={e => setInstallUrl(e.target.value)} className={inputClass} placeholder="https://github.com/..." />
                  </div>
                  <div>
                    <label htmlFor="install-plugin-branch" className="text-[10px] font-bold text-text-dim uppercase">{t("plugins.branch")}</label>
                    <input id="install-plugin-branch" value={installBranch} onChange={e => setInstallBranch(e.target.value)} className={inputClass} placeholder="main" />
                  </div>
                </>
              )}

              {installMutation.error && (
                <div className="flex items-center gap-2 text-error text-xs">
                  <AlertCircle className="w-4 h-4 shrink-0" />
                  {getErrorMessage(installMutation.error) ?? t("plugins.install_failed", { defaultValue: "Install failed" })}
                </div>
              )}

              <div className="flex gap-2 pt-2">
                <Button variant="primary" className="flex-1" onClick={handleInstall} disabled={installMutation.isPending}>
                  {installMutation.isPending ? <Loader2 className="w-4 h-4 animate-spin mr-1" /> : <Download className="w-4 h-4 mr-1" />}
                  {t("plugins.install")}
                </Button>
                <Button variant="secondary" onClick={() => setShowInstall(false)}>{t("common.cancel")}</Button>
              </div>
        </div>
      </DrawerPanel>

      {/* Scaffold Modal */}
      <DrawerPanel isOpen={showScaffold} onClose={() => setShowScaffold(false)} title={t("plugins.scaffold_title")} size="sm">
        <div className="p-5 space-y-4">
          <div>
            <label htmlFor="scaffold-plugin-name" className="text-[10px] font-bold text-text-dim uppercase">{t("plugins.plugin_name")}</label>
            <input id="scaffold-plugin-name" value={scaffoldName} onChange={e => setScaffoldName(e.target.value)} className={inputClass} placeholder="my-plugin" disabled={scaffoldMutation.isPending} />
          </div>
          <div>
            <label htmlFor="scaffold-plugin-desc" className="text-[10px] font-bold text-text-dim uppercase">{t("plugins.description")}</label>
            <input id="scaffold-plugin-desc" value={scaffoldDesc} onChange={e => setScaffoldDesc(e.target.value)} className={inputClass} placeholder={t("plugins.scaffold_desc")} disabled={scaffoldMutation.isPending} />
          </div>
          <div>
            <label htmlFor="scaffold-plugin-runtime" className="text-[10px] font-bold text-text-dim uppercase">{t("plugins.runtime", { defaultValue: "Runtime" })}</label>
            <select id="scaffold-plugin-runtime" value={scaffoldRuntime} onChange={e => setScaffoldRuntime(e.target.value)} className={inputClass} disabled={scaffoldMutation.isPending}>
              <option value="python">Python</option>
              <option value="node">Node.js</option>
              <option value="deno">Deno (TypeScript)</option>
              <option value="bun">Bun (TypeScript)</option>
              <option value="go">Go</option>
              <option value="v">V (vlang)</option>
              <option value="ruby">Ruby</option>
              <option value="php">PHP</option>
              <option value="lua">Lua</option>
              <option value="bash">Bash</option>
              <option value="native">Native binary</option>
            </select>
          </div>
          <div className="flex gap-2 pt-2">
            <Button variant="primary" className="flex-1"
              onClick={() => scaffoldMutation.mutate(
                { name: scaffoldName, desc: scaffoldDesc, runtime: scaffoldRuntime },
                {
                  onSuccess: () => {
                    setShowScaffold(false);
                    setScaffoldName("");
                    setScaffoldDesc("");
                    setScaffoldRuntime("python");
                    addToast(t("plugins.scaffold_success", { defaultValue: "Plugin created" }), "success");
                  },
                  onError: (e: unknown) => addToast(getErrorMessage(e) || t("plugins.scaffold_failed", { defaultValue: "Create failed" }), "error"),
                },
              )}
              disabled={!scaffoldName.trim() || scaffoldMutation.isPending}>
              {scaffoldMutation.isPending ? <Loader2 className="w-4 h-4 animate-spin mr-1" /> : <Plus className="w-4 h-4 mr-1" />}
              {t("plugins.create")}
            </Button>
            <Button variant="secondary" onClick={() => setShowScaffold(false)}>{t("common.cancel")}</Button>
          </div>
        </div>
      </DrawerPanel>

      {/* Plugin detail drawer */}
      <DrawerPanel
        isOpen={!!detailsPlugin}
        onClose={() => setDetailsPlugin(null)}
        title={detailsPlugin?.name ?? ""}
        size="md"
      >
        {detailsPlugin && (
          <div className="p-5 space-y-5">
            {/* Hero */}
            <div className="flex items-start gap-3">
              <div className="w-12 h-12 rounded-xl bg-brand/10 flex items-center justify-center shrink-0">
                <Puzzle className="w-5 h-5 text-brand" />
              </div>
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2 flex-wrap">
                  <h2 className="text-lg font-black tracking-tight truncate">{detailsPlugin.name}</h2>
                  <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-main text-text-dim font-mono">v{detailsPlugin.version}</span>
                  {!detailsPlugin.hooks_valid && <Badge variant="error">{t("plugins.invalid")}</Badge>}
                </div>
                {detailsPlugin.author && (
                  <p className="text-[11px] text-text-dim/70 mt-0.5">{detailsPlugin.author}</p>
                )}
              </div>
            </div>

            {/* Description */}
            {detailsPlugin.description && (
              <p className="text-xs text-text-dim leading-relaxed whitespace-pre-wrap">{detailsPlugin.description}</p>
            )}

            {/* Hooks */}
            <div>
              <p className="text-[10px] font-black uppercase tracking-widest text-text-dim/60 mb-2">{t("plugins.hooks", { defaultValue: "Hooks" })}</p>
              <div className="flex flex-wrap gap-1.5">
                {detailsPlugin.hooks?.ingest && <Badge variant="brand">ingest</Badge>}
                {detailsPlugin.hooks?.after_turn && <Badge variant="brand">after_turn</Badge>}
                {!detailsPlugin.hooks?.ingest && !detailsPlugin.hooks?.after_turn && (
                  <span className="text-[11px] text-text-dim/50 italic">{t("common.none", { defaultValue: "none" })}</span>
                )}
              </div>
            </div>

            {/* Metadata */}
            <div>
              <p className="text-[10px] font-black uppercase tracking-widest text-text-dim/60 mb-2">{t("common.metadata", { defaultValue: "Metadata" })}</p>
              <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1.5 text-[11px]">
                <dt className="text-text-dim/70">{t("common.size", { defaultValue: "Size" })}</dt>
                <dd className="font-mono">{formatBytes(detailsPlugin.size_bytes)}</dd>
                {detailsPlugin.path && (
                  <>
                    <dt className="text-text-dim/70">{t("common.path", { defaultValue: "Path" })}</dt>
                    <dd className="font-mono break-all text-[10px]">{detailsPlugin.path}</dd>
                  </>
                )}
              </dl>
            </div>

            {/* Actions */}
            <div className="flex flex-wrap gap-2 pt-3 border-t border-border-subtle/50">
              <Button
                variant="secondary"
                size="sm"
                leftIcon={depsMutation.isPending ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Download className="w-3.5 h-3.5" />}
                disabled={depsMutation.isPending}
                onClick={() => depsMutation.mutate(detailsPlugin.name, {
                  onSuccess: () => addToast(t("plugins.deps_installed", { defaultValue: "Dependencies installed" }), "success"),
                  onError: (e: unknown) => addToast(getErrorMessage(e) || t("plugins.deps_failed", { defaultValue: "Dependency install failed" }), "error"),
                })}
              >
                {t("plugins.deps", { defaultValue: "Install deps" })}
              </Button>
              <Button
                variant="secondary"
                size="sm"
                leftIcon={<Trash2 className="w-3.5 h-3.5" />}
                className="!text-error hover:!bg-error/10"
                onClick={() => {
                  uninstallMutation.mutate(detailsPlugin.name, {
                    onSuccess: () => {
                      setDetailsPlugin(null);
                      addToast(t("plugins.uninstall_success", { defaultValue: "Plugin removed" }), "success");
                    },
                    onError: (e: unknown) => addToast(getErrorMessage(e) || t("plugins.uninstall_failed", { defaultValue: "Uninstall failed" }), "error"),
                  });
                }}
              >
                {t("common.delete")}
              </Button>
            </div>
          </div>
        )}
      </DrawerPanel>

      {/* Registry plugin detail drawer */}
      <DrawerPanel
        isOpen={!!detailsRegistryPlugin}
        onClose={() => setDetailsRegistryPlugin(null)}
        title={detailsRegistryPlugin?.rp.name ?? ""}
        size="md"
      >
        {detailsRegistryPlugin && (() => {
          const { rp, repo } = detailsRegistryPlugin;
          const installKey = `${repo}:${rp.name}`;
          return (
            <div className="p-5 space-y-5">
              <div className="flex items-start gap-3">
                <div className="w-12 h-12 rounded-xl bg-brand/10 flex items-center justify-center shrink-0 text-brand">
                  <Puzzle className="w-5 h-5" />
                </div>
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2 flex-wrap">
                    <h2 className="text-lg font-black tracking-tight truncate">{rp.name}</h2>
                    {rp.version && (
                      <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-main text-text-dim font-mono">v{rp.version}</span>
                    )}
                    {rp.installed && (
                      <Badge variant="success">
                        <Check className="w-3 h-3 mr-1" />
                        {t("plugins.installed")}
                      </Badge>
                    )}
                  </div>
                  {rp.author && (
                    <p className="text-[11px] text-text-dim/70 mt-0.5">{rp.author}</p>
                  )}
                  <a
                    href={`https://github.com/${repo}`}
                    target="_blank"
                    rel="noreferrer"
                    className="inline-flex items-center gap-1 mt-1 text-[11px] font-mono text-text-dim/70 hover:text-brand transition-colors"
                  >
                    <GitBranch className="w-3 h-3" />
                    {repo}
                  </a>
                </div>
              </div>

              {rp.description && (
                <p className="text-sm text-text-dim leading-relaxed whitespace-pre-wrap">{rp.description}</p>
              )}

              {(rp.hooks ?? []).length > 0 && (
                <div>
                  <p className="text-[10px] font-black uppercase tracking-widest text-text-dim/60 mb-2">
                    {t("plugins.hooks", { defaultValue: "Hooks" })}
                  </p>
                  <div className="flex flex-wrap gap-1.5">
                    {(rp.hooks ?? []).map(h => (
                      <Badge key={h} variant="brand">{h}</Badge>
                    ))}
                  </div>
                </div>
              )}

              <div className="pt-3 border-t border-border-subtle/50">
                {rp.installed ? (
                  <Button variant="secondary" className="w-full" disabled leftIcon={<Check className="w-4 h-4" />}>
                    {t("plugins.installed")}
                  </Button>
                ) : (
                  <Button
                    variant="primary"
                    className="w-full"
                    disabled={installingName === installKey}
                    leftIcon={installingName === installKey ? <Loader2 className="w-4 h-4 animate-spin" /> : <Download className="w-4 h-4" />}
                    onClick={() => handleRegistryInstall(rp.name, repo)}
                  >
                    {installingName === installKey ? t("common.loading") : t("plugins.install")}
                  </Button>
                )}
              </div>
            </div>
          );
        })()}
      </DrawerPanel>
    </div>
  );
}
