An exhausted budget or resource quota now answers `429` with the refusal intact instead of a scrubbed `500`.
It reached the client as "Internal server error" — indistinguishable from a crash, and an invitation to retry the request that had just refused it on purpose.
  (#7903) (@houko)
