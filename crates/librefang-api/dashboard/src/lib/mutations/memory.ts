import { useMutation, useQueryClient } from "@tanstack/react-query";
import { addMemoryFromText, updateMemory, deleteMemory, cleanupMemories, updateMemoryConfig } from "../http/client";
import { memoryKeys } from "../queries/keys";

export function useAddMemory() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ content, level, agentId }: { content: string; level?: string; agentId?: string }) =>
      addMemoryFromText(content, { level, agentId }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: memoryKeys.lists() });
      qc.invalidateQueries({ queryKey: memoryKeys.statsAll() });
    },
  });
}

export function useUpdateMemory() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, content }: { id: string; content: string }) =>
      updateMemory(id, content),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: memoryKeys.lists() });
      qc.invalidateQueries({ queryKey: memoryKeys.statsAll() });
    },
  });
}

export function useDeleteMemory() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: deleteMemory,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: memoryKeys.lists() });
      qc.invalidateQueries({ queryKey: memoryKeys.statsAll() });
    },
  });
}

export function useCleanupMemories() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: cleanupMemories,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: memoryKeys.lists() });
      qc.invalidateQueries({ queryKey: memoryKeys.statsAll() });
    },
  });
}

export function useUpdateMemoryConfig() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: updateMemoryConfig,
    onSuccess: () => qc.invalidateQueries({ queryKey: memoryKeys.config() }),
  });
}
