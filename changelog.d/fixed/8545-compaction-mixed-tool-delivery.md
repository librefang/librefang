Compaction no longer keeps a tool-result delivery while summarising away the tool call it answers.
The split guard only recognised a user message made of nothing but tool results, and only checked the first tool call id, so a mixed tool-result-plus-text delivery could open the kept window as an orphaned `role:"tool"` message that providers reject with a non-retryable 400 (#8587, #8545) (@houko)
