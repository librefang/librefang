Fix Nextcloud Talk adapter to use real user identity (actor_id) instead
of room tokens, correctly classify DM vs group messages from Talk room
type, resolve channel_dm targets via a participant-to-DM-room map, stamp
group_messages with roster metadata for channel_members, and persist
per-room watermarks to disk so supervisor restarts don't replay history
(#8770) (@j5bart)
