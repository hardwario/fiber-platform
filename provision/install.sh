#!/bin/bash
# FIBER Platform installer: puts one release of the platform onto a FIBER and applies it.
#
#   curl -fsSL https://github.com/hardwario/fiber-platform/releases/download/v<version>/install.sh | sudo bash
#
# What it does (docs/provisioning.md, "The installer"):
#   1. checks that it runs as root on an arm64 Raspberry Pi OS trixie;
#   2. downloads the release archive and checks it against the sha256 written into this script;
#   3. unpacks it beside the release before it, under /opt/fiber/releases, checks every file, and
#      points /opt/fiber/current and /usr/local/bin/fiber at it;
#   4. installs Ansible from Debian;
#   5. takes the choice from FIBER Setup's seed file, or from --components;
#   6. applies it: fiber platform apply.
#
# The sha256 proves that the archive is the one this installer was made for. It does not prove who
# made the installer: take it from the release page, or let FIBER Setup fetch it, which checks it.
#
# Options:
#   --components a,b,c   turn these components on (when there is no seed file)
#
# Settings (environment):
#   FIBER_NO_REBOOT=1    do not restart at the end, even when the apply asks for it (exit status 10)
#   FIBER_NO_APPLY=1     install the release, but do not apply it
#   FIBER_RELEASE_URL    where the archive is downloaded from (default: this release on GitHub)
#   FIBER_ROOT           a directory to install into instead of / (for the tests)
#
# Exit status: 0 done; 10 done, a restart is needed; 20 partly done, the platform tries again by
# itself; anything else: failed, and the message says why.
#
# Every line of code is in a function, and only the last line calls them: a script piped into
# bash is read while it runs, so nothing may run before all of it has been read.

set -euo pipefail

# Written in when the release is built (provision/release.py).
VERSION="@VERSION@"
ARCHIVE_SHA256="@ARCHIVE_SHA256@"

say() {
    printf '%s\n' "$*"
}

step() {
    printf '== %s\n' "$*"
}

fail() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

usage() {
    say "Usage: install.sh [--components a,b,c]"
    say "Installs release ${VERSION} of the FIBER platform and applies it. Settings and exit"
    say "statuses: see the start of this script."
}

setup() {
    R="${FIBER_ROOT:-}"
    OPT="$R/opt/fiber"
    RELEASES="$OPT/releases"
    ARCHIVE="fiber-platform-${VERSION}-linux-arm64.tar.gz"
    URL="${FIBER_RELEASE_URL:-https://github.com/hardwario/fiber-platform/releases/download/v${VERSION}}"
    SEED="$R/boot/firmware/fiber-seed.yaml"
    FIBER="$OPT/current/bin/fiber"
    FIBER_ARGS=()
    if [ -n "$R" ]; then
        FIBER_ARGS=(--root "$R")
    fi
}

check_stamped() {
    case "$VERSION$ARCHIVE_SHA256" in
        *@*) fail "this installer is not stamped with a release. Take install.sh from a release of the FIBER platform" ;;
    esac
}

check_machine() {
    step "1 of 6: checking the machine"
    [ "$(id -u)" -eq 0 ] || fail "run it as root: sudo bash install.sh"
    local arch codename
    arch=$(dpkg --print-architecture 2>/dev/null || true)
    [ "$arch" = arm64 ] || fail "this is for 64-bit Raspberry Pi OS (arm64), and this machine is ${arch:-something else}"
    # In a subshell: os-release sets VERSION too.
    # shellcheck source=/dev/null
    codename=$(. /etc/os-release && printf '%s' "${VERSION_CODENAME:-}")
    [ "$codename" = trixie ] || fail "this is for Raspberry Pi OS trixie, and this system is ${codename:-something else}"
    [ -e /etc/rpi-issue ] || fail "this is not Raspberry Pi OS (there is no /etc/rpi-issue)"
    for tool in curl python3 sha256sum tar flock; do
        command -v "$tool" > /dev/null || fail "$tool is missing"
    done
}

