The terminal UI grew a Backups sub-tab, so an operator on a headless box can take an archive before an upgrade, list what archives exist, restore one and delete an old one without hand-writing HTTP requests.
  Restore is the full form the dashboard has: a keep-my-config toggle for cloning onto a machine that must keep its own key, port and paths, and a per-component selection built from the archive's own `manifest.json` — so the names the TUI sends are always names the daemon wrote, never a guess.
  Deselecting every component is answered in the TUI rather than sent, because `components: []` is the one shape `POST /api/restore` refuses by design.
  The analysis and the approach are @DaBlitzStein's, from #7833.
  (#7897) (@houko)
