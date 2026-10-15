Sampling overrides now reach every dispatch path — persistent, ephemeral (`/btw`), streaming, and ephemeral worker spawns — instead of only the persistent-session one.
A template that exists but cannot be read is reported as a server-side failure rather than as a missing template, and that verdict no longer depends on the caller's language: the status is decided where the error is raised instead of by matching English substrings of an already-translated message.
A bulk `POST /api/agents/bulk` failure now carries the same machine-readable `code` a single spawn returns.
Out-of-range sampling values are refused rather than written into the manifest TOML, so an operator learns the number was rejected instead of finding a value they never chose.
(#8112) (@DaBlitzStein)
