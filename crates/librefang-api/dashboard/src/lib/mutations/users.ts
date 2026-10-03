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
  uploadUserAvatar,
  deleteUserAvatar,
  updateUserIdentity,
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

// --- Avatar image and emoji (#8339) ------------------------------------------

/**
 * POST /api/users/{name}/avatar — store an image as this user's avatar.
 *
 * Raw bytes with no multipart and no filename, like the agent route: the daemon
 * sniffs the format, and the file it writes is named after a UUID it derives
 * from the name rather than after anything a caller sent.
 *
 * The avatar key is invalidated in `onSuccess` rather than `onSettled` for the
 * reason `useUploadAgentAvatar` documents: the handler places the new image
 * before clearing the superseded ones, so an upload that fails leaves the
 * previous picture exactly as it was, and refetching after one would only
 * arrive back at the bytes already cached.
 */
export function useUploadUserAvatar() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ name, file }: { name: string; file: Blob }) =>
      uploadUserAvatar(name, file),
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: userKeys.avatar(variables.name) });
      // The reads that report whether this user has a picture. It is not a
      // stored field — it is the file's existence — so the moment the file
      // changes, every cached copy of that answer is wrong.
      qc.invalidateQueries({ queryKey: userKeys.detail(variables.name) });
      qc.invalidateQueries({ queryKey: userKeys.lists() });
      // Neither of those can actually carry that answer: `UserItem` declares no
      // `has_avatar`, so the detail and list responses the two lines above
      // refresh have nothing to say about it. The read that does report it is
      // `whoami`, and the appearance section drives its buttons straight off
      // that. Left stale, it draws the picture the upload just stored and still
      // offers "Upload" with no Remove — the operator has to close and reopen
      // the drawer to remove what they can already see.
      qc.invalidateQueries({ queryKey: authzKeys.whoami() });
    },
  });
}

/**
 * DELETE /api/users/{name}/avatar — drop the image.
 *
 * The avatar key is **removed** rather than invalidated, exactly as
 * `useDeleteAgentAvatar` does: a refetch after a delete spends a request to be
 * told there is nothing there, and the answer to "is there a picture" is
 * already known.
 */
export function useDeleteUserAvatar() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (name: string) => deleteUserAvatar(name),
    onSuccess: (_data, name) => {
      qc.removeQueries({ queryKey: userKeys.avatar(name) });
      qc.invalidateQueries({ queryKey: userKeys.detail(name) });
      qc.invalidateQueries({ queryKey: userKeys.lists() });
      // Same reason as the upload above: `whoami` is the only read that reports
      // whether a picture exists, and the section's controls read it. Without
      // this they keep offering Remove for a picture that is no longer there.
      qc.invalidateQueries({ queryKey: authzKeys.whoami() });
    },
  });
}

/**
 * PATCH /api/users/{name}/identity — the user's emoji.
 *
 * Not partial, unlike the agent twin: the daemon documents `emoji` as "absent is
 * treated as `null`" and assigns the validated value straight onto the row, so
 * a body that omits the key clears the glyph rather than leaving it alone.
 *
 * This is the one avatar write that goes through the config file. The daemon
 * rewrites the `[[users]]` table with `toml_edit` — comments and unrelated
 * sections preserved — backs the file up, validates it and reloads the kernel
 * before this resolves. The image above never touches it: an image is a file.
 */
export function useUpdateUserIdentity() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ name, emoji }: { name: string; emoji?: string }) =>
      updateUserIdentity(name, { emoji }),
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: userKeys.detail(variables.name) });
      qc.invalidateQueries({ queryKey: userKeys.lists() });
      // The chat bubble draws the caller's emoji out of `whoami`, which is a
      // different key from the two above and which no user mutation would
      // otherwise touch.
      //
      // Invalidated unconditionally rather than only when the edited name turns
      // out to be the caller's: this hook is addressed by name and has no way to
      // know whose name it was handed, and the cost of guessing wrong is one
      // small request rather than a bubble that goes on showing the old emoji.
      // The two image writes do the same, for the same underlying reason: both
      // `emoji` and `has_avatar` are answers only this read carries.
      qc.invalidateQueries({ queryKey: authzKeys.whoami() });
    },
  });
}
