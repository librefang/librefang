Workflow runs now record which agent asked for them, and a spawned worker's spend rolls up to the agent that spawned it.
  Before this, a workflow started by an agent was indistinguishable from one an operator started by hand, and a step agent resolved from a `type` reference billed every owner's work to itself — so two teams sharing one `researcher` type had no way to tell their spend apart.
  Ownership is recorded on the *run* rather than on the executing agent, because find-or-spawn deliberately resolves a type to one shared canonical instance; attaching ownership to that instance would have made the second owner's runs report the first owner's.
  The owner is stamped once at creation and carried forward by resume and re-run, so re-running someone else's workflow does not silently transfer it to you.
  Billing is a second column rather than a rewrite of the existing one, which keeps quota enforcement pointed at the agent that actually made the call — attribution and enforcement stay independent, and per-agent limits behave exactly as before.
  The proposed `fresh = true` step flag is deliberately not part of this: per-step `session_mode = "new"` already isolates a step from a shared agent's history, and owner attribution already separates spend, so an extra agent instance per run would have bought only registry growth.
  See `docs/architecture/workflow-run-attribution.md`.
  (#7878) (@houko)
