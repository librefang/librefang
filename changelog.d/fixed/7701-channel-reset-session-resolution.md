`/new`, `/reboot` and `/compact` typed in a channel chat now resolve their target session through the same function the inbound message path uses, so they clear the conversation the user is actually looking at.
The reset commands used to re-derive the session id themselves, which is only correct for as long as two independently written derivations agree; when they drifted apart the command acked success against a session holding no messages while the visible history survived untouched.
Both ends now call the kernel's `channel_session_id`, which also carries the reserved-channel guard that a hand-rolled derivation could forget.
The `/new` ack reports how many messages were cleared, so a no-op is visible from the chat itself, and says so explicitly when the count could not be read rather than printing zero for a lookup that failed.
(#7701) (@DaBlitzStein)
