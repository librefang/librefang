import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  addMcpServer,
  updateMcpServer,
  deleteMcpServer,
  reconnectMcpServer,
  reloadMcp,
  startMcpAuth,
  revokeMcpAuth,
} from "../http/client";
import { mcpKeys } from "../queries/keys";

function invalidateMcpServer(qc: ReturnType<typeof useQueryClient>, id: string) {
  return Promise.all([
    qc.invalidateQueries({ queryKey: mcpKeys.servers() }),
    qc.invalidateQueries({ queryKey: mcpKeys.server(id) }),
    qc.invalidateQueries({ queryKey: mcpKeys.authStatus(id) }),
    qc.invalidateQueries({ queryKey: mcpKeys.health() }),
  ]);
}

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
    mutationFn: ({ id, server }: { id: string; server: Parameters<typeof updateMcpServer>[1] }) =>
      updateMcpServer(id, server),
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

export function useReconnectMcpServer() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: reconnectMcpServer,
    onSuccess: () => qc.invalidateQueries({ queryKey: mcpKeys.all }),
  });
}

export function useReloadMcp() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: reloadMcp,
    onSuccess: () => qc.invalidateQueries({ queryKey: mcpKeys.all }),
  });
}

export function useStartMcpAuth() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: startMcpAuth,
    onSuccess: (_data, id) => invalidateMcpServer(qc, id),
  });
}

export function useRevokeMcpAuth() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: revokeMcpAuth,
    onSuccess: (_data, id) => invalidateMcpServer(qc, id),
  });
}
