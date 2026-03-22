import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  listPlugins, listPluginRegistries, installPlugin, uninstallPlugin,
  scaffoldPlugin, installPluginDeps,
} from "../api";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Badge } from "../components/ui/Badge";
import { PageHeader } from "../components/ui/PageHeader";
import { ListSkeleton } from "../components/ui/Skeleton";
import {
  Puzzle, Plus, Download, Trash2, Package, FolderOpen,
  GitBranch, X, Loader2, Check, AlertCircle, FileCode
} from "lucide-react";

const REFRESH_MS = 30000;

export function PluginsPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [tab, setTab] = useState<"installed" | "registry">("installed");
  const [showInstall, setShowInstall] = useState(false);
  const [showScaffold, setShowScaffold] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);

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

  const pluginsQuery = useQuery({ queryKey: ["plugins"], queryFn: listPlugins, refetchInterval: REFRESH_MS });
  const registriesQuery = useQuery({ queryKey: ["plugins", "registries"], queryFn: listPluginRegistries, enabled: tab === "registry" });

  const installMutation = useMutation({
    mutationFn: installPlugin,
    onSuccess: () => { queryClient.invalidateQueries({ queryKey: ["plugins"] }); setShowInstall(false); resetInstallForm(); }
  });
  const uninstallMutation = useMutation({
    mutationFn: uninstallPlugin,
    onSuccess: () => { queryClient.invalidateQueries({ queryKey: ["plugins"] }); setConfirmDelete(null); }
  });
  const scaffoldMutation = useMutation({
    mutationFn: ({ name, desc }: { name: string; desc: string }) => scaffoldPlugin(name, desc),
    onSuccess: () => { queryClient.invalidateQueries({ queryKey: ["plugins"] }); setShowScaffold(false); setScaffoldName(""); setScaffoldDesc(""); }
  });
  const depsMutation = useMutation({ mutationFn: installPluginDeps });

  const plugins = pluginsQuery.data?.plugins ?? [];
  const registries = registriesQuery.data?.registries ?? [];

  const resetInstallForm = () => {
    setInstallName(""); setInstallPath(""); setInstallUrl(""); setInstallBranch(""); setInstallRepo("");
  };

  const handleInstall = () => {
    if (installSource === "registry") {
      installMutation.mutate({ source: "registry", name: installName, github_repo: installRepo || undefined });
    } else if (installSource === "local") {
      installMutation.mutate({ source: "local", path: installPath });
    } else {
      installMutation.mutate({ source: "git", url: installUrl, branch: installBranch || undefined });
    }
  };

  const handleRegistryInstall = (name: string, repo: string) => {
    installMutation.mutate({ source: "registry", name, github_repo: repo });
  };

  const handleDelete = (name: string) => {
    if (confirmDelete !== name) { setConfirmDelete(name); return; }
    setConfirmDelete(null);
    uninstallMutation.mutate(name);
  };

  const formatSize = (bytes: number) => {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  };

  const inputClass = "w-full rounded-xl border border-border-subtle bg-main px-3 py-2 text-sm outline-none focus:border-brand focus:ring-1 focus:ring-brand/20";

  return (
    <div className="flex flex-col gap-6">
      <PageHeader
        badge={t("plugins.section")}
        title={t("plugins.title")}
        subtitle={t("plugins.subtitle")}
        isFetching={pluginsQuery.isFetching}
        onRefresh={() => { pluginsQuery.refetch(); registriesQuery.refetch(); }}
        icon={<Puzzle className="h-4 w-4" />}
        actions={
          <div className="flex gap-2">
            <Button variant="secondary" onClick={() => setShowScaffold(true)}>
              <FileCode className="h-4 w-4" />
              <span className="hidden sm:inline">{t("plugins.new_plugin")}</span>
            </Button>
            <Button variant="primary" onClick={() => setShowInstall(true)}>
              <Download className="h-4 w-4" />
              <span className="hidden sm:inline">{t("plugins.install")}</span>
            </Button>
          </div>
        }
      />

      {/* Tabs */}
      <div className="flex gap-4 border-b border-border-subtle">
        <button onClick={() => setTab("installed")}
          className={`pb-2 text-sm font-bold transition-colors ${tab === "installed" ? "text-brand border-b-2 border-brand" : "text-text-dim hover:text-text"}`}>
          <Package className="w-4 h-4 inline mr-1.5" />
          {t("plugins.installed_tab")}
          <Badge variant="default" className="ml-2">{plugins.length}</Badge>
        </button>
        <button onClick={() => setTab("registry")}
          className={`pb-2 text-sm font-bold transition-colors ${tab === "registry" ? "text-brand border-b-2 border-brand" : "text-text-dim hover:text-text"}`}>
          <FolderOpen className="w-4 h-4 inline mr-1.5" />
          {t("plugins.registry_tab")}
        </button>
      </div>

      {/* Installed Tab */}
      {tab === "installed" && (
        <div>
          {pluginsQuery.isLoading ? (
            <ListSkeleton rows={3} />
          ) : plugins.length === 0 ? (
            <div className="text-center py-16">
              <div className="w-14 h-14 rounded-2xl bg-brand/10 flex items-center justify-center mx-auto mb-4">
                <Puzzle className="w-7 h-7 text-brand" />
              </div>
              <h3 className="text-lg font-bold">{t("plugins.no_plugins")}</h3>
              <p className="text-sm text-text-dim mt-1">{t("plugins.no_plugins_desc")}</p>
            </div>
          ) : (
            <div className="space-y-2 stagger-children">
              {plugins.map(p => (
                <div key={p.name} className="flex flex-col sm:flex-row items-start sm:items-center gap-3 sm:gap-4 p-3 sm:p-4 rounded-2xl border border-border-subtle bg-surface hover:border-brand/30 transition-all">
                  <div className="w-10 h-10 rounded-xl bg-brand/10 flex items-center justify-center shrink-0">
                    <Puzzle className="w-5 h-5 text-brand" />
                  </div>
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2 flex-wrap">
                      <h3 className="text-sm font-bold">{p.name}</h3>
                      <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-main text-text-dim font-mono">{p.version}</span>
                      {p.hooks?.ingest && <Badge variant="brand">ingest</Badge>}
                      {p.hooks?.after_turn && <Badge variant="brand">after_turn</Badge>}
                      {!p.hooks_valid && <Badge variant="error">invalid</Badge>}
                    </div>
                    <p className="text-[10px] text-text-dim mt-0.5">{p.description || "-"}</p>
                    <div className="flex items-center gap-3 mt-1 text-[9px] text-text-dim/50">
                      {p.author && <span>{p.author}</span>}
                      <span>{formatSize(p.size_bytes)}</span>
                    </div>
                  </div>
                  <div className="flex items-center gap-1 shrink-0 self-end sm:self-auto" onClick={e => e.stopPropagation()}>
                    <Button variant="secondary" size="sm"
                      onClick={() => depsMutation.mutate(p.name)}
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
                      <button onClick={() => handleDelete(p.name)} className="p-2 rounded-lg text-text-dim/30 hover:text-error hover:bg-error/10 transition-all">
                        <Trash2 className="w-3.5 h-3.5" />
                      </button>
                    )}
                  </div>
                </div>
              ))}
            </div>
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
            <div className="text-center py-16">
              <p className="text-sm text-text-dim">{t("plugins.no_registries")}</p>
            </div>
          ) : (
            <div className="space-y-8">
              {registries.map(reg => (
                <div key={reg.name}>
                  <div className="flex items-center gap-2 mb-3">
                    <h3 className="text-sm font-bold">{reg.name}</h3>
                    <span className="text-[10px] text-text-dim font-mono">{reg.github_repo}</span>
                    {reg.error && <Badge variant="error">{reg.error}</Badge>}
                  </div>
                  {reg.plugins.length === 0 ? (
                    <p className="text-xs text-text-dim italic">{t("plugins.no_available")}</p>
                  ) : (
                    <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
                      {reg.plugins.map(rp => (
                        <Card key={rp.name} padding="sm" className="flex items-center justify-between">
                          <div className="flex items-center gap-3">
                            <div className="w-8 h-8 rounded-lg bg-brand/10 flex items-center justify-center">
                              <Puzzle className="w-4 h-4 text-brand" />
                            </div>
                            <span className="text-sm font-bold">{rp.name}</span>
                          </div>
                          {rp.installed ? (
                            <Badge variant="success"><Check className="w-3 h-3 mr-1" />{t("plugins.installed")}</Badge>
                          ) : (
                            <Button variant="primary" size="sm"
                              onClick={() => handleRegistryInstall(rp.name, reg.github_repo)}
                              disabled={installMutation.isPending}>
                              {installMutation.isPending ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Download className="w-3.5 h-3.5 mr-1" />}
                              {t("plugins.install")}
                            </Button>
                          )}
                        </Card>
                      ))}
                    </div>
                  )}
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      {/* Install Modal */}
      {showInstall && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 backdrop-blur-xl backdrop-saturate-150" onClick={() => setShowInstall(false)}>
          <div className="bg-surface rounded-2xl shadow-2xl border border-border-subtle w-[440px] max-w-[90vw] animate-fade-in-scale" onClick={e => e.stopPropagation()}>
            <div className="flex items-center justify-between px-5 py-3 border-b border-border-subtle">
              <h3 className="text-sm font-bold">{t("plugins.install_title")}</h3>
              <button onClick={() => setShowInstall(false)} className="p-1 rounded hover:bg-main"><X className="w-4 h-4" /></button>
            </div>
            <div className="p-5 space-y-4">
              {/* Source Tabs */}
              <div>
                <label className="text-[10px] font-bold text-text-dim uppercase">{t("plugins.source")}</label>
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
                    <label className="text-[10px] font-bold text-text-dim uppercase">{t("plugins.plugin_name")}</label>
                    <input value={installName} onChange={e => setInstallName(e.target.value)} className={inputClass} placeholder="e.g. echo-memory" />
                  </div>
                  <div>
                    <label className="text-[10px] font-bold text-text-dim uppercase">{t("plugins.registry_optional")}</label>
                    <input value={installRepo} onChange={e => setInstallRepo(e.target.value)} className={inputClass} placeholder={t("plugins.registry_placeholder")} />
                  </div>
                </>
              )}
              {installSource === "local" && (
                <div>
                  <label className="text-[10px] font-bold text-text-dim uppercase">{t("plugins.path")}</label>
                  <input value={installPath} onChange={e => setInstallPath(e.target.value)} className={inputClass} placeholder="/path/to/plugin" />
                </div>
              )}
              {installSource === "git" && (
                <>
                  <div>
                    <label className="text-[10px] font-bold text-text-dim uppercase">{t("plugins.url")}</label>
                    <input value={installUrl} onChange={e => setInstallUrl(e.target.value)} className={inputClass} placeholder="https://github.com/..." />
                  </div>
                  <div>
                    <label className="text-[10px] font-bold text-text-dim uppercase">{t("plugins.branch")}</label>
                    <input value={installBranch} onChange={e => setInstallBranch(e.target.value)} className={inputClass} placeholder="main" />
                  </div>
                </>
              )}

              {installMutation.error && (
                <div className="flex items-center gap-2 text-error text-xs">
                  <AlertCircle className="w-4 h-4 shrink-0" />
                  {(installMutation.error as any)?.message || String(installMutation.error)}
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
          </div>
        </div>
      )}

      {/* Scaffold Modal */}
      {showScaffold && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 backdrop-blur-xl backdrop-saturate-150" onClick={() => setShowScaffold(false)}>
          <div className="bg-surface rounded-2xl shadow-2xl border border-border-subtle w-[400px] max-w-[90vw] animate-fade-in-scale" onClick={e => e.stopPropagation()}>
            <div className="flex items-center justify-between px-5 py-3 border-b border-border-subtle">
              <h3 className="text-sm font-bold">{t("plugins.scaffold_title")}</h3>
              <button onClick={() => setShowScaffold(false)} className="p-1 rounded hover:bg-main"><X className="w-4 h-4" /></button>
            </div>
            <div className="p-5 space-y-4">
              <div>
                <label className="text-[10px] font-bold text-text-dim uppercase">{t("plugins.plugin_name")}</label>
                <input value={scaffoldName} onChange={e => setScaffoldName(e.target.value)} className={inputClass} placeholder="my-plugin" />
              </div>
              <div>
                <label className="text-[10px] font-bold text-text-dim uppercase">{t("plugins.description")}</label>
                <input value={scaffoldDesc} onChange={e => setScaffoldDesc(e.target.value)} className={inputClass} placeholder={t("plugins.scaffold_desc")} />
              </div>
              <div className="flex gap-2 pt-2">
                <Button variant="primary" className="flex-1"
                  onClick={() => scaffoldMutation.mutate({ name: scaffoldName, desc: scaffoldDesc })}
                  disabled={!scaffoldName.trim() || scaffoldMutation.isPending}>
                  {scaffoldMutation.isPending ? <Loader2 className="w-4 h-4 animate-spin mr-1" /> : <Plus className="w-4 h-4 mr-1" />}
                  {t("plugins.create")}
                </Button>
                <Button variant="secondary" onClick={() => setShowScaffold(false)}>{t("common.cancel")}</Button>
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
