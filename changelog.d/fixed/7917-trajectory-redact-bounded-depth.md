Trajectory export no longer walks a JSON value recursively when redacting it, so a deeply nested tool input or output cannot abort the daemon.
The recursive walk overflowed the stack and killed the process outright rather than returning an error, and the recursive `Drop` glue for such a value would have overflowed even if only the walk had been bounded.
`librefang-rl-export` hardened its equivalent function against exactly this and this one was left behind, so the repository had two functions doing the same job with only one of them safe.
(#7917) (@houko)
