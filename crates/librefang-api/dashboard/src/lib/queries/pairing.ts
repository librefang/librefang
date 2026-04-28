import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  createPairingRequest,
  completePairing,
  listPairedDevices,
  removePairedDevice,
  type PairingRequestResult,
  type PairedDevice,
  type PairingCompleteResult,
} from "../../api";
import { pairingKeys } from "./keys";

export type { PairingRequestResult, PairedDevice, PairingCompleteResult };

export function usePairingRequest(enabled: boolean) {
  return useQuery({
    queryKey: pairingKeys.request(),
    queryFn: createPairingRequest,
    enabled,
    staleTime: 4 * 60 * 1000, // 4 min — token TTL is 5 min
    gcTime: 5 * 60 * 1000,
    retry: false,
  });
}

export function usePairedDevices() {
  return useQuery({
    queryKey: pairingKeys.devices(),
    queryFn: listPairedDevices,
  });
}

export function useCompletePairing() {
  return useMutation({
    mutationFn: completePairing,
  });
}

export function useRemovePairedDevice() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: removePairedDevice,
    onSuccess: () => qc.invalidateQueries({ queryKey: pairingKeys.devices() }),
  });
}
