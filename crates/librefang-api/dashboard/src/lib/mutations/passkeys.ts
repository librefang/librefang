import { useMutation, useQueryClient } from "@tanstack/react-query";
import { registerPasskey, revokePasskey } from "../../api";
import { passkeyKeys } from "../queries/keys";

export function useRegisterPasskey() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (label?: string) => registerPasskey(label),
    onSuccess: () => qc.invalidateQueries({ queryKey: passkeyKeys.all }),
  });
}

export function useRevokePasskey() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (credentialId: string) => revokePasskey(credentialId),
    onSuccess: () => qc.invalidateQueries({ queryKey: passkeyKeys.all }),
  });
}
