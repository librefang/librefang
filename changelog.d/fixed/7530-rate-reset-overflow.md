Reject rate-limit reset durations that overflow either `Duration` or the system clock so malformed headers fall through to a usable cooldown instead of panicking. (#7530) (@houko)
