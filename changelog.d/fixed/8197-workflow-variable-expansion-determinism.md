Workflow step prompts are now expanded in a single pass, so the text an agent receives no longer depends on hash-map iteration order.
Expansion ran one `String::replace` per variable against the growing result, which meant it rescanned text an earlier substitution had just inserted.
When a variable's value contained something shaped like another variable — an earlier step's output quoting a placeholder, or a top-level key of the caller's run input — that token was substituted or left literal depending on where `HashMap` happened to order the two keys, and that order varies between processes and between a run and its resume.
The same workflow with the same inputs could therefore send two different prompts, which is both a reproducibility problem and a silent provider prompt-cache miss.
Each placeholder found in the template is now resolved exactly once from the variable map, so a value is treated as content rather than as a further template (#8197) (@houko)