# One installation at a time: FIBER Setup's retries and a person could overlap.
lock() {
    mkdir -p /run/lock
    exec 9> /run/lock/fiber-platform-install.lock
    flock -n 9 || fail "another installation is running"
}

fetch() {
    step "2 of 6: downloading release ${VERSION}"
    if [ -f "$RELEASES/$VERSION/.complete" ]; then
        say "It is unpacked already: $RELEASES/$VERSION"
        return
    fi
    mkdir -p "$OPT/downloads"
    DOWNLOAD="$OPT/downloads/$ARCHIVE"
    say "From $URL/$ARCHIVE"
    if ! curl -fsSL --retry 5 --retry-delay 5 --connect-timeout 30 -o "$DOWNLOAD.part" "$URL/$ARCHIVE"; then
        rm -f "$DOWNLOAD.part"
        fail "could not download $URL/$ARCHIVE"
    fi
    mv -f "$DOWNLOAD.part" "$DOWNLOAD"
    local got
    got=$(sha256sum "$DOWNLOAD" | cut -d ' ' -f 1)
    if [ "$got" != "$ARCHIVE_SHA256" ]; then
        rm -f "$DOWNLOAD"
        fail "the archive is not the one this installer was made for: its sha256 is $got, not $ARCHIVE_SHA256"
    fi
    say "Its sha256 is right: $got"
}

# Every file of the unpacked release against manifest.json: none missing, none more, none changed.
verify() {
    python3 - "$1" "$VERSION" << 'PYTHON'
import hashlib, json, os, sys

root, version = sys.argv[1], sys.argv[2]
with open(os.path.join(root, "manifest.json")) as f:
    manifest = json.load(f)
if manifest.get("schema") != 1 or manifest.get("version") != version:
    sys.exit(f"error: manifest.json is not for schema 1 and release {version}")
found = set()
for directory, _, names in os.walk(root):
    for name in names:
        found.add(os.path.relpath(os.path.join(directory, name), root))
found.discard("manifest.json")
listed = set(manifest["files"])
problems = [f"missing: {p}" for p in sorted(listed - found)] + [f"not listed: {p}" for p in sorted(found - listed)]
for path in sorted(listed & found):
    with open(os.path.join(root, path), "rb") as f:
        if hashlib.sha256(f.read()).hexdigest() != manifest["files"][path]:
            problems.append(f"changed: {path}")
if problems:
    sys.exit("error: the unpacked release does not match its manifest:\n  " + "\n  ".join(problems))
PYTHON
}

unpack() {
    step "3 of 6: unpacking"
    local dir="$RELEASES/$VERSION" part="$RELEASES/$VERSION.partial"
    if [ -f "$dir/.complete" ]; then
        say "Nothing to unpack."
    else
        rm -rf "$part" "$dir"
        mkdir -p "$part"
        tar -xzf "$DOWNLOAD" -C "$part"
        # For the tests: stop as hard as a power cut would, right after unpacking.
        if [ "${FIBER_TEST_STOP_AT:-}" = unpacked ]; then
            kill -9 $$
        fi
        verify "$part"
        sync
        mv "$part" "$dir"
        touch "$dir/.complete"
        sync
        rm -f "$DOWNLOAD"
        say "Unpacked into $dir"
    fi
    switch
}

# Points `current` at this release and `previous` at the one before, each by renaming a new link
# over the old one, so that there is always one of them. Keeps those two releases, no others.
switch() {
    local target="releases/$VERSION" before link want
    before=$(readlink "$OPT/current" 2>/dev/null || true)
    if [ "$before" != "$target" ]; then
        if [ -n "$before" ]; then
            ln -sfn "$before" "$OPT/.previous.new"
            mv -T "$OPT/.previous.new" "$OPT/previous"
        fi
        ln -sfn "$target" "$OPT/.current.new"
        mv -T "$OPT/.current.new" "$OPT/current"
        say "Current release: $VERSION${before:+ (previous: ${before#releases/})}"
    fi
    link="$R/usr/local/bin/fiber"
    want="../../../opt/fiber/current/bin/fiber"
    if [ "$(readlink "$link" 2>/dev/null || true)" != "$want" ]; then
        mkdir -p "${link%/*}"
        ln -sfn "$want" "$link.new"
        mv -T "$link.new" "$link"
    fi
    local keep_previous
    keep_previous=$(readlink "$OPT/previous" 2>/dev/null || true)
    for old in "$RELEASES"/*; do
        [ -e "$old" ] || continue
        case "releases/${old##*/}" in
            "$target" | "$keep_previous") ;;
            *)
                rm -rf "$old"
                say "Removed the old release ${old##*/}"
                ;;
        esac
    done
    sync
}

