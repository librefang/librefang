`librefang.exe` no longer dies at startup with a stack overflow on Windows.
  Windows reserves 1 MiB for a main thread where Linux and macOS give 8 MiB, which the command dispatch exceeded before running anything; the binary now asks for 16 MiB in its image header.
  (#8340) (@houko)
