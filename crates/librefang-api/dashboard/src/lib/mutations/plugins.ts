import { useMutation, useQueryClient } from "@tanstack/react-query";
import { installPlugin, uninstallPlugin, scaffoldPlugin, installPluginDeps } from "../http/client";
import { pluginKeys } from "../queries/keys";

export function useInstallPlugin() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: installPlugin,
    onSuccess: () => qc.invalidateQueries({ queryKey: pluginKeys.lists() }),
  });
}

export function useUninstallPlugin() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: uninstallPlugin,
    onSuccess: () => qc.invalidateQueries({ queryKey: pluginKeys.lists() }),
  });
}

export function useScaffoldPlugin() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ name, desc, runtime }: { name: string; desc: string; runtime?: string }) =>
      scaffoldPlugin(name, desc, runtime),
    onSuccess: () => qc.invalidateQueries({ queryKey: pluginKeys.lists() }),
  });
}

export function useInstallPluginDeps() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: installPluginDeps,
    onSuccess: () => qc.invalidateQueries({ queryKey: pluginKeys.all }),
  });
}
