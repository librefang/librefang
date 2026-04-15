import { useState, useRef, useCallback } from "react";
import { transcribeAudio } from "../api";

type SpeechRecognitionCompat = {
  continuous: boolean;
  interimResults: boolean;
  lang: string;
  start: () => void;
  stop: () => void;
  abort: () => void;
  onresult: ((event: SpeechRecognitionEventCompat) => void) | null;
  onerror: ((event: { error: string }) => void) | null;
  onend: (() => void) | null;
};

type SpeechRecognitionEventCompat = {
  resultIndex: number;
  results: {
    length: number;
    [index: number]: {
      isFinal: boolean;
      [index: number]: { transcript: string };
    };
  };
};

function getSpeechRecognition(): (new () => SpeechRecognitionCompat) | null {
  const w = window as unknown as Record<string, unknown>;
  return (w.SpeechRecognition ?? w.webkitSpeechRecognition ?? null) as
    | (new () => SpeechRecognitionCompat)
    | null;
}

export function useVoiceInput(onTranscript: (text: string) => void) {
  const [isRecording, setIsRecording] = useState(false);
  const [isTranscribing, setIsTranscribing] = useState(false);

  const recognitionRef = useRef<SpeechRecognitionCompat | null>(null);
  const mediaRecorderRef = useRef<MediaRecorder | null>(null);
  const chunksRef = useRef<Blob[]>([]);
  const streamRef = useRef<MediaStream | null>(null);
  const recordingRef = useRef(false);

  // Which mode: "speech-api" | "media-recorder" | null
  const modeRef = useRef<"speech-api" | "media-recorder" | null>(null);

  const hasWebSpeechApi = typeof window !== "undefined" && getSpeechRecognition() !== null;
  const hasMediaDevices = typeof navigator !== "undefined" && !!navigator.mediaDevices?.getUserMedia;
  const isSupported = hasWebSpeechApi || hasMediaDevices;

  const cleanup = useCallback(() => {
    if (streamRef.current) {
      streamRef.current.getTracks().forEach((t) => t.stop());
      streamRef.current = null;
    }
    recognitionRef.current = null;
    mediaRecorderRef.current = null;
    chunksRef.current = [];
    modeRef.current = null;
    recordingRef.current = false;
    setIsRecording(false);
  }, []);

  const startRecording = useCallback(async () => {
    if (recordingRef.current) return;
    recordingRef.current = true;

    const SR = getSpeechRecognition();
    if (SR) {
      // Web Speech API path
      modeRef.current = "speech-api";
      const recognition = new SR();
      recognition.continuous = false;
      recognition.interimResults = false;
      recognition.lang = navigator.language || "en-US";

      recognition.onresult = (event: SpeechRecognitionEventCompat) => {
        let transcript = "";
        for (let i = event.resultIndex; i < event.results.length; i++) {
          if (event.results[i].isFinal) {
            transcript += event.results[i][0].transcript;
          }
        }
        if (transcript.trim()) {
          onTranscript(transcript.trim());
        }
      };

      recognition.onerror = (event: { error: string }) => {
        // "no-speech" and "aborted" are expected when user stops early
        if (event.error !== "no-speech" && event.error !== "aborted") {
          console.warn("SpeechRecognition error:", event.error);
        }
        cleanup();
      };

      recognition.onend = () => {
        cleanup();
      };

      recognitionRef.current = recognition;
      setIsRecording(true);
      recognition.start();
      return;
    }

    if (hasMediaDevices) {
      // MediaRecorder fallback — record and send to backend STT
      modeRef.current = "media-recorder";
      try {
        const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
        streamRef.current = stream;

        const mimeType = MediaRecorder.isTypeSupported("audio/webm;codecs=opus")
          ? "audio/webm;codecs=opus"
          : "audio/webm";
        const recorder = new MediaRecorder(stream, { mimeType });
        chunksRef.current = [];

        recorder.ondataavailable = (e) => {
          if (e.data.size > 0) chunksRef.current.push(e.data);
        };

        recorder.onstop = async () => {
          const blob = new Blob(chunksRef.current, { type: mimeType });
          streamRef.current?.getTracks().forEach((t) => t.stop());
          streamRef.current = null;

          if (blob.size === 0) {
            cleanup();
            return;
          }

          setIsTranscribing(true);
          try {
            const result = await transcribeAudio(blob);
            if (result.text.trim()) {
              onTranscript(result.text.trim());
            }
          } catch (err) {
            console.error("Transcription failed:", err);
          } finally {
            setIsTranscribing(false);
            cleanup();
          }
        };

        mediaRecorderRef.current = recorder;
        setIsRecording(true);
        recorder.start();
      } catch (err) {
        console.error("Microphone access denied:", err);
        cleanup();
      }
    }
  }, [onTranscript, cleanup, hasMediaDevices]);

  const stopRecording = useCallback(() => {
    if (modeRef.current === "speech-api" && recognitionRef.current) {
      recognitionRef.current.stop();
    } else if (modeRef.current === "media-recorder" && mediaRecorderRef.current) {
      mediaRecorderRef.current.stop();
      setIsRecording(false);
    }
  }, []);

  const toggleRecording = useCallback(() => {
    if (recordingRef.current) {
      stopRecording();
    } else {
      startRecording();
    }
  }, [startRecording, stopRecording]);

  return {
    isRecording,
    isTranscribing,
    isSupported,
    startRecording,
    stopRecording,
    toggleRecording,
  };
}
