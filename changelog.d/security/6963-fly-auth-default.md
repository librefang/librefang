Require authentication by default in the reusable Fly.io deploy template.
  `deploy/fly/fly.toml` previously shipped `LIBREFANG_ALLOW_NO_AUTH=1` unconditionally, so every deployment derived from the template inherited the official demo's intentionally open auth posture, not just the demo itself.
  The one-command deploy script now generates a 256-bit `LIBREFANG_API_KEY` and imports it as a Fly secret before the app's first boot, and the official public demo's unauthenticated exception moved into its own release CI job rather than the shared template (#6963) (@houko)
