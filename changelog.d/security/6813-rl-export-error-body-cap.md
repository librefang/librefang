Stop RL exporters from buffering an upstream's complete error response before truncating the diagnostic to 4 KiB. (@houko)
A malicious or broken W&B, Tinker, or Atropos endpoint could previously declare and stream an arbitrarily large 4xx/5xx body, forcing reqwest to accumulate it all in memory and potentially terminate the process before LibreFang applied its display cap.
Error bodies are now consumed incrementally into a buffer capped at 4096 bytes, and the reader returns as soon as that cap is reached instead of waiting for the remaining response.
