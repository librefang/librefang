Instantiating a workflow template resolves each parameter placeholder once instead of replacing them one map entry at a time.
The old loop rescanned text that an earlier parameter had inserted, so a parameter whose value carried another parameter's placeholder expanded or stayed literal depending on hash-map iteration order — and undeclared keys from the request body are substituted too, so any caller could reach that shape.
Unlike the run-time expansion this one is durable: the prompt it produces is written onto the instantiated workflow's steps, so the same template with the same parameters could be persisted as two different workflows, each of which then drives an agent with its own text.
Both substitution sites now share one order-independent expansion helper (#8197) (@houko)
