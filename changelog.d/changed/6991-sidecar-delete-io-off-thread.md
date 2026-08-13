`DELETE /api/channels/sidecar/{name}` rewrote `config.toml` synchronously on the async worker thread handling the request, still holding the existing `config_write_lock` across the call.
  The sidecar-block removal and durable atomic rewrite now run on Tokio's blocking pool instead, with the config write lock held across the whole operation exactly as before.
  A join failure on that blocking task (for example a panic inside the removal closure) is now caught and returned as a scrubbed internal error instead of propagating as an unhandled panic in the request future (#6991) (@houko)
