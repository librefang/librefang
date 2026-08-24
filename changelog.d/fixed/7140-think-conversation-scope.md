Channel `/think` now takes effect, and takes effect only where it was typed.
It previously stored a preference nothing ever read, so the next turn ran exactly as before while the reply said the setting had been applied; the toggle now rides the turn as a per-call thinking override on the streaming, non-streaming and image send paths.
The preference is keyed by the conversation — channel, bot account, chat, agent — rather than by the agent alone, so one Telegram group turning extended thinking on no longer changes the reasoning mode, and the token bill, of every other chat the same agent serves.
An unrecognised argument such as `/think of` is rejected instead of being read as "off" (#7854) (@houko)
