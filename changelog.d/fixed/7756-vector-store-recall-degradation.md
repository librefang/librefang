A single unusable ID from an external vector-store backend no longer denies the entire recall.
`recall_via_vector_store` parsed every ANN-returned ID as a UUID with `?`, so one malformed row in a result set of fifty threw away the forty-nine memories that would have hydrated fine — a denial of service handed to whoever controls the backend's ID column, and inconsistent with the hydrate loop three lines below, which had always dropped an ID SQLite did not recognise.
A non-UUID ID is now dropped with a `WARN` naming the backend, and the rest of the result set is hydrated in ANN order.
(#7883) (@houko)
