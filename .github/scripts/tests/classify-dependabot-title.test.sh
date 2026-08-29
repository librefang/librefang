#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
SCRIPT="$ROOT/.github/scripts/classify-dependabot-title.sh"

assert_classification() {
  title=$1
  expected_type=$2
  expected_dep=$3
  expected=$(printf '%s\t%s' "$expected_type" "$expected_dep")
  actual=$(TITLE="$title" bash "$SCRIPT")
  [ "$actual" = "$expected" ] || {
    echo "FAIL: $title classified as $actual, expected $expected" >&2
    exit 1
  }
}

assert_classification 'chore(deps): bump serde from 1.0.1 to 1.0.2' \
  'version-update:semver-patch' serde
assert_classification 'Bump actions/checkout from v4.1.0 to v4.2.0' \
  'version-update:semver-minor' actions/checkout
assert_classification 'Bump crate from 01.02.003 to 2.0.0' \
  'version-update:semver-major' crate
assert_classification 'Bump crate from 2.0.0 to 1.9.0' unknown crate
assert_classification 'Bump crate from 1.2.3 to 1.2.3' unknown crate
assert_classification 'Bump crate from 1.2.3.4 to 1.2.3.5' unknown crate
assert_classification 'Bump crate from 1.2.3-beta.1 to 1.2.3' unknown crate
assert_classification 'Bump crate from 1.2.3 to 999999999999999999999.0.0' unknown crate
assert_classification 'Bump the rust-minor-patch group with 4 updates' \
  unknown rust-minor-patch
assert_classification 'Unrecognized Dependabot title' unknown unknown

echo 'classify-dependabot-title tests passed'
