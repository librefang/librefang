Ephemeral worker turns record their usage again.
`UsageRecord` gained a `billed_agent_id` field and the ephemeral spawn path was the one constructor left without it, so `librefang-kernel` stopped compiling as soon as both changes were on `main` together.
Neither pull request was wrong on its own — each was green against a base that did not yet contain the other — which is the failure the repository's own merge guidance warns about when two independently green changes land in sequence.
The worker bills to its parent, matching what the attribution helper returns for a parented agent.
(#7887) (@houko)
