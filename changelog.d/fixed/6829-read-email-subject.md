Decode complete RFC 2047 email subjects. (@houko)
The IMAP email helper previously decoded only the first subject segment, truncating mixed plain/encoded subjects and subjects composed of multiple encoded words.
It now joins every decoded segment in order and retains a UTF-8 fallback for unknown charset labels.
