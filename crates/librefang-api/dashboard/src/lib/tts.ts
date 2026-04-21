import { useCallback, useEffect, useRef, useState } from "react";
import { synthesizeSpeech } from "../api";

export type TtsStatus = "idle" | "loading" | "playing" | "paused";

export interface UseTtsManagerReturn {
  speakingMessageId: string | null;
  status: TtsStatus;
  error: string | null;
  toggle: (messageId: string, content: string) => Promise<void>;
  stop: () => void;
  clearCache: () => void;
}

/**
 * Strip markdown formatting to produce plain text suitable for TTS.
 */
export function stripMarkdown(text: string): string {
  let result = text;

  // Remove fenced code blocks
  result = result.replace(/```[\s\S]*?```/g, "");

  // Remove LaTeX display blocks ($$...$$) before inline
  result = result.replace(/\$\$[\s\S]*?\$\$/g, "");

  // Remove LaTeX inline ($...$)
  result = result.replace(/\$[^$\n]+?\$/g, "");

  // Remove inline code
  result = result.replace(/`[^`]*`/g, "");

  // Remove images ![alt](url)
  result = result.replace(/!\[([^\]]*)\]\([^)]*\)/g, "");

  // Remove links [text](url) -> text
  result = result.replace(/\[([^\]]*)\]\([^)]*\)/g, "$1");

  // Remove heading markers
  result = result.replace(/^#{1,6}\s+/gm, "");

  // Remove horizontal rules
  result = result.replace(/^[-*]{3,}\s*$/gm, "");

  // Remove emphasis markers (bold+italic combos first, then individual)
  result = result.replace(/\*{3}(.+?)\*{3}/g, "$1");
  result = result.replace(/_{3}(.+?)_{3}/g, "$1");
  result = result.replace(/\*{2}(.+?)\*{2}/g, "$1");
  result = result.replace(/_{2}(.+?)_{2}/g, "$1");
  result = result.replace(/\*(.+?)\*/g, "$1");
  result = result.replace(/_(.+?)_/g, "$1");

  // Remove list markers (-, *, numbered)
  result = result.replace(/^\s*[-*]\s+/gm, "");
  result = result.replace(/^\s*\d+\.\s+/gm, "");

  // Collapse multiple newlines
  result = result.replace(/\n{2,}/g, "\n");

  return result.trim();
}

export interface TtsSpeechConfig {
  provider?: string;
  voice?: string;
  language?: string;
  speed?: number;
}

export function useTtsManager(config?: TtsSpeechConfig): UseTtsManagerReturn {
  const cacheRef = useRef<Map<string, string>>(new Map());
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const currentMessageIdRef = useRef<string | null>(null);

  const [speakingMessageId, setSpeakingMessageId] = useState<string | null>(null);
  const [status, setStatus] = useState<TtsStatus>("idle");
  const [error, setError] = useState<string | null>(null);

  const stop = useCallback(() => {
    if (audioRef.current) {
      const src = audioRef.current.src;
      audioRef.current.pause();
      audioRef.current.currentTime = 0;
      if (src && src.startsWith("blob:") && !cacheRef.current.has(currentMessageIdRef.current ?? "")) {
        URL.revokeObjectURL(src);
      }
      audioRef.current = null;
    }
    currentMessageIdRef.current = null;
    setStatus("idle");
    setSpeakingMessageId(null);
  }, []);

  const clearCache = useCallback(() => {
    stop();
    for (const url of cacheRef.current.values()) {
      if (url.startsWith("blob:")) {
        URL.revokeObjectURL(url);
      }
    }
    cacheRef.current.clear();
  }, [stop]);

  const toggle = useCallback(
    async (messageId: string, content: string) => {
      // Same message, currently playing -> pause
      if (currentMessageIdRef.current === messageId && status === "playing") {
        audioRef.current?.pause();
        setStatus("paused");
        return;
      }

      // Same message, currently paused -> resume
      if (currentMessageIdRef.current === messageId && status === "paused") {
        audioRef.current?.play();
        setStatus("playing");
        return;
      }

      // Different message or idle -> stop current, start new
      stop();

      setStatus("loading");
      setSpeakingMessageId(messageId);
      setError(null);
      currentMessageIdRef.current = messageId;

      let objectUrl = cacheRef.current.get(messageId);

      if (!objectUrl) {
        const stripped = stripMarkdown(content);
        if (!stripped) {
          setStatus("idle");
          setSpeakingMessageId(null);
          setError("tts_empty");
          currentMessageIdRef.current = null;
          return;
        }

        try {
          const result = await synthesizeSpeech({ text: stripped, provider: config?.provider, voice: config?.voice, language: config?.language, speed: config?.speed });
          objectUrl = result.url;
          cacheRef.current.set(messageId, objectUrl);
        } catch {
          setStatus("idle");
          setSpeakingMessageId(null);
          setError("tts_error");
          currentMessageIdRef.current = null;
          return;
        }
      }

      const audio = new Audio(objectUrl);
      audioRef.current = audio;

      audio.addEventListener("ended", () => {
        setStatus("idle");
        setSpeakingMessageId(null);
        currentMessageIdRef.current = null;
      }, { once: true });

      audio.addEventListener("error", () => {
        setStatus("idle");
        setSpeakingMessageId(null);
        setError("tts_error");
        currentMessageIdRef.current = null;
      }, { once: true });

      try {
        await audio.play();
        setStatus("playing");
      } catch {
        setStatus("idle");
        setSpeakingMessageId(null);
        setError("tts_error");
        currentMessageIdRef.current = null;
      }
    },
    [status, stop, config?.provider, config?.voice, config?.language, config?.speed],
  );

  useEffect(() => {
    return () => {
      for (const url of cacheRef.current.values()) {
        if (url.startsWith("blob:")) {
          URL.revokeObjectURL(url);
        }
      }
      cacheRef.current.clear();
      if (audioRef.current) {
        audioRef.current.pause();
        audioRef.current = null;
      }
    };
  }, []);

  return { speakingMessageId, status, error, toggle, stop, clearCache };
}
