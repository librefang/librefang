import { useMutation, useQueryClient } from "@tanstack/react-query";
import { discoverA2AAgent, sendA2ATask } from "../http/client";
import { a2aKeys } from "../queries/keys";

export function useDiscoverA2AAgent() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: discoverA2AAgent,
    onSuccess: () => qc.invalidateQueries({ queryKey: a2aKeys.agents() }),
  });
}

// fire-and-forget: task status tracked locally in A2APage, no query cache affected
export function useSendA2ATask() {
  return useMutation({
    mutationFn: sendA2ATask,
  });
}
