A registry checkout that arrives without an `agents/` directory no longer disables agent templates in silence.
The three places that resolved that directory — the runtime's post-sync fan-out, the hands registry's `base = "<template>"` lookup, and the kernel router's hand scan — each open-coded its own existence check and skipped its entire block on a miss, so "the registry ships no templates" and "the sync produced a checkout this code cannot read" were indistinguishable from outside the process, and the 24h cache TTL kept the degraded state around.
All three now share one resolver that logs at error level the first time a registry root fails to resolve, naming the path it tried, and reports again if the directory comes back and disappears a second time.
(#7851) (@houko)
