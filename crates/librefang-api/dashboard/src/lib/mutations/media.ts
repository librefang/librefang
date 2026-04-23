import {
  useMutation,
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

export function useGenerateImage(
  options?: Partial<
    UseMutationOptions<
      MediaImageResult,
      Error,
      { prompt: string; provider?: string; model?: string; count?: number; aspect_ratio?: string }
    >
  >,
) {
  return useMutation({
    ...options,
    mutationFn: generateImage,
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
  return useMutation({
    ...options,
    mutationFn: synthesizeSpeech,
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
  return useMutation({
    ...options,
    mutationFn: submitVideo,
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
  return useMutation({
    ...options,
    mutationFn: generateMusic,
  });
}
