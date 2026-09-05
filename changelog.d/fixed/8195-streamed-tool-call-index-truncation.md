Stop the OpenAI-compatible streaming driver from silently discarding a parallel tool call when the provider sends a `tool_call` delta for an index it has already moved past.
The accumulator grew with `Vec::resize_with`, which shrinks as well as grows, so a late `id`, a trailing `arguments` fragment, or two entries in one frame in descending index order truncated every slot above the referenced index.
The consumer had already been told those calls started, so a dashboard or ACP client was left rendering a tool call that never completes, and the model's request was never executed.
Growth is now one-directional, keeping the single-allocation sparse growth without the truncation (#8195) (@houko)
