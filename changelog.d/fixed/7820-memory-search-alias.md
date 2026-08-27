**`memory_search` no longer silently resolves to exact-key key/value recall.**
It was aliased to `memory_recall`, so a model calling the single most natural name for "search my memory" got a hashmap lookup instead.
The call did not fail — it returned "not found" for a key that was never a key, and the model concluded the memory was gone.
The alias now points at the real semantic search tool, and `memory_add` / `memory_forget` resolve to their semantic counterparts too.
The descriptions of the three key/value tools now say what they do *not* do: `memory_recall` states that it matches the key character for character and names `memory_semantic_search` as the tool that searches by meaning.
A tool description is the only thing a model has to choose between two stores with confusingly similar names, and these three were describing themselves as "the agent's memory". (#7820) (@houko)
