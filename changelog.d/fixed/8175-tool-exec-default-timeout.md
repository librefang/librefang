The local tool-execution backend now takes its default per-command timeout from configuration instead of a hardcoded 30 seconds.
`LocalBackend` documented `kernel config / agent manifest` as its source of truth while `ToolExecConfig` had no timeout field at all, so `build_backend` passed the constant on every path and an operator who raised `tool_timeout_secs` saw it apply to tool dispatch but not to the commands those tools ran — no error, nothing in the log.
The new `tool_exec.default_timeout_secs` fills that gap, and leaving it unset inherits `tool_timeout_secs` so the two timeout paths agree rather than merely both being configurable.
(#8175) (@DaBlitzStein)
