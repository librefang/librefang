Async delegation results now reach the operator on a daemon that runs more than one adapter of the same channel type.
  The forward that delivers them issued `send_channel_message("<type>", …)` with no account, so on such a daemon it was rejected as ambiguous ("N registered adapters share that channel type").
  Both call sites log that error and move on, leaving the result inside the delegating agent's session — indistinguishable from a lost task.
  The agent's home sidecar instance is now passed as the send's `account_id`, which is what `resolve_channel_adapter` matches against `adapter.name()`.
  The turn's own `SenderContext.account_id` is deliberately left empty, so the cross-account guard keeps no-opping for turns that arrived from nowhere.
  (#8772) (@nevgenov)
