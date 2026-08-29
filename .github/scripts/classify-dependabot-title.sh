#!/usr/bin/env bash
set -euo pipefail

: "${TITLE:?TITLE is required}"

if [[ "$TITLE" =~ [Bb]ump[[:space:]]+the[[:space:]]+([a-zA-Z0-9_-]+)[[:space:]]+group ]]; then
  printf 'unknown\t%s\n' "${BASH_REMATCH[1]}"
  exit 0
fi

if [[ ! "$TITLE" =~ [Bb]ump[[:space:]]+([^[:space:]]+)[[:space:]]+from[[:space:]]+([^[:space:]]+)[[:space:]]+to[[:space:]]+([^[:space:]]+) ]]; then
  printf 'unknown\tunknown\n'
  exit 0
fi

dep_name="${BASH_REMATCH[1]}"
old_ver="${BASH_REMATCH[2]#v}"
new_ver="${BASH_REMATCH[3]#v}"
IFS='.' read -r o_maj o_min o_pat remainder <<< "$old_ver"
IFS='.' read -r n_maj n_min n_pat new_remainder <<< "$new_ver"
o_maj="${o_maj:-0}"; o_min="${o_min:-0}"; o_pat="${o_pat:-0}"
n_maj="${n_maj:-0}"; n_min="${n_min:-0}"; n_pat="${n_pat:-0}"

for value in "$o_maj" "$o_min" "$o_pat" "$n_maj" "$n_min" "$n_pat"; do
  # Bash arithmetic is signed and fixed-width. Reject unusually large
  # components instead of allowing an overflow to change the bump class.
  if ! [[ "$value" =~ ^[0-9]+$ ]] || [ "${#value}" -gt 9 ]; then
    printf 'unknown\t%s\n' "$dep_name"
    exit 0
  fi
done
if [ -n "${remainder:-}" ] || [ -n "${new_remainder:-}" ]; then
  printf 'unknown\t%s\n' "$dep_name"
  exit 0
fi

old_major=$((10#$o_maj)); old_minor=$((10#$o_min)); old_patch=$((10#$o_pat))
new_major=$((10#$n_maj)); new_minor=$((10#$n_min)); new_patch=$((10#$n_pat))

if (( new_major > old_major )); then
  update_type='version-update:semver-major'
elif (( new_major == old_major && new_minor > old_minor )); then
  update_type='version-update:semver-minor'
elif (( new_major == old_major && new_minor == old_minor && new_patch > old_patch )); then
  update_type='version-update:semver-patch'
else
  update_type='unknown'
fi

printf '%s\t%s\n' "$update_type" "$dep_name"
