Add Mixture-of-Agents (MoA) as a routing mode, so a turn can be advised by several models and then answered by one.
When an agent's provider is `moa`, N advisor models produce private, text-only advice in parallel and an aggregator model consumes it as context while remaining the acting model — answering the user and driving tools, not summarising.
It ships as a composite `LlmDriver` (following the `FallbackDriver` precedent), so the agent loop is unchanged; MoA is a routing decision, not a capability.
Advisors run against a flattened, per-advisor context-window-trimmed view of the conversation, and their token usage is billed once per fan-out and cached with the response so cache hits are not re-charged (#6699) (@leszek3737)
