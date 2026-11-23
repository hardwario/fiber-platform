#!/usr/bin/env bash
# The ready-made image (docs/provisioning.md, "The ready-made image"), from a release:
#
#   sudo provision/image/build.sh RELEASE_DIR OUT_DIR
#
# RELEASE_DIR holds what a release publishes: its archive, install.sh and SHA256SUMS. OUT_DIR gets
# fiber-platform-<version>-image-arm64.img.xz and its .sha256. On an arm64 Linux (the image's
# programs run natively, without emulation), as root, with the internet.
#
# Each partition is reached through a loop device at its offset, not through partition device
# nodes, so that it runs the same on CI's runner and in a container.
#
#   FIBER_IMAGE_CACHE       where the base is kept between builds (default /var/cache/fiber-image)
#   FIBER_IMAGE_GROW_MIB    room added for the packages (default 2048)
#   FIBER_IMAGE_MARGIN_MIB  free room left in the root file system at the end (default 256)
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
# The base's pin, as the file has it (one place, base.env).
pin() { sed -n "s/^$1=//p" "$here/base.env"; }
BASE_NAME=$(pin BASE_NAME)
BASE_URL=$(pin BASE_URL)
BASE_SHA256=$(pin BASE_SHA256)

say() { printf '%s\n' "$*"; }
step() { printf '\n== %s\n' "$*"; }
fail() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

