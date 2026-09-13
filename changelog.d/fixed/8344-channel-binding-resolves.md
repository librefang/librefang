An agent's channel view now distinguishes a bot bound to a different agent from one bound to an agent that does not exist.
  A `[[sidecar_channels]].agent` naming an agent that was never spawned, has since been deleted, or is simply misspelled delivers nowhere — the router resolves the name and skips the binding on a miss — but the instance list reported it exactly like a live binding to somebody else.
  Each instance now carries `resolves`. (#8344) (@houko)
