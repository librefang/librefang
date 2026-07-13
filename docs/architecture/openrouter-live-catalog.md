# OpenRouter Live Catalog and Model Migration (#6384)

OpenRouter's model inventory changes independently of LibreFang's release cycle.
To prevent failures from delisted models, LibreFang resolves OpenRouter models dynamically at runtime.

## Live Catalog Resolution

The runtime loads a checked-in, embedded snapshot (`openrouter-models.snapshot.json`) by default.
When an OpenRouter API key is configured, the system lazily fetches the live `/models` endpoint to update its memory catalog.
The live catalog data is cached with a Time-To-Live (TTL) to avoid excessive API requests.

### Refresh Policy and Cooldown

- **Lazy loading:** A refresh is triggered in the background on startup or when the catalog is read.
- **Failures and cooldown:** If a fetch fails or is rate-limited, the system falls back to the embedded snapshot and enforces a 60-second retry cooldown.
- **Concurrent requests:** Concurrent callers within the cooldown window are rejected immediately and reuse existing cached data; only one refresh proceeds per 60-second window.

## Default Model Sync and Migration

When the live catalog refreshes or the default provider key changes, the default model is synced.

### Sync Behavior

- **Intra-provider narrowing:** If a default model on a provider changes (such as a free-model update), the system runs a narrow sync.
- **Narrow sync target:** Only agents specifically pinned to the delisted model are updated, while agents deliberately pinned to other models remain untouched.
- **Full provider switch:** If the default provider is changed entirely, all dashboard default-tracking agents migrate to the new provider's default.

### Assistant Exclusion

- **Explicit pinning:** The built-in `assistant` agent is excluded from automatic default-model migrations.
- **Intent preservation:** Once the user explicitly selects a model for the assistant in the dashboard, that choice is treated as a pin and preserved.
