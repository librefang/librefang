#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
ENTRYPOINT="${ROOT}/deploy/docker-entrypoint.sh"
TEST_DIR=$(mktemp -d)
trap 'rm -rf "$TEST_DIR"' EXIT
MOCK_BIN="${TEST_DIR}/bin"
mkdir -p "$MOCK_BIN"
cat > "${MOCK_BIN}/id" <<'EOF'
#!/usr/bin/env sh
if [ "$1" = "-u" ]; then printf '1001\n'; else exec /usr/bin/id "$@"; fi
EOF
chmod +x "${MOCK_BIN}/id"

expect_failure() {
  local expected=$1
  shift
  local output status
  set +e
  output=$(env PATH="${MOCK_BIN}:/usr/bin:/bin" "$@" sh "$ENTRYPOINT" true 2>&1)
  status=$?
  set -e
  [[ $status -ne 0 ]]
  [[ "$output" == *"$expected"* ]]
}

for port in 0 65536 -1 invalid; do
  expect_failure 'PORT must be an integer from 1 to 65535' \
    LIBREFANG_HOME="${TEST_DIR}/port-${port}" PORT="$port"
done

for model in 'provider/model&debug' 'provider|model'; do
  expect_failure 'LIBREFANG_MODEL contains a forbidden character' \
    LIBREFANG_HOME="${TEST_DIR}/model-invalid" LIBREFANG_MODEL="$model"
done

missing_listen="${TEST_DIR}/missing-listen"
mkdir -p "$missing_listen"
printf 'model = "provider/model"\n' > "${missing_listen}/config.toml"
expect_failure 'config.toml has no api_listen key' LIBREFANG_HOME="$missing_listen" PORT=4545

missing_model="${TEST_DIR}/missing-model"
mkdir -p "$missing_model"
printf 'api_listen = "0.0.0.0:4545"\n' > "${missing_model}/config.toml"
expect_failure 'config.toml has no model key' \
  LIBREFANG_HOME="$missing_model" LIBREFANG_MODEL='provider/model'
