#!/bin/bash
# The installer's tests, in a container like Raspberry Pi OS (`just check-install`). The release
# to install is in /dist (`just release`), served from a local web server. Runs as root.
#
#   1. install from the server, piped into bash as FIBER Setup does, with a seed
#   2. run it again: nothing changes
#   3. stop it hard in the middle of unpacking, then run it again
#   4. a wrong checksum is refused
#   and: an installer that is not stamped is refused, and so is one not run as root
set -euo pipefail

say() { printf '\n### %s\n' "$*"; }
fail() {
    printf 'FAILED: %s\n' "$*"
    exit 1
}

# expect <status> <command...>: the command must end with that status.
expect() {
    local want=$1 got=0
    shift
    "$@" || got=$?
    [ "$got" -eq "$want" ] || fail "'$*' ended with $got, not $want"
}

VERSION=$(sed -n 's/^VERSION="\(.*\)"$/\1/p' /dist/install.sh)
export FIBER_RELEASE_URL=http://127.0.0.1:8000 FIBER_NO_APPLY=1

# The web server: /dist as it is, and /bad with an archive that has one byte more.
mkdir -p /srv/bad
# The release's files only (dist/ also holds the update test's releases, in dist/test/).
find /dist -maxdepth 1 -type f -exec cp {} /srv/ \;
find /dist -maxdepth 1 -type f -exec cp {} /srv/bad/ \;
printf 'x' >> "/srv/bad/fiber-platform-$VERSION-linux-arm64.tar.gz"
(cd /srv && exec python3 -m http.server 8000 --bind 127.0.0.1 > /dev/null 2>&1) &
for _ in $(seq 50); do curl -fs -o /dev/null http://127.0.0.1:8000/install.sh && break; sleep 0.1; done

# A listing of everything the installer writes, with modes, sizes, times and link targets.
snapshot() {
    find /opt/fiber /usr/local/bin/fiber /etc/fiber -printf '%p %y %m %s %T@ %l\n' 2> /dev/null | sort
}

say "An installer that is not stamped is refused"
sed -e 's/^VERSION=".*"$/VERSION="@VERSION@"/' /dist/install.sh > /tmp/unstamped.sh
expect 1 bash /tmp/unstamped.sh

say "Only root may install"
expect 1 setpriv --reuid=65534 --regid=65534 --clear-groups bash /dist/install.sh

say "1. Install from the server, piped into bash, with a seed"
mkdir -p /boot/firmware
cat > /boot/firmware/fiber-seed.yaml << 'SEED'
schema: 1
components: {base: true, mosquitto: true, node-red: true}
settings: {mqtt: {lan: true}}
login: {user: pi, password: "a secret"}
SEED
curl -fsSL http://127.0.0.1:8000/install.sh | FIBER_NO_REBOOT=1 bash
[ "$(readlink /opt/fiber/current)" = "releases/$VERSION" ] || fail "current is not releases/$VERSION"
[ -f "/opt/fiber/releases/$VERSION/.complete" ] || fail "the release has no .complete"
[ "$(fiber version)" = "$VERSION" ] || fail "fiber version is not $VERSION"
grep -q '^  node-red: true$' /etc/fiber/platform.yaml || fail "the seed's components are not in platform.yaml"
grep -q '^    lan: true ' /etc/fiber/platform.yaml || fail "the seed's settings are not in platform.yaml"
[ "$(stat -c %a /etc/fiber/secrets/login.json)" = 600 ] || fail "login.json is not root only"
[ ! -e /boot/firmware/fiber-seed.yaml ] || fail "the seed is still on the boot partition"
command -v ansible-playbook > /dev/null || fail "Ansible is not installed"
for package in python3-apt python3-debian; do
    dpkg-query -W -f='${Status}' "$package" | grep -q 'ok installed' || fail "$package is not installed"
done
! dpkg-query -W -f='${Status}' ansible 2> /dev/null | grep -q 'ok installed' || fail "the ansible bundle came in"
# Its status ends with 3 (nothing is applied yet), so it is read first and searched after.
status=$(fiber platform status || true)
grep -q "^Release  $VERSION, in /opt/fiber/releases/$VERSION$" <<< "$status" || fail "fiber does not find its release: $status"
[ ! -e /opt/fiber/downloads/fiber-platform-"$VERSION"-linux-arm64.tar.gz ] || fail "the download was left behind"

say "2. Run it again: nothing changes"
before=$(snapshot)
bash /dist/install.sh
[ "$(snapshot)" = "$before" ] || fail "the second run changed something: $(diff <(echo "$before") <(snapshot))"

say "3. Stop it hard in the middle of unpacking, then run it again"
rm -rf /opt/fiber /usr/local/bin/fiber /etc/fiber
expect 137 env FIBER_TEST_STOP_AT=unpacked bash /dist/install.sh
[ -d "/opt/fiber/releases/$VERSION.partial" ] || fail "it did not stop while unpacking"
[ ! -e /opt/fiber/current ] || fail "current was set before the release was whole"
bash /dist/install.sh --components node-red
[ ! -e "/opt/fiber/releases/$VERSION.partial" ] || fail "the half unpacked release was left behind"
[ "$(readlink /opt/fiber/current)" = "releases/$VERSION" ] || fail "current is not releases/$VERSION"
grep -q '^  mosquitto: true$' /etc/fiber/platform.yaml || fail "--components did not turn on what node-red requires"

say "4. A wrong checksum is refused"
rm -rf /opt/fiber /usr/local/bin/fiber
expect 1 env FIBER_RELEASE_URL=http://127.0.0.1:8000/bad bash /dist/install.sh
[ ! -e "/opt/fiber/releases/$VERSION" ] && [ ! -e "/opt/fiber/releases/$VERSION.partial" ] || fail "a wrong archive was unpacked"
[ -z "$(ls -A /opt/fiber/downloads)" ] || fail "the wrong archive was kept"

say "Every installer test passed"
