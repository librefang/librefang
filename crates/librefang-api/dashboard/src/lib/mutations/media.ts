import {
  useMutation,
  useQueryClient,
  type UseMutationOptions,
} from "@tanstack/react-query";
import {
  generateImage,
  synthesizeSpeech,
  submitVideo,
  generateMusic,
  type MediaImageResult,
  type SpeechResult,
  type MediaVideoSubmitResult,
  type MediaMusicResult,
} from "../http/client";
import { budgetKeys, userBudgetKeys, usageKeys } from "../queries/keys";

export function useGenerateImage(
  options?: Partial<
    UseMutationOptions<
      MediaImageResult,
      Error,
      { prompt: string; provider?: string; model?: string; count?: number; aspect_ratio?: string }
    >
  >,
) {
  const queryClient = useQueryClient();
  return useMutation({
    ...options,
    mutationFn: generateImage,
    onSettled: (...args) => {
      // Media generation hits both global and per-user budget plus usage tallies.
      queryClient.invalidateQueries({ queryKey: budgetKeys.all });
      queryClient.invalidateQueries({ queryKey: userBudgetKeys.all });
      queryClient.invalidateQueries({ queryKey: usageKeys.all });
      options?.onSettled?.(...args);
    },
  });
}

export function useSynthesizeSpeech(
  options?: Partial<
    UseMutationOptions<
      SpeechResult,
      Error,
      { text: string; provider?: string; model?: string; voice?: string; format?: string; language?: string; speed?: number }
    >
  >,
) {
  const queryClient = useQueryClient();
  return useMutation({
    ...options,
    mutationFn: synthesizeSpeech,
    onSettled: (...args) => {
      // Media generation hits both global and per-user budget plus usage tallies.
      queryClient.invalidateQueries({ queryKey: budgetKeys.all });
      queryClient.invalidateQueries({ queryKey: userBudgetKeys.all });
      queryClient.invalidateQueries({ queryKey: usageKeys.all });
      options?.onSettled?.(...args);
    },
  });
}

export function useSubmitVideo(
  options?: Partial<
    UseMutationOptions<
      MediaVideoSubmitResult,
      Error,
      { prompt: string; provider?: string; model?: string }
    >
  >,
) {
  const queryClient = useQueryClient();
  return useMutation({
    ...options,
    mutationFn: submitVideo,
    onSettled: (...args) => {
      // Media generation hits both global and per-user budget plus usage tallies.
      queryClient.invalidateQueries({ queryKey: budgetKeys.all });
      queryClient.invalidateQueries({ queryKey: userBudgetKeys.all });
      queryClient.invalidateQueries({ queryKey: usageKeys.all });
      options?.onSettled?.(...args);
    },
  });
}

export function useGenerateMusic(
  options?: Partial<
    UseMutationOptions<
      MediaMusicResult,
      Error,
      { prompt?: string; lyrics?: string; provider?: string; model?: string; instrumental?: boolean }
    >
  >,
) {
  const queryClient = useQueryClient();
  return useMutation({
    ...options,
    mutationFn: generateMusic,
    onSettled: (...args) => {
      // Media generation hits both global and per-user budget plus usage tallies.
      queryClient.invalidateQueries({ queryKey: budgetKeys.all });
      queryClient.invalidateQueries({ queryKey: userBudgetKeys.all });
      queryClient.invalidateQueries({ queryKey: usageKeys.all });
      options?.onSettled?.(...args);
    },
  });
}
