`verify_max_retries` and `verify_agent_id` on the goal endpoints skipped the boundary validation their siblings already had.
A `verify_max_retries` above `u32::MAX` silently truncated to a small number instead of being rejected, and a negative or fractional value was indistinguishable from an absent field, unlike `max_iterations`, which already rejected both.
A non-string `verify_agent_id` was silently dropped instead of rejected, and update's hand-rolled check had the same gap, unlike `agent_id`, which already goes through the shared boundary helper.
Both fields now validate the same way their siblings do, and `verify_agent_id` is canonicalised the same way on write (#7785) (@DaBlitzStein)
