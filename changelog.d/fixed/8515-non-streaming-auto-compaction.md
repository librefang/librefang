Automatic history compaction now runs for non-streaming turns too, so sessions driven by channel bridges, cron jobs, `agent_send` and the REST message route are summarised when they cross `[compaction] threshold_messages` or the token threshold.
Both automatic compaction checks used to live only in the streaming sender, so those sessions grew until `max_history_messages` and then had their oldest messages trimmed away instead, and the `/compact` hint in the trim warning was the only way to get a summary.
(#8515) (@houko)
