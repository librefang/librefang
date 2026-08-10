Agent context reads now bind path validation and file access to the same opened handles, preventing a workspace path swap from redirecting `context.md` outside the workspace.
  Symlinked identity entries no longer shadow a regular legacy context, and replacing a previously trusted context with a symlink falls back to its cached good content (#6772) (@houko)
