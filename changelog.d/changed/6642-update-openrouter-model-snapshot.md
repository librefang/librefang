Refresh the checked-in OpenRouter model snapshot used as the offline fallback catalog.
The runtime's live catalog remains authoritative whenever OpenRouter is configured, so this update only affects lookups made before the first live fetch completes (#6642) (@houko)
