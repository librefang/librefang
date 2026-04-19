import { useMutation } from "@tanstack/react-query";
import {
  generateImage,
  synthesizeSpeech,
  submitVideo,
  generateMusic,
} from "../http/client";

export function useGenerateImage() {
  return useMutation({ mutationFn: generateImage });
}

export function useSynthesizeSpeech() {
  return useMutation({ mutationFn: synthesizeSpeech });
}

export function useSubmitVideo() {
  return useMutation({ mutationFn: submitVideo });
}

export function useGenerateMusic() {
  return useMutation({ mutationFn: generateMusic });
}
