The Chinese `librefang doctor` output is readable again: 71 `zh-CN` values were generated from their own key names rather than translated, and have been rewritten.
`CLI is up to date` rendered as `cliuptodate`, `Database status: { $status }` as `db状态fail失败：{ $status }`, and the `Channel Integrations:` section heading as `doctorsection频道`.
Two of them changed behaviour rather than only readability — the `[Y/n]` was missing from both `doctor` confirmation prompts, so a Chinese user was asked a yes/no question with no indication of what to type or which answer was the default, and the `.env file not found` warning dropped the `librefang config set-key` command that resolves it.
Korean and Ukrainian were unaffected; the values trace to a single bulk import in #6253. (@houko)
