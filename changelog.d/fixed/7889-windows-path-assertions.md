A kernel test no longer fails on the Windows shard because it hard-coded a forward slash.
`a_missing_template_names_the_directories_searched` asserts that the "no such template" message names the directories it searched, but it looked for the literal `workspaces/agents` in a message that interpolates real paths — so on Windows the message said `workspaces\agents` and the assertion failed while the code under test was correct.
The expected fragment is now built with the platform separator, which is what the message itself uses.
(#7890) (@houko)
