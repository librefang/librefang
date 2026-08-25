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

type MediaMutationOptions<TResult, TVariables> = Omit<
  UseMutationOptions<TResult, Error, TVariables>,
  "mutationFn"
>;

function useMediaMutation<TResult, TVariables>(
  mutationFn: (variables: TVariables) => Promise<TResult>,
  options?: MediaMutationOptions<TResult, TVariables>,
) {
  const queryClient = useQueryClient();
  return useMutation<TResult, Error, TVariables>({
    ...options,
    mutationFn,
    onSettled: (data, error, variables, onMutateResult, context) => {
      // Media calls affect every aggregate below: provider/global spend,
      // an unknown current user's budget, and all usage projections.
      queryClient.invalidateQueries({ queryKey: budgetKeys.all });
      queryClient.invalidateQueries({ queryKey: userBudgetKeys.all });
      queryClient.invalidateQueries({ queryKey: usageKeys.all });
      return options?.onSettled?.(data, error, variables, onMutateResult, context);
    },
  });
}

export function useGenerateImage(
  options?: MediaMutationOptions<
    MediaImageResult,
    { prompt: string; provider?: string; model?: string; count?: number; aspect_ratio?: string }
  >,
) {
  return useMediaMutation(generateImage, options);
}

export function useSynthesizeSpeech(
  options?: MediaMutationOptions<
    SpeechResult,
    { text: string; provider?: string; model?: string; voice?: string; format?: string; language?: string; speed?: number }
  >,
) {
  return useMediaMutation(synthesizeSpeech, options);
}

export function useSubmitVideo(
  options?: MediaMutationOptions<
    MediaVideoSubmitResult,
    { prompt: string; provider?: string; model?: string }
  >,
) {
  return useMediaMutation(submitVideo, options);
}

export function useGenerateMusic(
  options?: MediaMutationOptions<
    MediaMusicResult,
    { prompt?: string; lyrics?: string; provider?: string; model?: string; instrumental?: boolean }
  >,
) {
  return useMediaMutation(generateMusic, options);
}
