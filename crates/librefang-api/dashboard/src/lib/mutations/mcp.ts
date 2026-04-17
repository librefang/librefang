import { useMutation, useQueryClient } from "@tanstack/react-query";
import { addMcpServer, updateMcpServer, deleteMcpServer } from "../http/client";
import { mcpKeys } from "../queries/keys";

export function useAddMcpServer() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: addMcpServer,
    onSuccess: () => qc.invalidateQueries({ queryKey: mcpKeys.all }),
  });
}

export function useUpdateMcpServer() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ name, server }: { name: string; server: Parameters<typeof updateMcpServer>[1] }) =>
      updateMcpServer(name, server),
    onSuccess: () => qc.invalidateQueries({ queryKey: mcpKeys.all }),
  });
}

export function useDeleteMcpServer() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: deleteMcpServer,
    onSuccess: () => qc.invalidateQueries({ queryKey: mcpKeys.all }),
  });
}
