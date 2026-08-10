Stop and restart Python sidecars when their command reader encounters an unexpected fatal error. (@houko)
An exception from the stdin source, parser, or protocol-error emitter previously killed only the reader task while the main runtime waited forever, leaving a live process that could no longer receive commands or shutdown.
The runtime now logs the traceback, signals cleanup, raises a cause-preserving `ReaderCrashed`, and maps it to a nonzero stdio-process exit so the daemon supervisor can recover the adapter.
