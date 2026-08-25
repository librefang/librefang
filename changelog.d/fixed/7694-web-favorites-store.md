Website favorites now use React's tear-safe external-store contract and one shared cross-tab storage listener. (#7694) (@houko)

Favorite lists keep stable identities across unrelated renders, and failed persistence no longer commits an in-memory change. (#7694) (@houko)
