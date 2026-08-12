The dashboard agent editor now blocks periodic schedules without a cron expression and JSON-schema response formats whose schemas are empty, malformed, or cannot be represented faithfully in TOML.
Validation errors automatically open their sections and are exposed to assistive technology.
It also removes a redundant schedule parsing branch, clears duplicate tag submissions, and gives the stream-thinking toggle an accessible name.
(#6914) (@houko)
