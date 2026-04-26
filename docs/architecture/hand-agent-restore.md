## Hand Agent Restore Path

Hand-managed agents do not restore through the generic SQLite boot path.

- Regular agents are rehydrated from SQLite by `load_all_agents`.
- Hand agents (`is_hand = true`) are skipped there and are restored only from `data/hand_state.json` via `activate_hand_with_id`.
- The SQLite rows for hand agents are secondary state used for session continuity and orphan cleanup.

This split is intentional: hand runtime state is derived from the hand definition plus the persisted per-role runtime overrides in `hand_state.json`. Rebuilding through `activate_hand_with_id` keeps hand activation, default-model resolution, settings injection, and override replay on one path instead of trying to reconstruct the same state from drifted SQLite blobs.

Operational consequence:

- If `hand_state.json` is missing, no hands are restored.
- If `hand_state.json` is unreadable, hand restore is skipped and orphaned hand-agent GC must also be skipped to avoid deleting the only remaining hand-agent metadata.

In short: `hand_state.json` is the source of truth for active hand instances across restarts; SQLite hand-agent rows are not.
