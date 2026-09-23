An agent message the kernel skipped because no LLM provider is configured is now recorded with an outcome starting `failed:`, so the Logs page and `/api/logs/stream?level=error` show it as an error instead of `info`.
  Audit entries written before this change keep their old outcome and stay `info` (#8496) (@houko)
