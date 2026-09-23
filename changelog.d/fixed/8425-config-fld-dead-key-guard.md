The dashboard dead-key check now covers the `config.fld_*` field labels, which it previously exempted wholesale because `ConfigPage` builds them from a template literal.
A new API test derives every label key the page can look up from the real `GET /api/config/schema` response and pins it in a golden list, and the locale test fails on any `config.fld_*` key outside that list.
It immediately found ten channel labels (`fld_discord`, `fld_slack`, `fld_telegram` and seven more) for fields `ChannelsConfig` no longer has; they were dead in all five locales and are removed (#8425) (@houko)