install_ansible() {
    step "4 of 6: Ansible"
    local package missing=0
    for package in ansible-core python3-apt python3-debian; do
        dpkg-query -W -f='${Status}' "$package" 2> /dev/null | grep -q 'ok installed' || missing=1
    done
    if [ "$missing" -eq 0 ]; then
        say "It is installed already: $(ansible-playbook --version | head -n 1)"
        return
    fi
    export DEBIAN_FRONTEND=noninteractive
    dpkg --configure -a
    apt-get -o DPkg::Lock::Timeout=600 update
    # Without --no-install-recommends apt also installs the `ansible` bundle of collections (205 MB).
    # python3-apt is what Ansible's apt modules use, python3-debian what its deb822 module uses.
    apt-get -o DPkg::Lock::Timeout=600 install -y --no-install-recommends ansible-core python3-apt python3-debian
}

choose() {
    step "5 of 6: the choice of components"
    if [ -f "$SEED" ]; then
        "$FIBER" "${FIBER_ARGS[@]}" platform seed "$SEED"
        # The seed holds the password: overwrite it before deleting it.
        local size
        size=$(stat -c %s "$SEED")
        if [ "$size" -gt 0 ]; then
            dd if=/dev/zero of="$SEED" bs="$size" count=1 conv=notrunc,fsync status=none
        fi
        rm -f "$SEED"
        sync
        say "Removed the seed from the boot partition."
    elif [ -n "$COMPONENTS" ]; then
        local list
        read -r -a list <<< "${COMPONENTS//,/ }"
        "$FIBER" "${FIBER_ARGS[@]}" platform enable "${list[@]}"
    else
        say "No seed and no --components: the choice in /etc/fiber/platform.yaml stays as it is."
    fi
}

apply() {
    step "6 of 6: applying"
    if [ "${FIBER_NO_APPLY:-}" = 1 ]; then
        say "FIBER_NO_APPLY=1: installed and not applied. Apply it with: sudo fiber platform apply"
        exit 0
    fi
    local status=0
    "$FIBER" "${FIBER_ARGS[@]}" platform apply || status=$?
    # The fiber program has said how it went; what is left is the restart.
    case "$status" in
        0) ;;
        10)
            if [ "${FIBER_NO_REBOOT:-}" = 1 ]; then
                say "FIBER_NO_REBOOT=1, so the FIBER is not restarted now."
            else
                say "Restarting the FIBER."
                systemctl reboot
            fi
            exit 10
            ;;
        20) exit 20 ;;
        *) fail "the apply failed (exit status $status); the lines above say why" ;;
    esac
}

main() {
    COMPONENTS=""
    while [ $# -gt 0 ]; do
        case "$1" in
            --components)
                [ $# -ge 2 ] || fail "--components needs a list, as in --components base,mosquitto"
                COMPONENTS=$2
                shift 2
                ;;
            --components=*)
                COMPONENTS=${1#--components=}
                shift
                ;;
            -h | --help)
                usage
                exit 0
                ;;
            *) fail "unknown option: $1 (see --help)" ;;
        esac
    done
    check_stamped
    setup
    check_machine
    lock
    DOWNLOAD="$OPT/downloads/$ARCHIVE"
    fetch
    unpack
    install_ansible
    choose
    apply
}

main "$@" < /dev/null
