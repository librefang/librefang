import { useMutation, useQueryClient } from "@tanstack/react-query";
import { installPlugin, uninstallPlugin, scaffoldPlugin, installPluginDeps } from "../http/client";
import { pluginKeys } from "../queries/keys";

// Install / uninstall / scaffold all change the *installed* state of a
// plugin. The Marketplace tab's `rp.installed` flag is derived server-side
// in `pluginKeys.registries()`, NOT `pluginKeys.lists()` — those are two
// sibling keys under `pluginKeys.all`, so invalidating only `lists()`
// leaves the Marketplace tab showing the stale "Install" button after a
// successful install. Invalidate the whole domain to keep both views in
// sync.
export function useInstallPlugin() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: installPlugin,
    onSuccess: () => qc.invalidateQueries({ queryKey: pluginKeys.all }),
  });
}

export function useUninstallPlugin() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: uninstallPlugin,
    onSuccess: () => qc.invalidateQueries({ queryKey: pluginKeys.all }),
  });
}

export function useScaffoldPlugin() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ name, desc, runtime }: { name: string; desc: string; runtime?: string }) =>
      scaffoldPlugin(name, desc, runtime),
    onSuccess: () => qc.invalidateQueries({ queryKey: pluginKeys.all }),
  });
}

export function useInstallPluginDeps() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: installPluginDeps,
    onSuccess: () => qc.invalidateQueries({ queryKey: pluginKeys.all }),
  });
}
