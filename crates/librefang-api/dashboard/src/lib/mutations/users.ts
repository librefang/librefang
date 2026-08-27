// User RBAC mutations (Phase 4 / RBAC M6).
//
// Every write invalidates the `userKeys.lists()` shared list cache plus
// the affected detail cache. Bulk import dirties the whole `userKeys.all`
// subtree because the import can touch arbitrary rows; that's the exact
// "bulk reset" case AGENTS.md calls out as a legitimate `all` invalidation.
//
// Writes that change user configuration also reconcile
// `authzKeys.effective(name)` because the permission simulator derives its
// snapshot from the same `UserConfig` row (#3228 follow-up). API-key rotation
// is the intentional exception: it changes authentication credentials, not
// role, policy, bindings, or any simulator input.

import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  createUser,
  updateUser,
  deleteUser,
  importUsers,
  rotateUserKey,
  updateUserPolicy,
  type UserUpsertPayload,
  type PermissionPolicyUpdate,
  type BulkImportResult,
  type RotateUserKeyResponse,
} from "../http/client";
import {
  userKeys,
  permissionPolicyKeys,
  authzKeys,
  groupKeys,
} from "../queries/keys";

export function useCreateUser() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (payload: UserUpsertPayload) => createUser(payload),
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: userKeys.lists() });
      // The new user is immediately simulatable — drop any cached
      // "user not found" 404 from a prior lookup of the same name.
      qc.invalidateQueries({ queryKey: userKeys.detail(variables.name) });
      qc.invalidateQueries({ queryKey: authzKeys.effective(variables.name) });
      // A group can list a member before that member has a user row; creating
      // the row flips `unknown_members` on every group that named them.
      qc.invalidateQueries({ queryKey: groupKeys.all });
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
      const renamed = variables.payload.name !== variables.originalName;
      // `updateUser` can change `role` and `channel_bindings`, both of
      // which feed the effective-permissions snapshot. A rename ends the old
      // identity, so evict its detail/simulator entries rather than refetching
      // endpoints that now return 404.
      if (renamed) {
        qc.removeQueries({ queryKey: userKeys.detail(variables.originalName) });
        qc.removeQueries({ queryKey: authzKeys.effective(variables.originalName) });
        qc.invalidateQueries({ queryKey: userKeys.detail(variables.payload.name) });
        qc.invalidateQueries({
          queryKey: authzKeys.effective(variables.payload.name),
        });
        // The daemon carries a rename through every group's membership list
        // in the same config write, so the cached group rows still name the
        // old member.
        qc.invalidateQueries({ queryKey: groupKeys.all });
      } else {
        qc.invalidateQueries({ queryKey: userKeys.detail(variables.originalName) });
        qc.invalidateQueries({
          queryKey: authzKeys.effective(variables.originalName),
        });
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
      // The simulator should stop showing the deleted user immediately;
      // remove the snapshot rather than invalidate so a refetch doesn't
      // race a now-404 endpoint.
      qc.removeQueries({ queryKey: authzKeys.effective(name) });
      // The daemon strips a deleted user from every group in the same config
      // write (#7745), so the cached group rows and the per-user reverse
      // lookups are both stale the moment this resolves.
      qc.invalidateQueries({ queryKey: groupKeys.all });
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
      // Bulk import can rewrite roles, policies, and channel bindings on
      // arbitrary users — sweep the entire effective-permissions subtree.
      qc.invalidateQueries({ queryKey: authzKeys.all });
      // Import can create rows for names that groups already list, which
      // clears their `unknown_members` flag.
      qc.invalidateQueries({ queryKey: groupKeys.all });
    },
  });
}

// API-key rotation (RBAC follow-up to #3054 / M3 / M6). Owner-only on
// the daemon — non-Owner callers get a 403 surfaced through the mutation
// error path. The response contains the new plaintext key, which the UI
// must show exactly once (server can't reproduce it later); the dashboard
// itself never persists the value.
//
// Server-side, a successful rotation also swaps the live `user_api_keys`
// snapshot the auth middleware reads from, so any other tab still
// authenticated with the OLD key will start getting 401s on the next
// request. The dashboard doesn't track sessions independently — refreshing
// the user list is enough to surface the change.
export function useRotateUserKey() {
  const qc = useQueryClient();
  return useMutation<RotateUserKeyResponse, Error, string>({
    mutationFn: (name: string) => rotateUserKey(name),
    onSuccess: (_data, name) => {
      qc.invalidateQueries({ queryKey: userKeys.lists() });
      qc.invalidateQueries({ queryKey: userKeys.detail(name) });
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
      // Policy edits change every per-user slice the simulator surfaces:
      // tool_policy, tool_categories, memory_access, channel_tool_rules.
      qc.invalidateQueries({
        queryKey: authzKeys.effective(variables.name),
      });
    },
  });
}
