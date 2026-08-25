#!/bin/sh
set -eu

container="${DOCKER_SMOKE_CONTAINER:-librefang-ci}"
image="${DOCKER_SMOKE_IMAGE:-librefang:ci}"
attempts="${DOCKER_SMOKE_ATTEMPTS:-30}"
interval="${DOCKER_SMOKE_INTERVAL_SECONDS:-2}"
curl_timeout="${DOCKER_SMOKE_CURL_TIMEOUT_SECONDS:-2}"
base_url="${DOCKER_SMOKE_BASE_URL:-http://127.0.0.1:4545}"

docker run -d --name "$container" -p 4545:4545 "$image"

attempt=1
while [ "$attempt" -le "$attempts" ]; do
  running=$(docker inspect --format '{{.State.Running}}' "$container" 2>/dev/null || printf 'false')
  if [ "$running" != true ]; then
    echo "::error::Container $container exited before becoming ready" >&2
    docker logs "$container" 2>&1 || true
    exit 1
  fi

  if curl --max-time "$curl_timeout" -fsS "$base_url/api/health" >/dev/null \
    && curl --max-time "$curl_timeout" -fsS "$base_url/api/ready" >/dev/null; then
    echo "✓ /api/health and /api/ready responded on attempt $attempt"
    exit 0
  fi

  sleep "$interval"
  attempt=$((attempt + 1))
done

echo "::error::Container stayed up but did not become healthy and ready" >&2
docker logs "$container" 2>&1 || true
exit 1
