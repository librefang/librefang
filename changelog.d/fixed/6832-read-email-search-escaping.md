Escape sender values in IMAP email searches. (@houko)
The helper previously interpolated the sender directly into a quoted SEARCH criterion, allowing quotes, backslashes, or line controls to alter or break the command structure.
Quotes and backslashes are now escaped as IMAP quoted-string data, while CR, LF, and NUL are rejected before opening a connection.
