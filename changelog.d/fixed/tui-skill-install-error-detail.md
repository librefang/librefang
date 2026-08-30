A failed skill install in the TUI now says what the daemon reported instead of a generic line.
`spawn_install_skill` collapsed every non-success outcome into one `_ =>` arm, so a 4xx carrying the actual reason — `YAML parse error at line 3`, from a broken marketplace skill — arrived as "Failed to install {slug}" with nothing else.
That reads like the daemon fell over, and sends the operator looking for a fault in their own install rather than in someone else's skill.
The rejected-install and never-reached-the-daemon cases are now separate arms: the first reports the `error` field from the response body, falling back to the status code when the body carries none rather than inventing a cause, and the second reports the transport error. (@DaBlitzStein)
