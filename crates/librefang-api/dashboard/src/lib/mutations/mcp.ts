import { useMutation, useQueryClient, type QueryClient } from "@tanstack/react-query";
import {
  addMcpServer,
  updateMcpServer,
  patchMcpServerTaint,
  deleteMcpServer,
  reconnectMcpServer,
  reloadMcp,
  startMcpAuth,
  revokeMcpAuth,
} from "../http/client";
import type { McpTaintPolicy } from "../../api";
import { mcpKeys } from "../queries/keys";

function invalidateMcpServer(qc: QueryClient, id: string) {
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
    onSuccess: () => qc.invalidateQueries({ queryKey: mcpKeys.servers() }),
  });
}

export function useUpdateMcpServer() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, server }: { id: string; server: Parameters<typeof updateMcpServer>[1] }) =>
      updateMcpServer(id, server),
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: mcpKeys.servers() });
      qc.invalidateQueries({ queryKey: mcpKeys.server(variables.id) });
    },
  });
}

export function useDeleteMcpServer() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: deleteMcpServer,
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: mcpKeys.servers() });
      qc.removeQueries({ queryKey: mcpKeys.server(variables) });
    },
  });
}

export function useReconnectMcpServer() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: reconnectMcpServer,
    onSuccess: (_data, variables) => {
      qc.invalidateQueries({ queryKey: mcpKeys.server(variables) });
      qc.invalidateQueries({ queryKey: mcpKeys.health() });
    },
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

/**
 * Issue #3050: dedicated mutation for the taint policy tree editor.
 *
 * Uses the dedicated `PATCH /api/mcp/servers/{id}/taint` endpoint, which
 * accepts only `{ taint_scanning?, taint_policy? }`. This avoids the
 * round-trip of every other server field (transport, env, headers, …) that
 * the older PUT-based path required, removing the silent-drop risk if a
 * future required field gets added without dashboard support.
 */
export function useUpdateMcpTaintPolicy() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      id,
      taint_scanning,
      taint_policy,
    }: {
      id: string;
      taint_scanning?: boolean;
      taint_policy?: McpTaintPolicy;
    }) => patchMcpServerTaint(id, { taint_scanning, taint_policy }),
    onSuccess: (_data, variables) => invalidateMcpServer(qc, variables.id),
  });
}
