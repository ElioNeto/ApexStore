#!/usr/bin/env bash
set -euo pipefail

# Exercise the built image's default DATA_DIR and HEALTHCHECK. Recreating the
# container must preserve flushed data through the mounted volume.
image="${1:?usage: test-container.sh IMAGE}"
container="apexstore-persistence-${GITHUB_RUN_ID:-$$}"
volume="${container}-data"
cleanup() {
  docker rm -f "$container" >/dev/null 2>&1 || true
  docker volume rm "$volume" >/dev/null 2>&1 || true
}
trap cleanup EXIT

docker volume create "$volume" >/dev/null
start_container() {
  docker run -d --name "$container" -e API_AUTH_ENABLED="${1:-false}" \
    -p 127.0.0.1::8080 -v "$volume:/data" "$image" >/dev/null
  for _ in {1..60}; do
    if [[ "$(docker inspect --format '{{.State.Health.Status}}' "$container")" == healthy ]]; then
      return 0
    fi
    sleep 1
  done
  docker logs "$container"
  docker inspect --format '{{json .State.Health}}' "$container"
  return 1
}
endpoint() {
  printf 'http://%s' "$(docker port "$container" 8080/tcp)"
}

start_container
curl --fail --silent --show-error -X PUT "$(endpoint)/keys/container-persistence" \
  -H 'Content-Type: application/json' -d '{"value":"survives-recreation"}'
curl --fail --silent --show-error -X POST "$(endpoint)/admin/flush" -H 'Content-Type: application/json'
docker rm -f "$container" >/dev/null
start_container
curl --fail --silent --show-error "$(endpoint)/keys/container-persistence" \
  | python3 -c 'import json,sys; result=json.load(sys.stdin); assert result["value"] == "survives-recreation", result'

# The same image probe must stay healthy with authentication enabled while data
# and detailed health endpoints remain protected.
docker rm -f "$container" >/dev/null
start_container true
for path in /keys/container-persistence /health/readiness /health/startup /health/check; do
  status="$(curl --silent --show-error -o /dev/null -w '%{http_code}' "$(endpoint)$path")"
  if [[ "$status" != 401 ]]; then
    echo "Expected authentication to protect $path, got HTTP $status" >&2
    exit 1
  fi
done
