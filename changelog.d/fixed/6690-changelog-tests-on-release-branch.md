Fix the two `xtask` changelog tests that run against the repo's own `CHANGELOG.md` failing on any release branch, which blocked the v2026.7.31 release PR (#6688) on a state the release flow itself creates.
`cargo xtask release` drains `## [Unreleased]` into the dated section it cuts, so on a `chore/bump-version-*` branch the section is empty — and `drains_the_repos_own_unreleased_section_without_tripping_the_guard` opens by asserting it is not, while `folds_into_the_repos_own_changelog` asserted a `### ` heading count that only holds when `### Changed` already exists.
Both now read through a helper that reconstitutes the pre-release shape by hoisting the newest dated section back into `[Unreleased]`, so the real-file coverage survives on release branches rather than being skipped there, and the subsection assertion is a delta that permits exactly the one heading the fold may legitimately create.
Doing that surfaced a second, older defect in the same two tests: they checked headings with `str::contains` / `str::matches`, which count substrings anywhere on a line, while the `awk` extractor they mirror anchors at column 0.
A curated bullet that quotes a heading in its prose — the #6628 entry says "appended its bullet to the single `## [Unreleased]` section" on an indented continuation line — therefore read as a boundary overrun and as 19 `[Unreleased]` headings.
Both checks are now line-anchored, matching the extractor.
This had never fired because the assertions had only ever run against an empty `[Unreleased]`.
(#6690) (@houko)
