Let an Owner set their own emoji and avatar image from the dashboard, in the edit drawer of their own row on the users page, and draw it on the user's side of every chat message.
The daemon could already store both, but nothing in the web UI set or showed either, so every user message carried the same generic icon.
A chat bubble now draws the signed-in user's picture, then their emoji, then their initials, and keeps the generic icon for the synthetic `root` of a no-auth or master-key session, which names no user.
The editor appears only on the caller's own row because the dashboard can read only the caller's own emoji and picture, through `whoami` and `/api/users/me/avatar`, and only for an Owner because every write under `/api/users` is Owner-only on the daemon.
The users list itself is unchanged (#8387) (@DaBlitzStein)
