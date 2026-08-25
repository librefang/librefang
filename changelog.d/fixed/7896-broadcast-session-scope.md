`/new`, `/reboot` and `/compact` now clear the session a broadcast chat is actually talking in.
Broadcast fan-out was the one channel dispatch that reached the kernel without a `SenderContext`, and the kernel derives the per-chat `SessionId::for_sender_scope` only when a sender context is present — so those turns accumulated in the agent's canonical session, the one the dashboard chat writes to, while the reset commands addressed a per-chat session nothing had ever written to.
The command acked success and the bot went on answering out of the history the user had just asked it to forget.
Broadcast dispatch now carries the same sender context every other channel turn carries, so each target keeps its own session for the chat, and the three reset commands cover every agent in the fan-out rather than the single agent the router chain happens to resolve.
The canonical-versus-derived session framing is @DaBlitzStein's, from #7701. (#7896) (@houko)
