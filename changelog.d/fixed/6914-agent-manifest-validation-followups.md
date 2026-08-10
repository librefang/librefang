The dashboard agent editor now blocks periodic schedules without a cron expression and JSON-schema response formats whose schemas are empty, malformed, or cannot be represented faithfully in TOML.
Validation errors automatically open their sections and are exposed to assistive technology.
It also removes a redundant schedule parsing branch.
(#6914) (@houko)
