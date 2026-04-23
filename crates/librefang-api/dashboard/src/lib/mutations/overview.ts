import { useMutation, useQueryClient } from "@tanstack/react-query";
import { postQuickInit } from "../../api";
import { overviewKeys } from "../queries/keys";

export function useQuickInit() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: postQuickInit,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: overviewKeys.snapshot() });
    },
  });
}
