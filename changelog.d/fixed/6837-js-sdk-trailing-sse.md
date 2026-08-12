Fixed the generated JavaScript SDK dropping a final server-sent event when a stream ended without a trailing newline, the same defect class fixed for the Rust and Python SDKs in this release.
`_stream` split incoming bytes on `\n` and only processed complete lines, so a clean EOF right after the last `data: ` line left it sitting unprocessed in the leftover buffer (#6837) (@houko)
