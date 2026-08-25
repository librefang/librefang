An agent sitting in a shared Slack or Telegram group can now answer "who is in this channel?".
The channel bridge has been persisting every group sender it observes into the `group_roster` table since that table landed, and `KernelHandle::roster_members` could always read it back — but nothing in the tree ever called it, so the roster was write-only in practice and the membership was invisible to agents and operators alike.
The new read-only `channel_members` tool exposes it: `user_id`, `display_name` and `username` for everyone the daemon has seen speak in a conversation, defaulting to the conversation the current message arrived on.
That `user_id` is what an agent needs to attribute a request to the person who made it when handing work to an external system.
Reading a conversation other than the one the turn arrived on is refused on the same terms `channel_send` refuses a cross-chat dispatch, so one group cannot enumerate another's membership.
Roster rows now also break a shared-display-name tie on the user id, because an unstable tail in a list that reaches the prompt invalidates provider prompt caches on unchanged content.
(#7865) (@houko)
