import { useMutation } from "@tanstack/react-query";
import {
  generateImage,
  synthesizeSpeech,
  submitVideo,
  pollVideo,
  generateMusic,
} from "../../api";

export function useGenerateImage() {
  return useMutation({ mutationFn: generateImage });
}

export function useSynthesizeSpeech() {
  return useMutation({ mutationFn: synthesizeSpeech });
}

export function useSubmitVideo() {
  return useMutation({ mutationFn: submitVideo });
}

export function usePollVideo() {
  return useMutation({ mutationFn: ({ taskId, provider }: { taskId: string; provider: string }) => pollVideo(taskId, provider) });
}

export function useGenerateMusic() {
  return useMutation({ mutationFn: generateMusic });
}
