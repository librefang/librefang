An API error whose translation key the active language bundle did not define came back to the client as the key itself, so a `de` / `es` / `fr` / `zh-CN` client saw `api-error-agent-clone-spawn-failed` where the English sentence belonged.
The packs are not all the same size — nine locales are declared and five ship complete bundles — so a missing key is the ordinary case rather than an error, and the identifier must never reach a user-visible string.
A key the active bundle cannot answer is now answered in English first (#8450) (@DaBlitzStein)
