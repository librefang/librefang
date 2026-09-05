Stop telling an agent on the web interface to deliver media through `channel_send`.
The channel prompt handed the agent a `recipient="<client IP>"` for `webui`, `cron` and `autonomous` — kernel-internal channels with no messaging adapter behind them — so the call could only fail, and agents learned to route the file to Telegram instead of returning it in the response where the user was waiting.
Those channels now say the opposite: media generated during the turn is shown to the user automatically (#7995) (@DaBlitzStein)
