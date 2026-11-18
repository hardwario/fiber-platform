#!/bin/bash
# Sends one request to the FIBER's agent and prints its answer (docs/examples.md).
#
#   request.sh <op> [params as JSON]
#   request.sh buzzer.beep '{"pattern":"double"}'
#
# Exit status: 0 the answer says "ok":true; 1 it does not; 2 no answer came.
# FIBER_HOST, FIBER_PORT, FIBER_USER and FIBER_PASSWORD say where the broker is (on the FIBER:
# 127.0.0.1:1883, no login).
set -uo pipefail

op=${1:?usage: request.sh <op> [params as JSON]}
params=${2:-null}
login=()
if [ -n "${FIBER_USER:-}" ]; then
    login=(-u "$FIBER_USER" -P "${FIBER_PASSWORD:-}")
fi
# New for every request: the agent drops an id it has seen.
id="sh-$(date +%s%N)-$$"

# mosquitto_rr subscribes to the answer's topic first, then sends the request, and waits for the
# answer: every request gets one within 10 s (firmware.flash: within 2 minutes).
answer=$(mosquitto_rr -h "${FIBER_HOST:-127.0.0.1}" -p "${FIBER_PORT:-1883}" "${login[@]}" -q 1 \
    -t fiber/cmd -e "fiber/rsp/$id" -W 130 \
    -m "{\"id\":\"$id\",\"op\":\"$op\",\"params\":$params}") || {
    echo "no answer from the agent" >&2
    exit 2
}
echo "$answer"
case "$answer" in
    *'"ok":true'*) exit 0 ;;
    *) exit 1 ;;
esac
