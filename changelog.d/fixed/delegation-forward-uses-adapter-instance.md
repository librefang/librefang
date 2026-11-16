Async delegation results now reach the operator on a daemon that runs more than one
  adapter of the same channel type.
  The home-channel context carried `account_id: None`, so the forward issued
  `send_channel_message("telegram", …)` and was rejected as ambiguous
  ("N registered adapters share that channel type"); the callers log that error and
  move on, so the result stayed in the delegating agent's session and never hit the
  chat — indistinguishable from a lost task.
  The context now carries the sidecar's instance name as `account_id`, which is what
  `resolve_channel_adapter` matches against `adapter.name()`, pinning the send to the
  agent's own bot.
  (#TBD) (@smanic)
