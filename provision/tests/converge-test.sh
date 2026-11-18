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
#      nothing; only the expected ports listen; then the agent through Mosquitto against a
#      simulated south bridge, with the board's commands, and its last will
#   9. ChirpStack: at no moment does its API answer to admin/admin; the seed's login works; Redis
#      on loopback; the API key kept; the concentrator daemon and its udev rule there; a second
#      apply changes nothing
#  10. the agent's service with the base, against the simulated south bridge: it runs as the
#      user fiber without capabilities, holds the port, feeds its watchdog, and a second apply
#      changes nothing; then the footprint: every path the platform added is a package's, its
#      own, or listed
#  11. an update to a test release, a rollback, and an update to the current release; the agent
#      is restarted on each new program; turning it off takes fiber/status offline
#  12. an uninstall leaves only what packages own, and data
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

# Every path on the system but those that change on their own, sorted (the footprint test).
snapshot() {
    find / -xdev \( -path /proc -o -path /sys -o -path /dev -o -path /run -o -path /tmp -o -path /var/tmp \
        -o -path /var/cache -o -path /var/lib/apt/lists -o -path /var/log -o -path /dist -o -path /tests \
        -o -path /srv -o -path /root -o -path /boot/firmware \) -prune -o -print | sort
}

VERSION=$(sed -n 's/^VERSION="\(.*\)"$/\1/p' /dist/install.sh)
export FIBER_RELEASE_URL=http://127.0.0.1:8000
(cd /dist && exec python3 -m http.server 8000 --bind 127.0.0.1 > /dev/null 2>&1) &
wait_for 10 curl -fs -o /dev/null http://127.0.0.1:8000/install.sh || fail "the web server did not start"
wait_for 60 systemctl is-system-running --quiet --wait || true
# In /root, which the snapshot leaves out: systemd mounts a new /tmp while it starts.
snapshot > /root/before.txt

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

say "8d. The agent, through Mosquitto, against a simulated south bridge on a pseudo-terminal"
socat pty,raw,echo=0,link=/tmp/south-sim pty,raw,echo=0,link=/tmp/south-agent &
socat_pid=$!
wait_for 10 test -e /tmp/south-agent || fail "socat made no pseudo-terminals"
fiber south-simulator /tmp/south-sim &
simulator_pid=$!
FIBER_SOUTH_PORT=/tmp/south-agent fiber agent > /tmp/agent.log 2>&1 &
agent_pid=$!
agent_answers() { fiber status > /tmp/status.out 2>&1 && grep -q '^South       running' /tmp/status.out; }
wait_for 30 agent_answers || fail "fiber status: $(cat /tmp/status.out); the agent: $(cat /tmp/agent.log)"
cat /tmp/status.out
grep -qE '^South       running 0\.1\.0, image 0x[0-9a-f]{8} \(the one carried\)$' /tmp/status.out ||
    fail "fiber status does not show the south bridge running the image the agent carries"
grep -qE '^Power       PoE, input 1[12]\.[0-9]{2} V, battery 4\.[01][0-9] V$' /tmp/status.out || fail "fiber status does not show the power"
grep -q '^Ports       1 2 3 4 5 6 7 8 on$' /tmp/status.out || fail "fiber status does not show the ports"
[ "$(fiber ports off 3)" = "Ports       1 2 4 5 6 7 8 on, 3 off" ] || fail "fiber ports off 3"
mosquitto_sub -h 127.0.0.1 -t fiber/ports/3 -C 1 -W 5 | grep -q '"power":false' || fail "fiber/ports/3 does not say off"
[ "$(fiber ports on)" = "Ports       1 2 3 4 5 6 7 8 on" ] || fail "fiber ports on"
fiber leds amber port2 --blink 500 500
fiber leds | grep -q '^port2       set$' || fail "fiber leds does not show port2 set"
fiber leds auto
expect 1 fiber leds yellow port2
fiber firmware | grep -q '^South       running' || fail "fiber firmware"
kill "$agent_pid"
agent_gone() {
    local code=0
    fiber status > /dev/null 2>&1 || code=$?
    [ "$code" -eq 4 ]
}
wait_for 40 agent_gone || fail "the last will did not come"
expect 4 fiber status
kill "$simulator_pid" "$socat_pid" 2> /dev/null || true

