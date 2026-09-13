Config `include` entries and requested agent workspaces that Windows reports as relative — `/etc/passwd`, `C:passwd` — are now refused rather than joined onto the config or workspaces root, where they resolved outside it.
  The rule is stated once in the kernel and shared by both include walks and the dashboard's sidecar scan, which previously failed its whole scan on the read error instead of skipping the entry.
  (#8340) (@houko)
