Rejected malformed non-object Telegram media-group entries instead of silently dropping them from the outgoing group.
  Also rejected a media-group item missing its required `url` field instead of sending Telegram an empty `media` value. (#6870) (@houko)
