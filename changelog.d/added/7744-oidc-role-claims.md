OIDC role claims can now authorize API access, through a new `[external_auth.role_map]` that maps an identity provider's group names to LibreFang roles.
The `roles` claim was already parsed off every ID token and injected into request extensions, and no handler ever read it — the daemon fetched the provider's keys, verified the signature, and threw the answer away, so an SSO login could prove who it was and still not call anything.
The mapping is what grants privilege, and it is empty by default: until an operator writes an entry, a validated OIDC bearer authorizes exactly as much as it did before, which is nothing.
A caller holding several mapped groups gets the highest-privilege match, so claim ordering — the provider's business, not yours — cannot decide the effective role, and an unmapped group, a typo'd role string, an unverified email address or a provider with no audience to bind tokens to all grant nothing rather than falling back to a default.
Claims feed the same `viewer` < `user` < `admin` < `owner` ladder that `[[users]]` and `[channel_role_mapping]` already use, so an SSO caller is gated by the identical route/role checks as every other credential.
(#7906) (@houko)
