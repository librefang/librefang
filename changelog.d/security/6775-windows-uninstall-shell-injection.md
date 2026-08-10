The Windows desktop uninstaller now parses the registered NSIS command line with native Windows argument semantics and launches the executable directly.
A tampered per-user `UninstallString` can no longer append commands through shell metacharacters because the desktop app no longer passes it to `cmd /C` (#6775) (@houko)