say "9. ChirpStack: at no moment does its API answer to admin/admin"
login() { python3 /tests/chirpstack-login.py "$@" 2> /dev/null || echo "no answer"; }
rm -f /tmp/default-login-answered /tmp/watch-stop
(
    while [ ! -e /tmp/watch-stop ]; do
        [ "$(login admin admin)" = ok ] && touch /tmp/default-login-answered
        sleep 0.2
    done
) &
watcher=$!
fiber platform enable chirpstack
expect 0 fiber platform apply
seed_login_works() { [ "$(login admin "a secret")" = ok ]; }
wait_for 60 seed_login_works || fail "ChirpStack does not take the login from the seed"
touch /tmp/watch-stop
wait "$watcher" || true
[ ! -e /tmp/default-login-answered ] || fail "ChirpStack answered to admin/admin"
[ "$(login admin admin)" != ok ] || fail "ChirpStack takes admin/admin"
systemctl is-active --quiet chirpstack-sqlite || fail "ChirpStack does not run"
systemctl is-enabled --quiet chirpstack-sqlite || fail "ChirpStack does not start with the FIBER"
systemctl is-active --quiet redis-server || fail "Redis does not run"
! listening | grep -qE '^(0\.0\.0\.0|\*|\[::\]):6379$' || fail "Redis listens on the network"
[ "$(stat -c %a /etc/fiber/secrets/chirpstack-api-key)" = 600 ] || fail "the API key is not root only"
grep -q '^eyJ' /etc/fiber/secrets/chirpstack-api-key || fail "the API key is not a token"
[ ! -e /run/fiber/platform/chirpstack-password ] || fail "the password file was left behind"
[ -x /opt/fiber/concentratord/4.7.1/chirpstack-concentratord-sx1302 ] || fail "the concentrator daemon is not there"
/opt/fiber/concentratord/4.7.1/chirpstack-concentratord-sx1302 --version || fail "the concentrator daemon does not run here"
[ -f /etc/udev/rules.d/99-fiber-rak5146.rules ] || fail "the udev rule for the card is not there"
! systemctl is-enabled --quiet chirpstack-mqtt-forwarder || fail "the forwarder starts without the card"

say "9b. Apply again: nothing changes"
fiber platform apply | tee /tmp/apply.out
grep -q '^chirpstack: up, nothing changed$' /tmp/apply.out || fail "the second apply changed ChirpStack"

say "10. The agent's service, with the base, against a simulated south bridge"
mount -o loop /tmp/boot.img /boot/firmware
# The simulated south bridge on a pseudo-terminal, and a drop-in in /run that points the service
# at it and allows it (a pseudo-terminal is no device class the unit allows). A watchdog of 3 s,
# so that a loop that does not feed it shows.
mkdir -p /run/fiber-test
socat pty,raw,echo=0,link=/run/fiber-test/south-sim pty,raw,echo=0,link=/run/fiber-test/south-agent &
socat_pid=$!
wait_for 10 test -e /run/fiber-test/south-agent || fail "socat made no pseudo-terminals"
fiber south-simulator /run/fiber-test/south-sim &
simulator_pid=$!
mkdir -p /run/systemd/system/fiber-agent.service.d
cat > /run/systemd/system/fiber-agent.service.d/fiber-test.conf << 'UNIT'
[Service]
Environment=FIBER_SOUTH_PORT=/run/fiber-test/south-agent
DeviceAllow=char-pts rw
WatchdogSec=3s
UNIT
systemctl daemon-reload
fiber platform enable agent
expect 10 fiber platform apply
rm /run/fiber/platform/restart-needed
systemctl is-enabled --quiet fiber-agent || fail "the agent does not start with the FIBER"
# The user fiber is there now: the pseudo-terminal becomes its.
chown fiber "$(readlink -f /run/fiber-test/south-agent)"
wait_for 30 agent_answers || fail "fiber status: $(cat /tmp/status.out); the agent: $(journalctl -u fiber-agent --no-pager | tail -n 20)"
cat /tmp/status.out
agent_pid=$(systemctl show -p MainPID --value fiber-agent)
[ "$(stat -c %U "/proc/$agent_pid")" = fiber ] || fail "the agent does not run as the user fiber"
grep -q '^CapEff:[[:space:]]*0000000000000000$' "/proc/$agent_pid/status" || fail "the agent has capabilities"
[ -f /etc/fiber/agent.toml ] || fail "the agent's settings file is not there"
fiber firmware chip south 2> /tmp/chip.err && fail "fiber firmware chip ran beside the agent"
grep -q 'is in use by another program' /tmp/chip.err || fail "fiber firmware chip does not see the agent's lock: $(cat /tmp/chip.err)"
sleep 8
[ "$(systemctl show -p MainPID --value fiber-agent)" = "$agent_pid" ] || fail "the agent was restarted (by its watchdog?)"
fiber platform apply | tee /tmp/apply.out
grep -q '^agent: up, nothing changed$' /tmp/apply.out || fail "the second apply changed the agent"

