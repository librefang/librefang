Render an agent's avatar wherever the agent is listed, not only in the drawer that sets it.
The image was settable but invisible: the drawer was the single place in the SPA that drew it, so a list of twenty-two agents, the chat's agent picker and every message bubble went on showing initials for an agent that had a picture.
The agent list, the picker and the message bubbles now draw it, falling back to the agent's emoji and then to its initials.
The identity mutations invalidate the dashboard snapshot the list rows read from, and the daemon evicts that snapshot's 900 ms memo on every identity and avatar write, so an upload appears as soon as the mutation answers instead of waiting out the snapshot's five-second poll (#8371) (@DaBlitzStein)
