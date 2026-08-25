A usage-budget refusal now tells the operator which cap was hit and what it is set to.
The kernel already computed that detail — the agent, the window, the spend so far, this call's cost, and the limit — but the WebSocket surface matched on the error prefix and replaced the whole message with generic guidance, so an operator was told to "raise the matching limit" while the message withheld which limit that was.
Reported after three requests produced an unexplained refusal, with no way to tell from the message whether the cap was hourly, daily, monthly or token-based.
(#7907) (@houko)
