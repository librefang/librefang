The manifest editor asked for model ids in free-text boxes — the three routing tiers, `pinned_model`, and each fallback's provider and model — so configuring one meant knowing an id by heart.
Every one of those fields now offers the live catalog.
The tiers and `pinned_model` take a flat, model-only shape rather than the provider-then-model drill-down, because they do not hold a pair: they hold a bare model name the daemon resolves against the global catalog (`ModelCatalog::find_model` / `resolve_alias` in `crates/librefang-runtime/src/routing.rs`), so writing `provider/model` into one would not resolve at all.
One flat list over a catalog this size is only navigable because each row is labelled with its provider and the search matches on it too.
Fallbacks do hold a pair, so each is one provider+model control instead of two text boxes, while `api_key_env` and `base_url` stay as they were.
The main model field is deliberately left alone: it is already a controlled pair of selects with granular per-field validation and a documented free-text fallback for when discovery is broken, so it was not one of the free-text sites this was about. (#8405) (@DaBlitzStein)
