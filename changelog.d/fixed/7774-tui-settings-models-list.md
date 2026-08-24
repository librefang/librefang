The TUI's Settings > Models list rendered empty against a healthy daemon, and its price column was pinned to `$0.00/$0.00`.
`GET /api/models` answers with a `{ "models": [...] }` object and the screen read the body as a bare array, so every fetch produced nothing; the cost fields it read were named after keys the response has never carried (#7881) (@houko)
