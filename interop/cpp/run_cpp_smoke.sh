#!/usr/bin/env bash
# End-to-end smoke test of the C++ harness against itself:
#   qi-cpp-sd  <-  qi-cpp-service  <-  qi-cpp-client / qi-cpp-echo-client
#
# Usage: interop/cpp/run_cpp_smoke.sh [build-dir]
# Exit code 0 only if every client scenario passed.
set -u

BUILD_DIR="${1:-$(cd "$(dirname "$0")" && pwd)/build}"
SD="$BUILD_DIR/qi-cpp-sd"
SERVICE="$BUILD_DIR/qi-cpp-service"
CLIENT="$BUILD_DIR/qi-cpp-client"
ECHO_CLIENT="$BUILD_DIR/qi-cpp-echo-client"
LOG_DIR="${LOG_DIR:-$BUILD_DIR/smoke-logs}"
mkdir -p "$LOG_DIR"

for bin in "$SD" "$SERVICE" "$CLIENT" "$ECHO_CLIENT"; do
  [ -x "$bin" ] || { echo "missing binary: $bin" >&2; exit 2; }
done

SD_PID=""
SERVICE_PID=""
cleanup() {
  for pid in $SERVICE_PID $SD_PID; do
    [ -n "$pid" ] && kill -TERM "$pid" 2>/dev/null
  done
  for pid in $SERVICE_PID $SD_PID; do
    [ -n "$pid" ] && wait "$pid" 2>/dev/null
  done
}
trap cleanup EXIT

wait_for_line() { # file pattern timeout_s
  local file="$1" pattern="$2" timeout="$3" i=0
  while ! grep -q "$pattern" "$file" 2>/dev/null; do
    sleep 0.1
    i=$((i + 1))
    if [ "$i" -ge $((timeout * 10)) ]; then
      echo "timeout waiting for '$pattern' in $file" >&2
      cat "$file" >&2
      return 1
    fi
  done
}

"$SD" --qi-listen-url tcp://127.0.0.1:0 > "$LOG_DIR/sd.out" 2> "$LOG_DIR/sd.err" &
SD_PID=$!
wait_for_line "$LOG_DIR/sd.out" '^LISTENING ' 10 || exit 1
SD_URL="$(sed -n 's/^LISTENING //p' "$LOG_DIR/sd.out" | head -n1)"
echo "service directory: $SD_URL"

"$SERVICE" --qi-url "$SD_URL" --qi-listen-url tcp://127.0.0.1:0 > "$LOG_DIR/service.out" 2> "$LOG_DIR/service.err" &
SERVICE_PID=$!
wait_for_line "$LOG_DIR/service.out" '^READY' 10 || exit 1
echo "service ready"

echo "--- qi-cpp-echo-client"
"$ECHO_CLIENT" --qi-url "$SD_URL" 2> "$LOG_DIR/echo-client.err"
ECHO_RC=$?
echo "echo client exit code: $ECHO_RC"

echo "--- qi-cpp-client"
"$CLIENT" --qi-url "$SD_URL" --scenarios all 2> "$LOG_DIR/client.err" | tee "$LOG_DIR/client.out"
CLIENT_RC=${PIPESTATUS[0]}
echo "client exit code: $CLIENT_RC"

[ "$ECHO_RC" -eq 0 ] && [ "$CLIENT_RC" -eq 0 ]
