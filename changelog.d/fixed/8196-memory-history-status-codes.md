Answer `GET /api/memory/items/{memory_id}/history` with 400 for a malformed memory id and 404 for one that matches no memory, instead of 500 for both.
The store reported both client mistakes as internal errors, so the route returned `Internal server error` with no indication of what was wrong and wrote an `ERROR` line into the daemon log for every mistyped or already-deleted id.
A client polling a memory it had just deleted could not tell "gone" from "the daemon is broken"; the route now matches the 404 its sibling update and delete endpoints already returned for the same id (#8196) (@houko)
