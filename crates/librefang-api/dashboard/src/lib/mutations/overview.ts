import { useMutation, useQueryClient } from "@tanstack/react-query";
import { postQuickInit } from "../../api";
import { overviewKeys } from "../queries/keys";

export function useQuickInit() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: postQuickInit,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: overviewKeys.snapshot() });
    },
  });
}
