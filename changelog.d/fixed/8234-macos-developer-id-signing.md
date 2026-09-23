Keep macOS Full Disk Access and Automation grants across CLI updates by signing `librefang` and `librefang-sidecar-telegram` with the project's Developer ID certificate instead of ad hoc.
An ad-hoc signature derives the code identity from the binary's own hash, so every release looked like a new, never-approved app to macOS privacy controls, and a daemon reading a configured vault lost access after each `librefang update`.
The installer no longer overwrites a valid Developer ID signature with an ad-hoc one, and still ad-hoc signs unsigned or ad-hoc builds so Apple Silicon will run them.
The binaries are not notarized and do not use the hardened runtime; the first update from an ad-hoc build still needs the grants re-added once (#8234) (@houko)
