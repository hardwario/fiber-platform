#!/bin/sh
# The `fiber platform` commands in a bare Debian container: no broker, no hardware, no settings.
# `just check-container` runs it as root, with the static program in /usr/local/bin.
set -eu

# expect <status> <command...>: the command must end with that exit status.
expect() {
    want=$1
    shift
    set +e
    "$@"
    got=$?
    set -e
    if [ "$got" -ne "$want" ]; then
        echo "FAILED: '$*' ended with $got, not $want"
        exit 1
    fi
}

echo "== fiber version --all"
fiber version --all
echo "== fiber platform list, with no /etc/fiber/platform.yaml"
fiber platform list
echo "== fiber platform status"
fiber platform status
echo "== fiber platform enable agent"
fiber platform enable agent
cat /etc/fiber/platform.yaml
test "$(stat -c %a /etc/fiber/platform.yaml)" = 644
echo "== fiber platform status: nothing is applied yet, so it ends with 3"
expect 3 fiber platform status
echo "== fiber platform disable mosquitto: refused, the agent needs it"
expect 1 fiber platform disable mosquitto
echo "== fiber platform disable agent"
fiber platform disable agent
fiber platform list
echo "== Every command ran."
