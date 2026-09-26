A translation key that the active language bundle could not answer was rendered as the key itself, so the interface showed `translation:agents.builtin.foo.name` where the name belonged.
The packs are not all the same size — nine locales are declared and five ship complete bundles — so a missing key is the ordinary case rather than an error, and the identifier must never reach a user-visible string.
A key the active bundle cannot answer is now answered in English first, which also means the English pack is parsed once per thread rather than once per translator (#8450) (@DaBlitzStein)
