import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  updateStorageConfig,
  migrateStorage,
  linkUarStorage,
  unlinkUarStorage,
} from "../http/client";
import type { LinkUarBody, StorageConfig } from "../http/client";
import { storageKeys } from "../queries/keys";
import { overviewKeys } from "../queries/keys";

/**
 * Persist a new storage backend configuration.
 *
 * Invalidates `storageKeys.all` and `overviewKeys.snapshot()` on success so
 * the Status panel and dashboard snapshot both reflect the change immediately.
 */
export function useUpdateStorageConfig() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (config: Partial<Omit<StorageConfig, "uar_linked">>) =>
      updateStorageConfig(config),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: storageKeys.all });
      qc.invalidateQueries({ queryKey: overviewKeys.snapshot() });
    },
  });
}

/**
 * Run a SQLite → SurrealDB migration.
 *
 * Pass `{ from: "sqlite", dry_run: true }` for a row-count preview.
 * Invalidates `storageKeys.all` on success so the Status panel updates.
 */
export function useMigrateStorage() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (body: { from: "sqlite"; dry_run?: boolean }) =>
      migrateStorage(body),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: storageKeys.all });
    },
  });
}

/**
 * Provision a UAR namespace + application user on a remote SurrealDB and
 * write the `[uar.remote]` block into config.toml.
 *
 * Invalidates `storageKeys.all` and `overviewKeys.snapshot()` on success.
 */
export function useLinkUarStorage() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (body: LinkUarBody) => linkUarStorage(body),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: storageKeys.all });
      qc.invalidateQueries({ queryKey: overviewKeys.snapshot() });
    },
  });
}

/**
 * Remove the `[uar.remote]` / `share_librefang_storage` block from config.toml.
 *
 * Invalidates `storageKeys.all` and `overviewKeys.snapshot()` on success.
 */
export function useUnlinkUarStorage() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (body?: { purge_user?: boolean }) => unlinkUarStorage(body),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: storageKeys.all });
      qc.invalidateQueries({ queryKey: overviewKeys.snapshot() });
    },
  });
}
