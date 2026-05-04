#!/usr/bin/env bash
#
# changelog-to-article.sh — scaffold articles/release-<date>.md from CHANGELOG.md
#
# `articles/release-<YYYY.M.D>.md` is consumed by two GitHub workflows on push
# to main:
#   - .github/workflows/devto-publish.yml     publishes / updates the dev.to post
#   - .github/workflows/release-notify.yml    posts a GitHub Discussion using
#                                             the article body
#
# Articles fell behind CHANGELOG.md after 2026-03-22 (#3397). This script makes
# generating one a single command.
#
# Usage:
#   bash scripts/changelog-to-article.sh <YYYY.M.D> [<git-tag>]
#
#   <YYYY.M.D> must match a `## [YYYY.M.D]` heading in CHANGELOG.md.
#   <git-tag>  defaults to `v<YYYY.M.D>`. CalVer tags often carry suffixes
#              (e.g. `v2026.4.27-beta6`); pass the actual tag to make
#              canonical_url accurate. The placeholder is safe to hand-edit
#              before pushing.
#
# Examples:
#   bash scripts/changelog-to-article.sh 2026.4.27
#   bash scripts/changelog-to-article.sh 2026.4.27 v2026.4.27-beta6
#
# Output: articles/release-<YYYY.M.D>.md (overwrites if it exists).

set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
    echo "usage: $0 <YYYY.M.D> [<git-tag>]" >&2
    exit 2
fi

DATE="$1"
TAG="${2:-v${DATE}}"

# Locate repo root so this script works from any cwd.
ROOT="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "${ROOT}" ]]; then
    echo "error: not inside a git repository" >&2
    exit 1
fi

CHANGELOG="${ROOT}/CHANGELOG.md"
OUT_DIR="${ROOT}/articles"
OUT="${OUT_DIR}/release-${DATE}.md"

if [[ ! -f "${CHANGELOG}" ]]; then
    echo "error: ${CHANGELOG} not found" >&2
    exit 1
fi
if [[ ! -d "${OUT_DIR}" ]]; then
    echo "error: ${OUT_DIR} not found" >&2
    exit 1
fi

# Validate date shape (YYYY.M.D — month/day not zero-padded, matches CHANGELOG).
if [[ ! "${DATE}" =~ ^[0-9]{4}\.[0-9]{1,2}\.[0-9]{1,2}$ ]]; then
    echo "error: <date> must look like YYYY.M.D (got '${DATE}')" >&2
    exit 2
fi

# Slice the `## [DATE]` block out of CHANGELOG.md, stopping at the next `## [`.
# Match a fixed string (the literal heading) rather than a regex so the dots
# in the date don't get interpreted as wildcards (BSD awk has no gensub).
HEADING="## [${DATE}]"
SECTION="$(awk -v h="${HEADING}" '
    index($0, h) == 1 && !found {found=1; next}
    found && /^## \[/ {exit}
    found {print}
' "${CHANGELOG}")"

if [[ -z "${SECTION//[[:space:]]/}" ]]; then
    echo "error: no '## [${DATE}]' section found in CHANGELOG.md" >&2
    exit 1
fi

# Strip leading/trailing blank lines from the slice.
SECTION_TRIMMED="$(printf '%s\n' "${SECTION}" | awk '
    NF {found=1}
    found {buf = buf $0 "\n"}
    END {sub(/\n+$/, "", buf); printf "%s", buf}
')"

# CHANGELOG anchor on GitHub: keep-a-changelog renders `## [2026.4.27] - 2026-04-27`
# as id="2026427---2026-04-27" but we link to the simpler `#YYYY-M-D` slug GitHub
# also recognises via the date suffix; fall back to the full file at worst.
ANCHOR_DATE="${DATE//./-}"

CANONICAL="https://github.com/librefang/librefang/releases/tag/${TAG}"
CHANGELOG_LINK="https://github.com/librefang/librefang/blob/main/CHANGELOG.md#${ANCHOR_DATE}"

# Heredoc with the same dev.to-friendly shape as the most recent hand-written
# articles (release-2026.3.22.md, release-2026.3.21.md): outer ```markdown
# fence (release-notify.yml strips it), front matter between ---, body below.
{
    printf '```markdown\n'
    printf -- '---\n'
    printf 'title: "LibreFang %s Released"\n' "${DATE}"
    printf 'published: true\n'
    printf 'description: "LibreFang v%s release notes — open-source Agent OS built in Rust"\n' "${DATE}"
    printf 'tags: rust, ai, opensource, release\n'
    printf 'canonical_url: %s\n' "${CANONICAL}"
    printf 'cover_image: https://raw.githubusercontent.com/librefang/librefang/main/public/assets/logo.png\n'
    printf -- '---\n\n'
    printf '# LibreFang %s Released\n\n' "${DATE}"
    printf 'LibreFang v%s ships the changes below. See the [full changelog](%s) for the complete list.\n\n' \
        "${DATE}" "${CHANGELOG_LINK}"
    printf '%s\n' "${SECTION_TRIMMED}"
    printf '\n## Links\n\n'
    printf -- '- [Full Changelog](%s)\n' "${CHANGELOG_LINK}"
    printf -- '- [GitHub Release](%s)\n' "${CANONICAL}"
    printf -- '- [GitHub](https://github.com/librefang/librefang)\n'
    printf -- '- [Discord](https://discord.gg/DzTYqAZZmc)\n'
    printf -- '- [Contributing Guide](https://github.com/librefang/librefang/blob/main/CONTRIBUTING.md)\n'
    printf '```\n'
} > "${OUT}"

echo "wrote ${OUT}"
echo "  date: ${DATE}"
echo "  tag:  ${TAG}"
echo
echo "Review the file, adjust the tag/canonical_url if the actual release tag"
echo "differs from the default placeholder, then commit and push to main."
echo "devto-publish.yml will pick it up on push."
