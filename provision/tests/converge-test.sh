#!/bin/bash
# The converge test (`just check-converge`), in a container with systemd running. The release to
# install is in /dist (`just release`). Runs as root.
#
#   1. install and apply, through the installer, with a seed: Mosquitto comes up
#   2. apply again: nothing changes
#   3. apply with the network taken away: it passes, as nothing needs installing
#   4. a role made to fail: the others end ok, the apply ends "partly", the service tries again
#      and then ends ok; the installer ends with 20 for it
#   5. a restart needed: the installer ends with 10 (with FIBER_NO_REBOOT=1)
set -euo pipefail

say() { printf '\n### %s\n' "$*"; }
fail() {
    printf 'FAILED: %s\n' "$*"
    journalctl -u fiber-platform-apply --no-pager | tail -n 30 || true
    tail -n 60 /var/log/fiber/platform.log 2> /dev/null || true
    exit 1
}

# expect <status> <command...>: the command must end with that status.
expect() {
    local want=$1 got=0
    shift
    "$@" || got=$?
    [ "$got" -eq "$want" ] || fail "'$*' ended with $got, not $want"
}

# wait_for <seconds> <command...>: until the command succeeds.
wait_for() {
    local deadline=$((SECONDS + $1))
    shift
    until "$@"; do
        [ "$SECONDS" -lt "$deadline" ] || return 1
        sleep 1
    done
}

VERSION=$(sed -n 's/^VERSION="\(.*\)"$/\1/p' /dist/install.sh)
export FIBER_RELEASE_URL=http://127.0.0.1:8000
(cd /dist && exec python3 -m http.server 8000 --bind 127.0.0.1 > /dev/null 2>&1) &
wait_for 10 curl -fs -o /dev/null http://127.0.0.1:8000/install.sh || fail "the web server did not start"
wait_for 60 systemctl is-system-running --quiet --wait || true

say "1. Install and apply, with a seed"
mkdir -p /boot/firmware
cat > /boot/firmware/fiber-seed.yaml << 'SEED'
schema: 1
components: {mosquitto: true, chirpstack: false}
login: {user: pi, password: "a secret"}
SEED
FIBER_NO_REBOOT=1 bash /dist/install.sh | tee /tmp/install.out
[ "$(fiber version)" = "$VERSION" ] || fail "fiber version is not $VERSION"
grep -q '^mosquitto: up, ' /tmp/install.out || fail "the install did not bring Mosquitto up"
grep -q '^chirpstack: off' /tmp/install.out || fail "the install did not turn ChirpStack off"
systemctl is-active --quiet mosquitto || fail "Mosquitto does not run"
systemctl is-enabled --quiet mosquitto || fail "Mosquitto does not start with the FIBER"
ss -ltnH | grep -q '127.0.0.1:1883 ' || fail "Mosquitto does not listen on 127.0.0.1:1883"
! ss -ltnH | grep -q '0.0.0.0:1883 ' || fail "Mosquitto listens on the network"
mosquitto_sub -h 127.0.0.1 -t fiber/test -C 1 -W 5 > /tmp/sub.out &
sub=$!
sleep 1
mosquitto_pub -h 127.0.0.1 -t fiber/test -m hello
wait "$sub" || true
[ "$(cat /tmp/sub.out)" = hello ] || fail "a message did not go through Mosquitto"
[ ! -e /var/lib/fiber/platform/pending ] || fail "pending is still there after a whole apply"
fiber platform status | tee /tmp/status.out
grep -q '^mosquitto   on          ok, ' /tmp/status.out || fail "status does not show Mosquitto ok"
[ -f /boot/firmware/fiber-platform-status.json ] || fail "the status was not copied to the boot partition"
[ -f /boot/firmware/fiber-platform.log ] || fail "the log was not copied to the boot partition"

say "2. Apply again: nothing changes"
fiber platform apply | tee /tmp/apply.out
grep -q '^mosquitto: up, nothing changed$' /tmp/apply.out || fail "the second apply changed something"
grep -q '^common: up, nothing changed$' /tmp/apply.out || fail "the second apply changed something in common"

say "3. Apply with the network taken away"
ip link set eth0 down
expect 0 fiber platform apply
ip link set eth0 up

say "4. A role made to fail: the others end ok, and the service tries again"
echo mosquitto > /run/fiber/platform/fail-on-purpose
expect 20 fiber platform apply
fiber platform status > /tmp/status.out || true
grep -q '^mosquitto   on          failed, .*fails on purpose' /tmp/status.out || fail "status does not show the failure: $(cat /tmp/status.out)"
grep -q '^chirpstack  off         ok, ' /tmp/status.out || fail "the other component did not end ok"
[ -e /var/lib/fiber/platform/pending ] || fail "pending is gone after a partly apply"
rm /run/fiber/platform/fail-on-purpose
wait_for 120 test ! -e /var/lib/fiber/platform/pending || fail "the service did not try again"
expect 0 fiber platform status

say "4b. The installer ends with 20 when the apply is partly done"
echo mosquitto > /run/fiber/platform/fail-on-purpose
expect 20 env FIBER_NO_REBOOT=1 bash /dist/install.sh
rm /run/fiber/platform/fail-on-purpose
wait_for 120 test ! -e /var/lib/fiber/platform/pending || fail "the service did not try again"

say "5. A restart needed: the installer ends with 10"
touch /run/fiber/platform/restart-needed
expect 10 env FIBER_NO_REBOOT=1 bash /dist/install.sh
rm /run/fiber/platform/restart-needed

say "Turning Mosquitto off stops it, and keeps its settings"
fiber platform disable mosquitto
expect 0 fiber platform apply
! systemctl is-active --quiet mosquitto || fail "Mosquitto still runs"
! systemctl is-enabled --quiet mosquitto || fail "Mosquitto still starts with the FIBER"
[ -f /etc/mosquitto/conf.d/fiber.conf ] || fail "its settings are gone"

say "Every converge test passed"
