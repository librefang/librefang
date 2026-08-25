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

  // Remove inline LaTeX only when it contains a command. A broad `$...$`
  // match also consumes ordinary currency ranges such as "$5 to $10".
  result = result.replace(/\$[^$\n]*\\[a-zA-Z]+[^$\n]*\$/g, "");

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
  const currentAudioUrlRef = useRef<string | null>(null);

  const [speakingMessageId, setSpeakingMessageId] = useState<string | null>(null);
  const [status, setStatus] = useState<TtsStatus>("idle");
  const [error, setError] = useState<string | null>(null);

  const stop = useCallback(() => {
    if (audioRef.current) {
      audioRef.current.pause();
      audioRef.current.currentTime = 0;
      audioRef.current = null;
    }
    if (
      currentAudioUrlRef.current?.startsWith("blob:") &&
      !Array.from(cacheRef.current.values()).includes(currentAudioUrlRef.current)
    ) {
      URL.revokeObjectURL(currentAudioUrlRef.current);
    }
    currentAudioUrlRef.current = null;
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

      // A second click while synthesis is pending must not launch another
      // request for the same message.
      if (currentMessageIdRef.current === messageId && status === "loading") {
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
          if (currentMessageIdRef.current !== messageId) {
            if (result.url.startsWith("blob:")) {
              URL.revokeObjectURL(result.url);
            }
            return;
          }
          objectUrl = result.url;
          cacheRef.current.set(messageId, objectUrl);
        } catch {
          if (currentMessageIdRef.current !== messageId) return;
          setStatus("idle");
          setSpeakingMessageId(null);
          setError("tts_error");
          currentMessageIdRef.current = null;
          return;
        }
      }

      const audio = new Audio(objectUrl);
      audioRef.current = audio;
      currentAudioUrlRef.current = objectUrl;

      const onEnded = () => {
        if (audioRef.current !== audio) return;
        setStatus("idle");
        setSpeakingMessageId(null);
        audioRef.current = null;
        currentMessageIdRef.current = null;
        currentAudioUrlRef.current = null;
      };

      const onError = () => {
        if (audioRef.current !== audio) return;
        setStatus("idle");
        setSpeakingMessageId(null);
        setError("tts_error");
        audioRef.current = null;
        currentMessageIdRef.current = null;
        currentAudioUrlRef.current = null;
      };

      audio.addEventListener("ended", onEnded, { once: true });
      audio.addEventListener("error", onError, { once: true });

      try {
        await audio.play();
        if (audioRef.current !== audio) return;
        setStatus("playing");
      } catch {
        audio.removeEventListener("ended", onEnded);
        audio.removeEventListener("error", onError);
        if (audioRef.current !== audio) return;
        setStatus("idle");
        setSpeakingMessageId(null);
        setError("tts_error");
        audioRef.current = null;
        currentMessageIdRef.current = null;
        currentAudioUrlRef.current = null;
      }
    },
    [status, stop, config?.provider, config?.voice, config?.language, config?.speed],
  );

  useEffect(() => {
    const cache = cacheRef.current;
    return () => {
      for (const url of cache.values()) {
        if (url.startsWith("blob:")) {
          URL.revokeObjectURL(url);
        }
      }
      cache.clear();
      if (audioRef.current) {
        audioRef.current.pause();
        audioRef.current = null;
      }
      currentAudioUrlRef.current = null;
    };
  }, []);

  return { speakingMessageId, status, error, toggle, stop, clearCache };
}
