Rejected malformed Telegram poll options and missing or out-of-range quiz answers before issuing a Bot API request.
Also enforced the Bot API's question, option, and explanation length bounds locally so an oversize poll fails fast instead of a 400 from Telegram. (#6887) (@houko)
