A media provider no longer advertises functions it cannot actually perform, and one the registry already describes can be reached without editing config.toml.
Configuring such a provider used to *reduce* what it offered: the unconfigured entry repeated everything the registry said the service could do, and the moment it was wired up the answer narrowed to what the generic connector implements, so it vanished from the video section exactly when it started working.
Both states now report the same thing — what can actually be served.
Separately, a provider whose endpoint the registry states is now reachable from its API key alone; before, it stayed marked unconfigured with nothing to indicate that a hand-written endpoint override was the missing piece.
The list of media providers is also read fresh on each request rather than taken once at startup, so one added by a catalog update appears without a restart. (#8320) (@DaBlitzStein)
