Proactive memory and the canonical cross-channel session no longer miss a turn whose session history was rewritten partway through.
Inline context compression and the repair that runs when a turn hits its iteration cap both change the length of the session, but the index marking where the turn began kept its old value, so end-of-turn recording saw an empty turn or one missing its prompt.
The index is now moved to where the turn sits in the rewritten history (#8734) (@houko)
