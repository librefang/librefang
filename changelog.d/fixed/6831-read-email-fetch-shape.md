Validate IMAP FETCH responses before parsing email bytes. (@houko)
The email helper previously indexed the server response without checking its shape, producing opaque index/type errors or passing flag-only data into the MIME parser.
Empty, truncated, non-tuple, and non-byte responses now fail with a clear malformed-response diagnostic.
