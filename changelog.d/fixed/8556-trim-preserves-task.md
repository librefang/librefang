History trimming now guarantees that the current turn's task message survives, rather than only that some user message does.
The post-trim guard used to accept any surviving user message, and when none survived it synthesized a fresh conversation from the raw message text, losing any sender prefix or attachments.
It now captures the current-turn task before trimming and tests its survival by position rather than by text, so an earlier message with identical text no longer reads as the task.
A drained task is re-inserted ahead of any trailing tool cycle so no `ToolUse`/`ToolResult` pair is split (#8556, #8557) (@DaBlitzStein)
