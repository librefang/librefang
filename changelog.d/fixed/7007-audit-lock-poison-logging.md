Audit-log lock recovery from a poisoned state is now logged with the specific state involved — entries, tip, chain anchor, or load-error — instead of recovering silently across every accessor.
Recording, verification, and retention all continue to operate correctly after recovery, with the hash chain's integrity preserved (#7007) (@houko)