[ $# -eq 2 ] || fail "usage: $0 RELEASE_DIR OUT_DIR"
[ "$(id -u)" -eq 0 ] || fail "run it as root"
[ "$(uname -m)" = aarch64 ] || fail "run it on an arm64 Linux: the image's programs run natively"
for tool in curl xz sha256sum sfdisk losetup e2fsck resize2fs dumpe2fs zerofree mount chroot; do
    command -v "$tool" > /dev/null || fail "$tool is missing"
done

release_dir=$(realpath "$1")
out=$(realpath -m "$2")
cache=${FIBER_IMAGE_CACHE:-/var/cache/fiber-image}
grow_mib=${FIBER_IMAGE_GROW_MIB:-2048}
margin_mib=${FIBER_IMAGE_MARGIN_MIB:-256}

archives=("$release_dir"/fiber-platform-*-linux-arm64.tar.gz)
[ -e "${archives[0]}" ] || fail "$release_dir has no fiber-platform-<version>-linux-arm64.tar.gz"
archive=${archives[0]##*/}
version=${archive#fiber-platform-}
version=${version%-linux-arm64.tar.gz}
name="fiber-platform-$version-image-arm64.img"

work=$(mktemp -d /var/tmp/fiber-image.XXXXXX)
root="$work/root"
image="$work/image.img"
loops=()

cleanup() {
    set +e
    if [ -d "$root" ]; then
        for dir in dev/pts dev sys proc boot/firmware; do
            mountpoint -q "$root/$dir" && umount "$root/$dir"
        done
        mountpoint -q "$root" && umount "$root"
    fi
    for loop in "${loops[@]}"; do
        losetup -d "$loop" 2> /dev/null
    done
    rm -rf "$work"
}
trap cleanup EXIT

# The start and the size of partition $1, in sectors of 512 bytes.
partition() {
    sfdisk -d "$image" | awk -v n="$image$1" '$1 == n {
        gsub(/[ ,]/, ""); split($0, f, /start=|size=|type=/); print f[2], f[3] }'
}

attach() {
    local loop
    loop=$(losetup --find --show "$@" "$image")
    loops+=("$loop")
    printf '%s\n' "$loop"
}

step "The release $version"
(cd "$release_dir" && sha256sum --ignore-missing -c SHA256SUMS) || fail "the release does not check out"
[ -f "$release_dir/install.sh" ] || fail "$release_dir has no install.sh"

step "The base, $BASE_NAME"
mkdir -p "$cache"
base="$cache/${BASE_URL##*/}"
if [ ! -f "$base" ] || ! echo "$BASE_SHA256  $base" | sha256sum -c --quiet > /dev/null 2>&1; then
    curl -fL --retry 5 --retry-delay 5 -o "$base.part" "$BASE_URL"
    mv "$base.part" "$base"
    echo "$BASE_SHA256  $base" | sha256sum -c --quiet || fail "the base does not check out"
fi
xz -dc "$base" > "$image"

step "Room for the packages: $grow_mib MiB"
truncate -s "+${grow_mib}M" "$image"
echo ", +" | sfdisk -N 2 --no-reread --quiet "$image"
read -r boot_start boot_size < <(partition 1)
read -r root_start _ < <(partition 2)
root_loop=$(attach --offset $((root_start * 512)))
e2fsck -f -y "$root_loop" > /dev/null || [ $? -le 1 ] || fail "the root file system does not check"
resize2fs "$root_loop" > /dev/null
boot_loop=$(attach --offset $((boot_start * 512)) --sizelimit $((boot_size * 512)))

mkdir -p "$root"
mount "$root_loop" "$root"
mount "$boot_loop" "$root/boot/firmware"
for dir in proc sys dev dev/pts; do
    mount --bind "/$dir" "$root/$dir"
done
# The chroot's name service is the builder's while it is built, and its own again after.
resolv_was=$(readlink "$root/etc/resolv.conf" || true)
cp --remove-destination /etc/resolv.conf "$root/etc/resolv.conf"

step "The release installed, Ansible with it"
mkdir -p "$root/tmp/fiber-release"
cp "$release_dir/$archive" "$release_dir/install.sh" "$root/tmp/fiber-release/"
chroot "$root" env FIBER_RELEASE_URL=file:///tmp/fiber-release FIBER_NO_APPLY=1 \
    DEBIAN_FRONTEND=noninteractive bash /tmp/fiber-release/install.sh
install -m 0755 "$release_dir/install.sh" "$root/opt/fiber/install.sh"

step "Every component installed, none turned on"
chroot "$root" env DEBIAN_FRONTEND=noninteractive fiber platform image --base "$BASE_NAME"

step "The first start's own jobs"
# The image's machine id is left empty (below), so that its first start is no first boot to
# systemd: a first boot applies systemd's presets, which enable every unit no preset names, and
# in a container that turned on the platform's services, systemd-networkd-wait-online (which held
# the apply back) and both SSH units (2026-11-23). But two of Raspberry Pi OS's units wait for a
# first boot, and without them 1.0.0's image had no SSH host keys (SSH refused) and its root file
# system left at the image's size (seen on a FIBER, 2026-11-23). These two drop-ins run them on
# the first start all the same.
dropin() {
    mkdir -p "$root/etc/systemd/system/$1.d"
    cat > "$root/etc/systemd/system/$1.d/fiber-image.conf"
}
dropin regenerate_ssh_host_keys.service << 'EOF'
# The ready-made image's first start is no first boot (provision/image/build.sh): the host keys
# are made when there are none. The empty line clears the unit's conditions; the others are its own.
[Unit]
ConditionFirstBoot=
ConditionPathIsReadWrite=/etc
ConditionFileIsExecutable=/usr/bin/ssh-keygen
ConditionPathExists=!/etc/ssh/ssh_host_ed25519_key
EOF
dropin rpi-resize.service << 'EOF'
# The ready-made image's first start is no first boot (provision/image/build.sh): the root file
# system is grown all the same. The unit turns itself off once it has run.
[Unit]
ConditionFirstBoot=
EOF

step "Nothing of one FIBER's, and no caches"
chroot "$root" apt-get clean
rm -rf "$root/tmp/fiber-release" "$root"/var/lib/apt/lists/* "$root"/var/tmp/* "$root/root/.cache" \
    "$root/root/.npm" "$root/root/.ansible" "$root"/run/*
rm -f "$root"/etc/ssh/ssh_host_* "$root/var/lib/dbus/machine-id"
# Empty, not `uninitialized` as in the base: no first boot, so no presets (see "The first start's
# own jobs"); systemd makes the machine id at the first start.
: > "$root/etc/machine-id"
find "$root/var/log" -type f -exec truncate -s 0 {} +
if [ -n "$resolv_was" ]; then
    ln -sfn "$resolv_was" "$root/etc/resolv.conf"
else
    : > "$root/etc/resolv.conf"
fi
for dir in dev/pts dev sys proc boot/firmware; do
    umount "$root/$dir"
done
umount "$root"

step "Shrunk to what it holds, and $margin_mib MiB"
e2fsck -f -y "$root_loop" > /dev/null || [ $? -le 1 ] || fail "the root file system does not check"
resize2fs -M "$root_loop" > /dev/null
block_size=$(dumpe2fs -h "$root_loop" 2> /dev/null | awk -F: '/^Block size/ { gsub(/ /, "", $2); print $2 }')
blocks=$(dumpe2fs -h "$root_loop" 2> /dev/null | awk -F: '/^Block count/ { gsub(/ /, "", $2); print $2 }')
# Whole 4 MiB, as the partitions of the base are aligned.
size_mib=$(((blocks * block_size / 1048576 + margin_mib + 3) / 4 * 4))
resize2fs "$root_loop" "${size_mib}M" > /dev/null
e2fsck -f -y "$root_loop" > /dev/null || [ $? -le 1 ] || fail "the root file system does not check"
zerofree "$root_loop"
for loop in "${loops[@]}"; do
    losetup -d "$loop"
done
loops=()
echo ",$((size_mib * 2048))" | sfdisk -N 2 --no-reread --quiet "$image"
truncate -s $(((root_start + size_mib * 2048) * 512)) "$image"
say "$(du -h --apparent-size "$image" | cut -f1) uncompressed"

step "Compressed: $name.xz"
mkdir -p "$out"
xz -T0 -6 -c "$image" > "$out/$name.xz.part"
mv "$out/$name.xz.part" "$out/$name.xz"
(cd "$out" && sha256sum "$name.xz" > "$name.xz.sha256")
say "$(du -h "$out/$name.xz" | cut -f1)  $out/$name.xz"
