import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  createTerminalWindow,
  renameTerminalWindow,
  deleteTerminalWindow,
  type TerminalWindow,
} from "../http/client";
import { terminalKeys } from "../queries/keys";

export function useCreateTerminalWindow() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (body: { name?: string } = {}) => createTerminalWindow(body),
    onSuccess: () => qc.invalidateQueries({ queryKey: terminalKeys.windows() }),
  });
}

interface WindowMutationContext {
  previous: TerminalWindow[] | undefined;
}

export function useRenameTerminalWindow() {
  const qc = useQueryClient();
  return useMutation<
    void,
    Error,
    { windowId: string; name: string },
    WindowMutationContext
  >({
    mutationFn: ({ windowId, name }) => renameTerminalWindow(windowId, name),
    // Optimistic update: swap the tab label immediately so the UI feels
    // instant. If the PATCH fails we roll back to the captured snapshot.
    onMutate: async ({ windowId, name }) => {
      await qc.cancelQueries({ queryKey: terminalKeys.windows() });
      const previous = qc.getQueryData<TerminalWindow[]>(terminalKeys.windows());
      qc.setQueryData<TerminalWindow[]>(terminalKeys.windows(), (prev) =>
        prev?.map((w) => (w.id === windowId ? { ...w, name } : w))
      );
      return { previous };
    },
    onError: (_err, _vars, context) => {
      if (context?.previous !== undefined) {
        qc.setQueryData(terminalKeys.windows(), context.previous);
      }
    },
    onSettled: () => qc.invalidateQueries({ queryKey: terminalKeys.windows() }),
  });
}

export function useDeleteTerminalWindow() {
  const qc = useQueryClient();
  return useMutation<
    Awaited<ReturnType<typeof deleteTerminalWindow>>,
    Error,
    string,
    WindowMutationContext
  >({
    mutationFn: (windowId: string) => deleteTerminalWindow(windowId),
    onMutate: async (windowId) => {
      await qc.cancelQueries({ queryKey: terminalKeys.windows() });
      const previous = qc.getQueryData<TerminalWindow[]>(terminalKeys.windows());
      qc.setQueryData<TerminalWindow[]>(terminalKeys.windows(), (prev) =>
        prev?.filter((window) => window.id !== windowId)
      );
      return { previous };
    },
    onError: (_err, _windowId, context) => {
      if (context?.previous !== undefined) {
        qc.setQueryData(terminalKeys.windows(), context.previous);
      }
    },
    onSettled: () => qc.invalidateQueries({ queryKey: terminalKeys.windows() }),
  });
}
