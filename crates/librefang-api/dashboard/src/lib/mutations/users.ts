// User RBAC mutations (Phase 4 / RBAC M6).
//
// Every write invalidates the `userKeys.lists()` shared list cache plus
// the affected detail cache. Bulk import dirties the whole `userKeys.all`
// subtree because the import can touch arbitrary rows; that's the exact
// "bulk reset" case AGENTS.md calls out as a legitimate `all` invalidation.

import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  createUser,
  updateUser,
  deleteUser,
  importUsers,
  updateUserPolicy,
  type UserUpsertPayload,
  type PermissionPolicyUpdate,
  type BulkImportResult,
} from "../http/client";
import {
  userKeys,
  permissionPolicyKeys,
} from "../queries/keys";

export function useCreateUser() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (payload: UserUpsertPayload) => createUser(payload),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: userKeys.lists() });
    },
  });
}

export function useUpdateUser() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (vars: { originalName: string; payload: UserUpsertPayload }) =>
      updateUser(vars.originalName, vars.payload),
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: userKeys.lists() });
      qc.invalidateQueries({ queryKey: userKeys.detail(variables.originalName) });
      // If the user renamed, also invalidate the new-name detail cache so
      // any open detail view falls through to a fresh fetch.
      if (variables.payload.name !== variables.originalName) {
        qc.invalidateQueries({ queryKey: userKeys.detail(variables.payload.name) });
      }
    },
  });
}

export function useDeleteUser() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (name: string) => deleteUser(name),
    onSuccess: (_data, name) => {
      qc.invalidateQueries({ queryKey: userKeys.lists() });
      qc.removeQueries({ queryKey: userKeys.detail(name) });
    },
  });
}

export function useImportUsers() {
  const qc = useQueryClient();
  return useMutation<
    BulkImportResult,
    Error,
    { rows: UserUpsertPayload[]; dryRun?: boolean }
  >({
    mutationFn: ({ rows, dryRun }) => importUsers(rows, { dryRun }),
    onSuccess: (data) => {
      // Dry run never mutates state — keep the cache as-is.
      if (data.dry_run) return;
      qc.invalidateQueries({ queryKey: userKeys.all });
    },
  });
}

// RBAC M3 (#3205) — per-user policy upsert. Invalidates the policy detail
// AND the user detail/list caches because policy fields are part of the
// `UserConfig` row and could surface in any user-listing widget that grows
// to render policy badges.
export function useUpdateUserPolicy() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (vars: { name: string; policy: PermissionPolicyUpdate }) =>
      updateUserPolicy(vars.name, vars.policy),
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({
        queryKey: permissionPolicyKeys.detail(variables.name),
      });
      qc.invalidateQueries({ queryKey: userKeys.detail(variables.name) });
      qc.invalidateQueries({ queryKey: userKeys.lists() });
    },
  });
}
