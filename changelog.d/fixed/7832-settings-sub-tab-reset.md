Re-entering the TUI Settings tab now returns it to the Providers sub-tab instead of reopening on whichever sub-tab was last used, and clears any modal left over from it.
  `SettingsState::sub` was a plain field that outlived the tab while `on_tab_enter` reloaded providers regardless, so the screen could show one sub-tab's contents over another's freshly loaded data — and a sub-tab holding a modal that binds the `1`-`4` switch keys had no second way out for the rest of the session.
  (#7897) (@houko)
