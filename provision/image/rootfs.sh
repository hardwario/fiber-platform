#!/usr/bin/env bash
# The root of a ready-made image, its boot partition at /boot/firmware, as a tar for `docker
# import` (`just check-image`, provision/tests/image-test.sh):
#
#   sudo provision/image/rootfs.sh IMAGE.img.xz OUT.tar
#
# As root, on Linux; each partition through a loop device at its offset, as build.sh does.
set -euo pipefail

fail() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

[ $# -eq 2 ] || fail "usage: $0 IMAGE.img.xz OUT.tar"
[ "$(id -u)" -eq 0 ] || fail "run it as root"
work=$(mktemp -d /var/tmp/fiber-rootfs.XXXXXX)
image="$work/image.img"
root="$work/root"
loops=()

cleanup() {
    set +e
    mountpoint -q "$root/boot/firmware" && umount "$root/boot/firmware"
    mountpoint -q "$root" && umount "$root"
    for loop in "${loops[@]}"; do
        losetup -d "$loop" 2> /dev/null
    done
    rm -rf "$work"
}
trap cleanup EXIT

partition() {
    sfdisk -d "$image" | awk -v n="$image$1" '$1 == n {
        gsub(/[ ,]/, ""); split($0, f, /start=|size=|type=/); print f[2], f[3] }'
}

xz -dc "$1" > "$image"
read -r boot_start boot_size < <(partition 1)
read -r root_start root_size < <(partition 2)
root_loop=$(losetup --find --show --read-only --offset $((root_start * 512)) --sizelimit $((root_size * 512)) "$image")
loops+=("$root_loop")
boot_loop=$(losetup --find --show --read-only --offset $((boot_start * 512)) --sizelimit $((boot_size * 512)) "$image")
loops+=("$boot_loop")
mkdir -p "$root"
mount -o ro "$root_loop" "$root"
mount -o ro "$boot_loop" "$root/boot/firmware"
tar -C "$root" --numeric-owner -cf "$2" .
