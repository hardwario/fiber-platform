#!/usr/bin/env bash
# The ready-made image's first start without the network (docs/provisioning.md, "The ready-made
# image"): run inside the image's root, started as a container with systemd and no network
# (`just check-image`). What FIBER Setup does on a FIBER is done here by hand: the seed on the
# boot partition, then the image's own installer.
set -euo pipefail

fail() {
    echo "FAILED: $*"
    exit 1
}
say() { printf '\n### %s\n' "$*"; }

say "0. The container is up, with systemd, without a network"
[ "$(cat /proc/1/comm)" = systemd ] || fail "process 1 is $(cat /proc/1/comm), not systemd"
# Its bus first, then the end of its start. Degraded is fine: Raspberry Pi OS's units for the
# hardware fail in a container. Started too early, a file written to /tmp vanishes under the tmpfs
# systemd mounts there.
for _ in $(seq 1 60); do
    if systemctl show -p Version > /dev/null 2>&1; then break; fi
    sleep 1
done
state=$(timeout 300 systemctl is-system-running --wait || true)
case "$state" in
    running | degraded) echo "systemd: $state" ;;
    *) fail "systemd is $state: $(systemctl list-jobs --no-legend | head -5)" ;;
esac
if ip route 2> /dev/null | grep -q default; then
    fail "there is a default route: this test must run without a network"
fi

say "1. Nothing runs that the seed does not turn on, and nothing of one FIBER's is there"
[ -f /etc/fiber/image.json ] || fail "no /etc/fiber/image.json"
grep -q '"release"' /etc/fiber/image.json || fail "the mark has no release: $(cat /etc/fiber/image.json)"
[ ! -e /etc/fiber/platform.yaml ] || fail "the image has a platform.yaml"
[ -z "$(ls -A /etc/fiber/secrets 2> /dev/null)" ] || fail "the image has secrets: $(ls /etc/fiber/secrets)"
[ ! -e /var/lib/node-red/settings.js ] || fail "the image has Node-RED's settings.js"
for unit in mosquitto nodered fiber-agent fiber-restart.path chirpstack-sqlite chirpstack-mqtt-forwarder \
    redis-server; do
    state=$(systemctl is-enabled "$unit" 2> /dev/null || true)
    [ "$state" != enabled ] || fail "$unit is enabled in the image"
    ! systemctl is-active -q "$unit" || fail "$unit runs before the seed turned it on"
done
# No first boot (an empty machine id, so no presets), but the two first-boot jobs ran all the same
# (build.sh's drop-ins): the SSH host keys made, the root file system's growth started.
ls /etc/ssh/ssh_host_*_key > /dev/null 2>&1 || fail "no SSH host keys: SSH would refuse"
[ "$(systemctl show -p ConditionResult --value rpi-resize)" = yes ] || fail "rpi-resize did not run"
grep -q '^# BEGIN FIBER PLATFORM' /boot/firmware/config.txt || fail "config.txt has no FIBER block"
[ -x /opt/fiber/install.sh ] || fail "no /opt/fiber/install.sh"

say "2. The first start: the seed, then the image's installer, without a network"
cat > /boot/firmware/fiber-seed.yaml << 'EOF'
schema: 1
components: {base: true, mosquitto: true, agent: true, node-red: true, chirpstack: true}
settings: {mqtt: {lan: true}}
login: {user: pi, password: "a secret password"}
EOF
status=0
FIBER_NO_REBOOT=1 bash /opt/fiber/install.sh > /tmp/install.out 2>&1 || status=$?
cat /tmp/install.out
[ "$status" -eq 0 ] || fail "the installer ended with $status (0 expected: the boot lines are in place)"
grep -q "It is installed already" /tmp/install.out || fail "the installer installed Ansible again"
[ ! -e /boot/firmware/fiber-seed.yaml ] || fail "the seed is still there"

say "3. Every component up"
for unit in mosquitto nodered fiber-agent chirpstack-sqlite redis-server; do
    systemctl is-active -q "$unit" || fail "$unit does not run: $(systemctl status "$unit" --no-pager | tail -5)"
done
fiber platform status > /tmp/status.out || fail "fiber platform status: $(cat /tmp/status.out)"
# The login from the seed opens Node-RED, and a wrong one does not.
login() {
    curl -fs -o /dev/null -d client_id=node-red-admin -d grant_type=password -d scope='*' \
        -d username=pi -d "password=$1" http://127.0.0.1:1880/auth/token
}
for _ in $(seq 1 60); do
    if login "a secret password"; then break; fi
    sleep 1
done
login "a secret password" || fail "Node-RED does not take the login from the seed"
! login "fiber" || fail "Node-RED takes a password that is not the seed's"
# Mosquitto's listener for the network asks for the login.
mosquitto_pub -h 127.0.0.1 -p 1884 -u pi -P "a secret password" -t fiber/test -m x ||
    fail "Mosquitto's listener for the network does not take the login"
! mosquitto_pub -h 127.0.0.1 -p 1884 -t fiber/test -m x 2> /dev/null ||
    fail "Mosquitto's listener for the network takes no login"
# ChirpStack's secret is this FIBER's own, made now.
[ -s /etc/fiber/secrets/chirpstack-api-secret ] || fail "no ChirpStack API secret"
[ -s /etc/fiber/secrets/chirpstack-api-key ] || fail "no ChirpStack API key"

say "The image's first start: passed"
