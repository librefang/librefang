The builtin-tools reference described `memory_recall` as "semantic search over the agent's memories" and gave `memory_store` an argument list (`content`, `category`, `scope`) it has never accepted.
Both tools are an exact-key key/value store.
The Memory Tools section now separates the two stores explicitly, documents the real arguments, and states the capability scopes each family needs. (#7820) (@houko)
