Scrub internal user-management failures before returning HTTP 500 responses, while retaining full configuration, credential-store, hashing, and persistence errors in server logs.
Publish a durably written user-auth snapshot before attempting the wider kernel reload, so key rotation revokes the old credential even when reload fails or the request is cancelled.
User mutation also rejects control characters in names, reports duplicate import rows against live batch state, and documents the actual 204 delete response (#7076) (@xiaomo)
