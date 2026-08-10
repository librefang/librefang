Reject non-object `params` on known Python sidecar commands as a recoverable protocol error. (@houko)
Truthy arrays, strings, booleans, or numbers previously escaped `parse_command` as `AttributeError`, killed the reader task, and left the sidecar waiting forever instead of reporting the malformed frame and processing the next command.
Unknown future command methods retain their raw parameter shape for forward compatibility.
