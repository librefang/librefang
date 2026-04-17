import { useMutation, useQueryClient } from "@tanstack/react-query";
import { addMemoryFromText, updateMemory, deleteMemory, cleanupMemories, updateMemoryConfig } from "../http/client";
import { memoryKeys } from "../queries/keys";

export function useAddMemory() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ content, agentId }: { content: string; agentId?: string }) =>
      addMemoryFromText(content, agentId),
    onSuccess: () => qc.invalidateQueries({ queryKey: memoryKeys.all }),
  });
}

export function useUpdateMemory() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, content }: { id: string; content: string }) =>
      updateMemory(id, content),
    onSuccess: () => qc.invalidateQueries({ queryKey: memoryKeys.all }),
  });
}

export function useDeleteMemory() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: deleteMemory,
    onSuccess: () => qc.invalidateQueries({ queryKey: memoryKeys.all }),
  });
}

export function useCleanupMemories() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: cleanupMemories,
    onSuccess: () => qc.invalidateQueries({ queryKey: memoryKeys.all }),
  });
}

export function useUpdateMemoryConfig() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: updateMemoryConfig,
    onSuccess: () => qc.invalidateQueries({ queryKey: memoryKeys.all }),
  });
}