say "10b. The footprint: every path the platform added is a package's, its own, or listed"
snapshot > /root/after.txt
python3 /tests/footprint-check.py /root/before.txt /root/after.txt /opt/fiber/current/provision/footprint.yaml ${FOOTPRINT_REPORT:+--report} ||
    fail "the platform wrote paths that the footprint does not list"

say "11. An update to a test release, and a rollback"
export FIBER_RELEASES_URL=http://127.0.0.1:8000/test/releases
FIBER_NO_REBOOT=1 fiber platform update | tee /tmp/update.out
[ "$(readlink /opt/fiber/current)" = "releases/$VERSION-update-test" ] || fail "the update did not make the test release current"
[ "$(readlink /opt/fiber/previous)" = "releases/$VERSION" ] || fail "the release before is not previous"
grep -q "^Updating from release $VERSION to $VERSION-update-test" /tmp/update.out || fail "the update did not say what it does"
fiber platform status | grep -q "^Release  $VERSION-update-test, " || fail "status does not show the test release"
agent_program() { readlink "/proc/$(systemctl show -p MainPID --value fiber-agent)/exe"; }
[ "$(agent_program)" = "/opt/fiber/releases/$VERSION-update-test/bin/fiber" ] ||
    fail "the agent does not run the new program, but $(agent_program)"
FIBER_NO_REBOOT=1 fiber platform rollback | tee /tmp/rollback.out
[ "$(readlink /opt/fiber/current)" = "releases/$VERSION" ] || fail "the rollback did not go back"
[ "$(readlink /opt/fiber/previous)" = "releases/$VERSION-update-test" ] || fail "the rollback did not keep the other release"
grep -q '^Done' /tmp/rollback.out || fail "the rollback did not apply"
[ "$(agent_program)" = "/opt/fiber/releases/$VERSION/bin/fiber" ] ||
    fail "the agent does not run the program it was rolled back to, but $(agent_program)"
fiber platform update --to "$VERSION" | grep -q "is the current one already" || fail "an update to the current release did something"
unset FIBER_RELEASES_URL

say "Turning the agent and the base off: the agent stops, and its last will says so"
fiber platform disable agent base
expect 10 fiber platform apply
rm /run/fiber/platform/restart-needed
! systemctl is-active --quiet fiber-agent || fail "the agent still runs"
! systemctl is-enabled --quiet fiber-agent || fail "the agent still starts with the FIBER"
wait_for 10 agent_gone || fail "fiber/status does not say the agent is offline"
[ -f /etc/fiber/agent.toml ] || fail "the agent's settings are gone"
umount /boot/firmware
kill "$simulator_pid" "$socat_pid" 2> /dev/null || true
rm -r /run/systemd/system/fiber-agent.service.d /run/fiber-test
systemctl daemon-reload

say "Turning ChirpStack, Node-RED and Mosquitto off stops them, and keeps their settings"
fiber platform disable chirpstack node-red mosquitto
expect 0 fiber platform apply
! systemctl is-active --quiet chirpstack-sqlite || fail "ChirpStack still runs"
! systemctl is-active --quiet redis-server || fail "Redis still runs"
expect 0 fiber platform apply
! systemctl is-active --quiet mosquitto || fail "Mosquitto still runs"
! systemctl is-enabled --quiet mosquitto || fail "Mosquitto still starts with the FIBER"
! systemctl is-active --quiet nodered || fail "Node-RED still runs"
[ -f /etc/mosquitto/conf.d/fiber.conf ] || fail "Mosquitto's settings are gone"
[ -f /var/lib/node-red/settings.js ] || fail "Node-RED's settings are gone"

say "12. An uninstall leaves only what packages own, and data"
expect 1 fiber platform uninstall
fiber platform uninstall --yes | tee /tmp/uninstall.out
for gone in /opt/fiber /etc/fiber /var/lib/fiber /usr/local/bin/fiber /etc/systemd/system/fiber-platform-apply.service \
    /etc/systemd/system/fiber-agent.service; do
    [ ! -e "$gone" ] || fail "$gone is still there"
done
grep -q '^  sudo apt purge ' /tmp/uninstall.out || fail "the uninstall did not print how to purge the packages"
grep -q '^  sudo userdel fiber$' /tmp/uninstall.out || fail "the uninstall did not say how to take the user fiber off"
snapshot > /root/uninstalled.txt
python3 /tests/footprint-check.py /root/before.txt /root/uninstalled.txt /footprint.yaml --uninstalled ||
    fail "the uninstall left paths behind"

say "Every converge test passed"
