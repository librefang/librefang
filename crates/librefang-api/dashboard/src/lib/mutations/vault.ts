import { useMutation, useQueryClient } from "@tanstack/react-query";
import { setVaultKey, deleteVaultKey } from "../http/client";
import { vaultKeys } from "../queries/keys";

// A write flips the `set` boolean the listing reports, and the listing is the
// only cached vault query there is — `lists()` is both the narrowest and the
// complete set of affected keys.

export function useSetVaultKey() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ key, value }: { key: string; value: string }) =>
      setVaultKey(key, value),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: vaultKeys.lists() });
    },
  });
}

export function useDeleteVaultKey() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ key }: { key: string }) => deleteVaultKey(key),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: vaultKeys.lists() });
    },
  });
}
