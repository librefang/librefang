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
#   bash scripts/changelog-to-article.sh <YYYY.M.D> [<git-tag>] [--force]
#
#   <YYYY.M.D> must match a `## [YYYY.M.D]` heading in CHANGELOG.md.
#   <git-tag>  defaults to `v<YYYY.M.D>`. CalVer tags often carry suffixes
#              (e.g. `v2026.4.27-beta6`); pass the actual tag to make
#              canonical_url accurate. The placeholder is safe to hand-edit
#              before pushing.
#   --force    replaces an existing generated article. Without it, an existing
#              file is preserved and the command fails.
#
# Examples:
#   bash scripts/changelog-to-article.sh 2026.4.27
#   bash scripts/changelog-to-article.sh 2026.4.27 v2026.4.27-beta6
#   bash scripts/changelog-to-article.sh 2026.4.27 v2026.4.27-beta6 --force
#
# Output: articles/release-<YYYY.M.D>.md.

set -euo pipefail

if [[ $# -lt 1 || $# -gt 3 ]]; then
    echo "usage: $0 <YYYY.M.D> [<git-tag>] [--force]" >&2
    exit 2
fi

DATE="$1"
shift
TAG="v${DATE}"
FORCE=false

if [[ $# -gt 0 && "$1" != "--force" ]]; then
    TAG="$1"
    shift
fi
if [[ $# -gt 0 && "$1" == "--force" ]]; then
    FORCE=true
    shift
fi
if [[ $# -ne 0 ]]; then
    echo "usage: $0 <YYYY.M.D> [<git-tag>] [--force]" >&2
    exit 2
fi

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
IFS=. read -r _ MONTH DAY <<< "${DATE}"
if (( 10#${MONTH} < 1 || 10#${MONTH} > 12 || 10#${DAY} < 1 || 10#${DAY} > 31 )); then
    echo "error: <date> month/day is out of range (got '${DATE}')" >&2
    exit 2
fi

# Git permits several characters that are unsafe in a URL path or unquoted
# YAML scalar. Release tags only need this conservative CalVer-safe subset.
if [[ ! "${TAG}" =~ ^[A-Za-z0-9._+-]+$ ]]; then
    echo "error: <git-tag> contains unsupported characters (got '${TAG}')" >&2
    exit 2
fi

if [[ -e "${OUT}" && "${FORCE}" != true ]]; then
    echo "error: ${OUT} already exists; pass --force to replace it" >&2
    exit 1
fi

# Slice the `## [DATE]` block out of CHANGELOG.md, stopping at the next `## [`.
# Match a fixed string (the literal heading) rather than a regex so the dots
# in the date don't get interpreted as wildcards (BSD awk has no gensub).
HEADING="## [${DATE}]"
if ! EXTRACTED="$(awk -v h="${HEADING}" '
    function trim_fence_indent(line, spaces) {
        spaces = 0
        while (substr(line, 1, 1) == " ") {
            spaces++
            line = substr(line, 2)
        }
        if (spaces > 3) return ""
        return line
    }

    function opening_fence_length(line, trimmed, char, run, rest) {
        trimmed = trim_fence_indent(line)
        if (trimmed == "") return 0
        char = substr(trimmed, 1, 1)
        if (char != "`" && char != "~") return 0
        run = 0
        while (substr(trimmed, run + 1, 1) == char) run++
        if (run < 3) return 0
        rest = substr(trimmed, run + 1)
        if (char == "`" && index(rest, "`") != 0) return 0
        opening_char = char
        return run
    }

    function is_closing_fence(line, expected_char, minimum_run,
                              trimmed, run, rest) {
        trimmed = trim_fence_indent(line)
        if (trimmed == "" || substr(trimmed, 1, 1) != expected_char) return 0
        run = 0
        while (substr(trimmed, run + 1, 1) == expected_char) run++
        if (run < minimum_run) return 0
        rest = substr(trimmed, run + 1)
        return rest ~ /^[[:space:]]*$/
    }

    {
        fence_line = 0
        if (!in_fence && (opening_length = opening_fence_length($0)) > 0) {
            in_fence = 1
            fence_char = opening_char
            fence_length = opening_length
            fence_line = 1
        } else if (in_fence && is_closing_fence($0, fence_char, fence_length)) {
            in_fence = 0
            fence_char = ""
            fence_length = 0
            fence_line = 1
        }

        if (!found && !in_fence && !fence_line && index($0, h) == 1 &&
            substr($0, length(h) + 1) ~ /^([[:space:]]|$)/) {
            found = 1
            print "HEADING\t" $0
            next
        }

        if (found) {
            if (!in_fence && !fence_line && /^## \[/) exit
            print "BODY\t" $0
        }
    }
' "${CHANGELOG}")"; then
    echo "error: failed to read ${CHANGELOG}" >&2
    exit 1
fi

RELEASE_HEADING="$(printf '%s\n' "${EXTRACTED}" | awk -F '\t' '$1 == "HEADING" {sub(/^HEADING\t/, ""); print; exit}')"
SECTION="$(printf '%s\n' "${EXTRACTED}" | sed -n 's/^BODY\t//p')"

if [[ -z "${RELEASE_HEADING}" || -z "${SECTION//[[:space:]]/}" ]]; then
    echo "error: no '## [${DATE}]' section found in CHANGELOG.md" >&2
    exit 1
fi

# Strip leading/trailing blank lines from the slice.
SECTION_TRIMMED="$(printf '%s\n' "${SECTION}" | awk '
    NF {found=1}
    found {buf = buf $0 "\n"}
    END {sub(/\n+$/, "", buf); printf "%s", buf}
')"

# Derive GitHub's heading slug from the complete release heading. For example,
# `## [2026.4.27] - 2026-04-27` becomes `2026427---2026-04-27`.
ANCHOR="$(printf '%s' "${RELEASE_HEADING#\#\# }" \
    | tr '[:upper:]' '[:lower:]' \
    | sed -e 's/\[//g' -e 's/\]//g' -e 's/\.//g' -e 's/[[:space:]]/-/g')"

CANONICAL="https://github.com/librefang/librefang/releases/tag/${TAG}"
CHANGELOG_LINK="https://github.com/librefang/librefang/blob/main/CHANGELOG.md#${ANCHOR}"

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
