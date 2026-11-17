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
#   6. the base role on a FAT boot partition: the stock config.txt becomes the expected file,
#      a second apply changes nothing, a file edited by hand gets its block back, fiber doctor
#      names a bad line, and turning the base off takes the block away
#   7. Mosquitto's listener for the network with the seed's login; a changed password stays
#   8. Node-RED behind the seed's login, with the flow to start from; a second apply changes
#      nothing; only the expected ports listen
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
# The link going down takes the default route with it; it is put back afterwards.
default_route=$(ip route show default | head -n 1)
ip link set eth0 down
expect 0 fiber platform apply
ip link set eth0 up
# shellcheck disable=SC2086 # the route is words for ip
ip route replace $default_route

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

say "6. The base role, on a FAT boot partition: the stock config.txt gets the block"
truncate -s 64M /tmp/boot.img
mkfs.vfat -n bootfs /tmp/boot.img > /dev/null
mount -o loop /tmp/boot.img /boot/firmware
mkdir /boot/firmware/overlays
cp /tests/fixtures/config.txt.stock /boot/firmware/config.txt
fiber platform enable base
expect 10 fiber platform apply
cmp /boot/firmware/config.txt /tests/fixtures/config.txt.expected || fail "config.txt is not the expected file"
cmp /boot/firmware/config.txt.fiber-original /tests/fixtures/config.txt.stock || fail "the copy of the original is not the stock file"
[ -s /boot/firmware/overlays/fiber-ds2482.dtbo ] || fail "the overlay is not in /boot/firmware/overlays"
grep -qx i2c-dev /etc/modules-load.d/fiber.conf || fail "i2c-dev is not loaded at the start"
getent group gpio i2c spi > /dev/null || fail "the groups are missing"
rm /run/fiber/platform/restart-needed

say "6b. Apply again: nothing changes, no restart needed"
fiber platform apply | tee /tmp/apply.out
grep -q '^base: up, nothing changed$' /tmp/apply.out || fail "the second apply changed the base"

say "6c. A config.txt edited by hand gets its block back; fiber doctor names the bad line"
sed -i '/^dtoverlay=uart4$/d' /boot/firmware/config.txt
echo 'dtparam=spi=on' >> /boot/firmware/config.txt
expect 10 fiber platform apply
cat /tests/fixtures/config.txt.expected > /tmp/expected
echo 'dtparam=spi=on' >> /tmp/expected
cmp /boot/firmware/config.txt /tmp/expected || fail "the block did not come back as it was"
rm /run/fiber/platform/restart-needed
expect 1 fiber doctor > /tmp/doctor.out
# shellcheck disable=SC2016 # the backticks are in the text, not a command
grep -q '^FAIL  config.txt, line [0-9]*: `dtparam=spi=on` turns on SPI0' /tmp/doctor.out || fail "fiber doctor did not name the line: $(cat /tmp/doctor.out)"
grep -q '^ok    config.txt has the FIBER block$' /tmp/doctor.out || fail "fiber doctor did not see the block"

say "6d. Turning the base off takes the block away"
fiber platform disable base
expect 10 fiber platform apply
cat /tests/fixtures/config.txt.stock > /tmp/expected
echo 'dtparam=spi=on' >> /tmp/expected
cmp /boot/firmware/config.txt /tmp/expected || fail "the block is still in config.txt"
[ ! -e /boot/firmware/overlays/fiber-ds2482.dtbo ] || fail "the overlay is still there"
rm /run/fiber/platform/restart-needed
umount /boot/firmware

say "7. Mosquitto's listener for the network, with the login from the seed"
sed -i 's/^    lan: false/    lan: true /' /etc/fiber/platform.yaml
expect 0 fiber platform apply
listening() { ss -ltnH | awk '{print $4}'; }
listening | grep -qE '^(0\.0\.0\.0|\*|\[::\]):1884$' || fail "Mosquitto does not listen on 1884 for the network: $(listening)"
listening | grep -qx '127.0.0.1:1883' || fail "Mosquitto does not listen on 127.0.0.1:1883"
! listening | grep -qE '^(0\.0\.0\.0|\*|\[::\]):1883$' || fail "port 1883 is open to the network"
[ "$(stat -c '%U %a' /etc/mosquitto/fiber-passwd)" = "mosquitto 600" ] || fail "the password file is not Mosquitto's alone"
! mosquitto_pub -h 127.0.0.1 -p 1884 -t fiber/test -m x 2> /dev/null || fail "port 1884 takes a message without a login"
mosquitto_pub -h 127.0.0.1 -p 1884 -u pi -P "a secret" -t fiber/test -m x || fail "port 1884 refuses the login from the seed"

say "7b. A password the owner changes stays changed"
mosquitto_passwd -b /etc/mosquitto/fiber-passwd pi "another secret"
systemctl restart mosquitto
expect 0 fiber platform apply
mosquitto_pub -h 127.0.0.1 -p 1884 -u pi -P "another secret" -t fiber/test -m x || fail "the changed password was set back"
! mosquitto_pub -h 127.0.0.1 -p 1884 -u pi -P "a secret" -t fiber/test -m x 2> /dev/null || fail "the old password works again"

say "8. Node-RED: its editor asks for the login, and behind it is the flow to start from"
fiber platform enable node-red
expect 0 fiber platform apply
systemctl is-active --quiet nodered || fail "Node-RED does not run"
wait_for 60 curl -fs -o /dev/null http://127.0.0.1:1880/auth/login || fail "Node-RED does not answer on port 1880"
curl -fs http://127.0.0.1:1880/auth/login | grep -q '"type":"credentials"' || fail "Node-RED's editor does not ask for a login"
[ "$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:1880/flows)" = 401 ] || fail "the flows can be read without the login"
token=$(curl -fs -d client_id=node-red-admin -d grant_type=password -d scope='*' -d username=pi \
    --data-urlencode 'password=a secret' http://127.0.0.1:1880/auth/token |
    python3 -c 'import json, sys; print(json.load(sys.stdin)["access_token"])') || fail "Node-RED refuses the login from the seed"
curl -fs -H "Authorization: Bearer $token" http://127.0.0.1:1880/flows | grep -q '"fiber-in"' || fail "the flow to start from is not there"
[ "$(stat -c '%U %a' /var/lib/node-red/settings.js)" = "nodered 640" ] || fail "settings.js is readable by others"

say "8b. Apply again: nothing changes"
fiber platform apply | tee /tmp/apply.out
grep -q '^node-red: up, nothing changed$' /tmp/apply.out || fail "the second apply changed Node-RED"
grep -q '^mosquitto: up, nothing changed$' /tmp/apply.out || fail "the second apply changed Mosquitto"

say "8c. Only the expected ports listen: 1880 (Node-RED), 1883 (loopback), 1884 (with a login)"
ports=$(listening | sed 's/.*://' | sort -un | grep -vx 8000 | tr '\n' ' ')
[ "$ports" = "1880 1883 1884 " ] || fail "these ports listen: $ports"

say "Turning Node-RED and Mosquitto off stops them, and keeps their settings"
fiber platform disable node-red mosquitto
expect 0 fiber platform apply
! systemctl is-active --quiet mosquitto || fail "Mosquitto still runs"
! systemctl is-enabled --quiet mosquitto || fail "Mosquitto still starts with the FIBER"
! systemctl is-active --quiet nodered || fail "Node-RED still runs"
[ -f /etc/mosquitto/conf.d/fiber.conf ] || fail "Mosquitto's settings are gone"
[ -f /var/lib/node-red/settings.js ] || fail "Node-RED's settings are gone"

say "Every converge test passed"
