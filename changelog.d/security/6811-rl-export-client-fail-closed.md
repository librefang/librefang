Fail closed when the RL trajectory exporters cannot construct their redirect-disabled HTTP client. (@houko)
W&B, Tinker, and Atropos previously fell back to the shared default client after a builder error; because that fallback follows redirects, a rare local client-configuration failure silently removed the SSRF guard and could replay export credentials to a redirected destination.
Client construction is now shared by all three exporters, preserves the configured proxy and TLS settings, disables redirects, and returns the construction error instead of weakening the transport policy.
