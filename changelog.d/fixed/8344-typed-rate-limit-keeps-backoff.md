A rate limit the provider reported with a typed error code keeps its backoff retry again, whatever HTTP status carried it.
  #8322 decided retryability by re-reading the status off the error, which overrode the classifier: a gateway that reports a rate limit as 403, or as 400 with `error.code = "rate_limit_exceeded"`, was failed over immediately instead of waiting and retrying — the one case the retry loop exists for.
  The decision is back on the reason the classifier produced, so only the ambiguous-status catch-all consults the status. (#8344) (@houko)
