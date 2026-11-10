The history fold now finds the running turn in the message list it actually folds, so a turn's own tool results stay out of reach even when the session holds System messages.
The #8653 fix bounded the fold with an index into the stored session, but the fold walks the LLM working copy, which leaves out System messages, gains injected context at its head, and is rewritten by repair and compression.
In a session with System messages that index landed past the turn's real start, so the turn's first tool results were still folded and the agent went back to re-fetching them.
The boundary is now the turn's first new tool call, found by tool id, which stays correct through each of those rewrites (#8731) (@houko)
