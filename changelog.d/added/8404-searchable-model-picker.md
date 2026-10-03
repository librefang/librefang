Add a searchable `ModelPicker`: a two-level provider → model select that owns its own search, replacing the free-text model inputs where an operator was expected to know a model id by heart.
It is extracted from the switcher buried inside `ChatPage`, which nothing else could reuse, so the agent's model, the three complexity tiers, the fallbacks and the per-modality routes can all offer a real choice instead of a text box.
The provider comes back alongside the model because a model id is only unique within its provider, which is also why a row is never marked active on a matching id alone.
A "Custom…" row stays, because the catalog is built from live discovery and a list-only control would lock an operator out of configuring a model exactly when discovery is broken.
Strings reuse the existing `chat.*` keys rather than a new namespace, so the picker renders translated in every locale instead of waiting on five translations.
No production caller consumes it yet — the wiring lands separately. (#8404) (@DaBlitzStein)
