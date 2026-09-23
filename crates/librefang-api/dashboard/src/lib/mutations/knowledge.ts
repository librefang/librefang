// Knowledge-base mutations (#8327).
//
// Invalidation rule for this domain: a write that changes a base's *contents*
// sweeps that base's document list and the base list — the latter because each
// base row carries `document_count` and `total_bytes`, so leaving it alone
// shows a card that disagrees with the list open underneath it.
//
// A write that changes *sharing* also sweeps `agentKeys.all`. Granting a base
// edits the agent's manifest, which is what the agent detail view renders, so
// a knowledge-only invalidation would leave the agent screen claiming a
// workspace set the daemon no longer holds. That is the same one-sided
// staleness #8321 was about, arriving through the cache instead of the API.

import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  createKnowledgeBase,
  deleteKnowledgeBase,
  deleteKnowledgeDocument,
  putKnowledgeDocument,
  setKnowledgeHolders,
} from "../../api";
import { agentKeys, knowledgeKeys } from "../queries/keys";

export function useCreateKnowledgeBase() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (name: string) => createKnowledgeBase(name),
    onSuccess: () => {
      // Only the list: a new base is empty and cannot have changed any other
      // base's documents, so sweeping `all` would refetch every open panel.
      qc.invalidateQueries({ queryKey: knowledgeKeys.lists() });
    },
  });
}

export function useDeleteKnowledgeBase() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (name: string) => deleteKnowledgeBase(name),
    onSuccess: () => {
      // Deleting revokes the base from every holder, so the agent views change too.
      qc.invalidateQueries({ queryKey: knowledgeKeys.all });
      qc.invalidateQueries({ queryKey: agentKeys.all });
    },
  });
}

export function useUploadKnowledgeDocument() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (vars: { name: string; filename: string; file: Blob }) =>
      putKnowledgeDocument(vars.name, vars.filename, vars.file),
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: knowledgeKeys.documentsFor(variables.name) });
      qc.invalidateQueries({ queryKey: knowledgeKeys.lists() });
    },
  });
}

export function useDeleteKnowledgeDocument() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (vars: { name: string; filename: string }) =>
      deleteKnowledgeDocument(vars.name, vars.filename),
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: knowledgeKeys.documentsFor(variables.name) });
      qc.invalidateQueries({ queryKey: knowledgeKeys.lists() });
    },
  });
}

export function useSetKnowledgeHolders() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (vars: { name: string; agents: { agent_id: string; mode: "r" | "rw" }[] }) =>
      setKnowledgeHolders(vars.name, vars.agents),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: knowledgeKeys.all });
      qc.invalidateQueries({ queryKey: agentKeys.all });
    },
  });
}
