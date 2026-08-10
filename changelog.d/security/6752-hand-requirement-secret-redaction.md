Stop `GET /api/hands` from returning plaintext values for satisfied environment-variable requirements, which exposed host credentials and other sensitive process configuration to any caller allowed to list Hands.
  Requirement status now reports only whether each variable is present while preserving the existing Dashboard save contract (#6752) (@houko)
