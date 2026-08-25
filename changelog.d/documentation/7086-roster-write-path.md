Corrected three comments that described the group roster inaccurately, and locked the roster write path with tests.
  `extract_group_members` in the channel bridge claimed to persist bulk membership metadata to the roster store; it never did, and the only roster writer is the per-sender `upsert_sender_into_roster`, which records one row per person actually observed speaking.
  That distinction decides who `channel_dm` is allowed to reach, so a comment implying the wider set is worth more than a typo.
  The memory crate still pointed readers at a `group_members` tool that was renamed `channel_members` before it shipped, and the Slack sidecar still said display names are never resolved, which stopped being true when `SLACK_RESOLVE_DISPLAY_NAMES` landed.
  The new tests pin the junction the whole feature rests on: a resolved display name has to survive the bridge to reach the roster column `channel_members` reads back, and direct messages, senderless group messages and blank ids must not be recorded at all.
  (#7901) (@houko)
