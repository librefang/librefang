import { useState, useRef, useCallback, useEffect } from "react";
import { transcribeAudio } from "../api";

export function useVoiceInput(onTranscript: (text: string) => void) {
  const [isRecording, setIsRecording] = useState(false);
  const [isTranscribing, setIsTranscribing] = useState(false);

  const mediaRecorderRef = useRef<MediaRecorder | null>(null);
  const chunksRef = useRef<Blob[]>([]);
  const streamRef = useRef<MediaStream | null>(null);
  const recordingRef = useRef(false);
  const onTranscriptRef = useRef(onTranscript);
  const mountedRef = useRef(true);

  useEffect(() => {
    onTranscriptRef.current = onTranscript;
  }, [onTranscript]);

  const hasMediaDevices = typeof navigator !== "undefined" && !!navigator.mediaDevices?.getUserMedia;

  const cleanup = useCallback(() => {
    if (streamRef.current) {
      streamRef.current.getTracks().forEach((t) => t.stop());
      streamRef.current = null;
    }
    mediaRecorderRef.current = null;
    chunksRef.current = [];
    recordingRef.current = false;
    if (mountedRef.current) setIsRecording(false);
  }, []);

  const startRecording = useCallback(async () => {
    if (recordingRef.current) return;
    recordingRef.current = true;

    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      if (!mountedRef.current) {
        stream.getTracks().forEach((track) => track.stop());
        return;
      }
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

        if (!mountedRef.current) return;
        setIsTranscribing(true);
        try {
          const result = await transcribeAudio(blob);
          if (!mountedRef.current) return;
          if (result.text.trim()) {
            onTranscriptRef.current(result.text.trim());
          }
        } catch (err) {
          console.error("Transcription failed:", err);
        } finally {
          if (mountedRef.current) {
            setIsTranscribing(false);
            cleanup();
          }
        }
      };

      mediaRecorderRef.current = recorder;
      setIsRecording(true);
      recorder.start();
    } catch (err) {
      console.error("Microphone access denied:", err);
      cleanup();
    }
  }, [cleanup]);

  const stopRecording = useCallback(() => {
    if (mediaRecorderRef.current) {
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

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      cleanup();
    };
  }, [cleanup]);

  return {
    isRecording,
    isTranscribing,
    isSupported: hasMediaDevices,
    startRecording,
    stopRecording,
    toggleRecording,
  };
}
