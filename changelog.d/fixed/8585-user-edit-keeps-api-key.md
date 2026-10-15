Editing a user from the dashboard, the identity wizard or a CSV import no longer revokes their API key.
Those clients never send `api_key_hash`, and the update treated the missing field as "clear"; an absent key now keeps the stored hash, while an explicit `null` or empty string still clears it. (#8585) (@houko)
