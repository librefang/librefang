// User-group mutations (#7745).
//
// Invalidation rule for this domain: every write sweeps `groupKeys.all`.
// That looks coarse next to the users domain's targeted `lists()` + `detail()`
// pair, and it is deliberate — a group write changes two derived views at
// once. Adding alice to `oncall` changes the group row, the group list, AND
// the `/api/users/alice/groups` reverse lookup that resolves her role set, and
// the last one is keyed by *user*, not by group, so a group-keyed targeted
// invalidation would leave it stale. Since the whole domain is one small
// config section refetched in a single request, the precision is not worth the
// class of bug it invites.
//
// `authzKeys` is NOT invalidated here. Group-conferred roles do not yet reach
// the effective-permissions snapshot the simulator renders — #7746 is what
// connects the two ladders — so invalidating it would imply a dependency that
// does not exist. That invalidation belongs in the change that creates the
// dependency.

import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  createGroup,
  updateGroup,
  deleteGroup,
  addGroupMember,
  removeGroupMember,
  type GroupUpsertPayload,
} from "../http/client";
import { groupKeys } from "../queries/keys";

export function useCreateGroup() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (payload: GroupUpsertPayload) => createGroup(payload),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: groupKeys.all });
    },
  });
}

export function useUpdateGroup() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (vars: { originalName: string; payload: GroupUpsertPayload }) =>
      updateGroup(vars.originalName, vars.payload),
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: groupKeys.all });
      // A rename leaves the old detail key pointing at a name the daemon now
      // 404s on; remove it rather than invalidate so nothing refetches it.
      if (variables.payload.name !== variables.originalName) {
        qc.removeQueries({ queryKey: groupKeys.detail(variables.originalName) });
      }
    },
  });
}

export function useDeleteGroup() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (name: string) => deleteGroup(name),
    onSuccess: (_data, name) => {
      qc.removeQueries({ queryKey: groupKeys.detail(name) });
      qc.invalidateQueries({ queryKey: groupKeys.all });
    },
  });
}

export function useAddGroupMember() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (vars: { group: string; user: string }) =>
      addGroupMember(vars.group, vars.user),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: groupKeys.all });
    },
  });
}

export function useRemoveGroupMember() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (vars: { group: string; user: string }) =>
      removeGroupMember(vars.group, vars.user),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: groupKeys.all });
    },
  });
}
