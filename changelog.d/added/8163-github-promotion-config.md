A `[skills.promotion]` config section for the registry promotion flow, which previously had almost every GitHub-side value hardcoded or derived at runtime.
`api_base_url` makes "Propose to Registry" usable on GitHub Enterprise at all, where a compiled-in `api.github.com` left it with no workaround.
`commit_author_name` / `commit_author_email` fix the attribution of the pushed commits, which GitHub otherwise credits to whoever owns the token — wrong for a shared or service token, and not something an operator could correct.
`fork_owner`, `base_branch`, `head_branch_prefix` and a `mode` of `fork` or `direct_push` cover the organisation-owned fork, the non-default target branch, the branch-naming convention and the internal registry nobody is meant to fork.
Every field is optional and reproduces the previous behaviour when unset, so an installation that configures nothing sees no change. (#8163) (@DaBlitzStein)
