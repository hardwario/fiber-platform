# HARDWARIO FIBER platform: task runner (https://just.systems)
#
# The HOST workspace lives in this directory (crates/*, agent). The two firmware workspaces
# (firmware/south, firmware/north) will get justfiles of their own, run inside their directories
# so that their cargo config applies.
#
#   just check     # every hardware-free gate CI runs
#   just test      # host workspace tests
#
# Nothing here touches a device. The bench scripts are in hil/ and are run by hand on a FIBER.

# On Windows, run recipe lines with cmd.exe (propagates child exit codes); Unix keeps `sh`.
set windows-shell := ["cmd.exe", "/c"]

# The running `just`, so the delegating recipes use the same binary.
just := just_executable()

# The bare container of `check-container`: Debian trixie, the base of Raspberry Pi OS, pinned by
# the digest of its multi-platform index (looked up 2026-11-17).
debian_image := "debian:trixie-slim@sha256:a72b4433de1cf7d6bae0c4cee74d0547f58601a643b3375f070c51600d63b6b2"

# List the available recipes.
default:
    @{{just}} --list


# === Gates ========================================================================================

# What CI runs. Needs cargo-audit, python3 and network access (the RustSec advisory database).
# Run every hardware-free gate (host workspace, firmware, the FIBER's build, the release, scripts, no long dashes, audit).
check: check-host check-south build-fiber check-release shellcheck check-hil no-dashes audit

# The thumb builds prove the pinned TOWER kernels and our own no_std crates still build for the
# north bridge (an STM32WL55, Cortex-M4 without FPU) and the south bridge (STM32C031, Cortex-M0+).
# Host workspace gates: fmt, clippy, tests, the no_std build, rustdoc.
check-host: doc
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets --locked -- -D warnings
    cargo test --workspace --locked
    cargo build --locked --target thumbv7em-none-eabi -p fiber-tower-air -p fiber-protocol
    cargo build --locked --target thumbv6m-none-eabi -p fiber-protocol

# A workspace of its own, built inside its directory so that its target, its linker (flip-link) and
# its profile apply.
# Build the south bridge's firmware.
south:
    cd firmware/south && cargo build --locked --release

# The flash it takes: its sections as llvm-size (from the toolchain's llvm-tools) has them.
# Measure the south bridge's firmware against its budget of 24 KB.
south-size: south
    #!/usr/bin/env bash
    set -euo pipefail
    size_tool=$(find "$(rustc --print sysroot)/lib/rustlib" -name llvm-size -type f | head -n 1)
    flash=$("$size_tool" -A firmware/south/target/thumbv6m-none-eabi/release/fiber-south |
        awk '/^\.vector_table|^\.text|^\.rodata|^\.data/ {f += $2} END {print f}')
    echo "south bridge firmware: $flash bytes of flash, budget 24576"
    [ "$flash" -le 24576 ] || { echo "error: over the budget (docs/bench-checks.md, check 5)"; exit 1; }

# Needs flip-link (cargo install flip-link).
# Check the south bridge's firmware: fmt, clippy, rustdoc, and its size.
check-south: south-size
    cd firmware/south && cargo fmt -- --check
    cd firmware/south && cargo clippy --locked --release -- -D warnings
    cd firmware/south && RUSTDOCFLAGS="-D warnings" cargo doc --locked --release --no-deps

# rust-lld links it with the target's own musl (.cargo/config.toml): no C toolchain needed. It
# carries the south bridge's firmware as a binary image (llvm-objcopy, from llvm-tools), to flash
# the chip with.
# Build the static `fiber` program for the FIBER (arm64 Linux, musl), with the firmware in it.
build-fiber: south
    #!/usr/bin/env bash
    set -euo pipefail
    objcopy=$(find "$(rustc --print sysroot)/lib/rustlib" -name llvm-objcopy -type f | head -n 1)
    mkdir -p target/firmware
    "$objcopy" -O binary firmware/south/target/thumbv6m-none-eabi/release/fiber-south target/firmware/south.bin
    FIBER_SOUTH_IMAGE="$PWD/target/firmware/south.bin" \
        cargo build --locked --release --target aarch64-unknown-linux-musl -p fiber-agent

# The archive, the stamped installer and SHA256SUMS (provision/release.py).
# Build a release of the platform into dist/.
release: build-fiber
    python3 provision/release.py --binary target/aarch64-unknown-linux-musl/release/fiber --out dist

# For the update test: dist/test/releases/ as GitHub serves a release's files (download/v<version>/
# and latest/download/), with this release and a test release `<version>-update-test`, the latest.
# Build two releases for the update test.
test-releases: build-fiber
    #!/usr/bin/env bash
    set -euo pipefail
    version=$(sed -n 's/^version = "\(.*\)"$/\1/p' agent/Cargo.toml | head -n 1)
    rm -rf dist/test
    for v in "$version" "$version-update-test"; do
        python3 provision/release.py --binary target/aarch64-unknown-linux-musl/release/fiber \
            --out "dist/test/releases/download/v$v" --version "$v" > /dev/null
    done
    mkdir -p dist/test/releases/latest
    cp -r "dist/test/releases/download/v$version-update-test" dist/test/releases/latest/download

# Builds the release twice and compares the bytes; keeps nothing.
# Check that the release archive is the same in every build.
check-release: build-fiber
    python3 provision/release.py --binary target/aarch64-unknown-linux-musl/release/fiber --self-test

# Needs gpg. The weekly canary runs it (.github/workflows/canary.yml).
# Check that the package sources' keys the roles carry can sign for 60 more days.
check-keys:
    python3 provision/tests/keys-check.py

# Every shell script of the repository.
# Run shellcheck on the shell scripts.
shellcheck:
    shellcheck provision/install.sh provision/tests/*.sh agent/tests/*.sh

# Needs Docker on an arm64 machine and the internet (the installer installs Ansible from Debian).
# Run the installer's tests in a container like Raspberry Pi OS.
check-install: release
    docker build --platform linux/arm64 -q -t fiber-platform-test-rpios -f provision/tests/Containerfile provision/tests
    docker run --rm --platform linux/arm64 \
        -v "{{justfile_directory()}}/dist:/dist:ro" \
        -v "{{justfile_directory()}}/provision/tests/install-test.sh:/install-test.sh:ro" \
        fiber-platform-test-rpios bash /install-test.sh

# In a Debian trixie container (Docker), with the tools Debian has; on a copy, as ansible-lint
# writes its cache next to what it checks.
# Run yamllint and ansible-lint (with only-builtins) on the playbook and the roles.
lint-ansible:
    docker build -q -t fiber-platform-lint -f provision/tests/Containerfile.lint provision/tests
    docker run --rm -v "{{justfile_directory()}}/provision:/provision:ro" fiber-platform-lint sh -c \
        'cp -r /provision /tmp/provision && cd /tmp/provision/ansible && yamllint -s -c ../tests/yamllint.yml . && ansible-lint -c ../tests/ansible-lint.yml'

# Needs Docker on an arm64 machine and the internet (the roles install from Debian). The container
# runs systemd, privileged, so that services start in it as on a FIBER.
# Run the converge test: install, apply, apply again, without network, with a role made to fail.
check-converge: release test-releases
    docker build --platform linux/arm64 -q -t fiber-platform-test-systemd -f provision/tests/Containerfile.systemd provision/tests
    docker rm -f fiber-platform-converge > /dev/null 2>&1 || true
    docker run -d --name fiber-platform-converge --platform linux/arm64 --privileged --cgroupns=host \
        -v /sys/fs/cgroup:/sys/fs/cgroup:rw --tmpfs /run --tmpfs /run/lock \
        -v "{{justfile_directory()}}/dist:/dist:ro" \
        -v "{{justfile_directory()}}/provision/tests:/tests:ro" \
        -v "{{justfile_directory()}}/provision/footprint.yaml:/footprint.yaml:ro" \
        fiber-platform-test-systemd > /dev/null
    docker exec fiber-platform-converge bash /tests/converge-test.sh; status=$?; docker rm -f fiber-platform-converge > /dev/null; exit $status

# Needs Docker on an arm64 machine (an Apple silicon Mac, or CI's arm64 runner). The container
# has no network, no broker, no hardware and no settings. Not part of `check`.
# Run the static program's commands in a bare Debian container.
check-container: build-fiber
    docker run --rm --platform linux/arm64 --network none \
        -v "{{justfile_directory()}}/target/aarch64-unknown-linux-musl/release/fiber:/usr/local/bin/fiber:ro" \
        -v "{{justfile_directory()}}/agent/tests/bare-container.sh:/bare-container.sh:ro" \
        {{debian_image}} sh /bare-container.sh

# The probe talks to a fake ROM bootloader on a pseudo-terminal: the protocol code is exercised,
# no GPIO is touched.
# Self-test of the bench scripts (no hardware).
check-hil:
    python3 hil/bootloader_probe.py --self-test

# Vulnerabilities fail; informational warnings (unmaintained, yanked) do not.
# RustSec audit of the lockfile.
audit:
    cargo audit
    cargo audit --file firmware/south/Cargo.lock

# Build the rustdoc of the workspace, failing on any warning.
[env("RUSTDOCFLAGS", "-D warnings")]
doc:
    cargo doc --locked --no-deps --workspace

# Extra args go to `cargo test`, e.g. `just test -p fiber-agent`.
# Run the host workspace tests.
test *args:
    cargo test --workspace --locked {{args}}

# Apply rustfmt to the workspace.
fmt:
    cargo fmt --all

# AGENTS.md, "Writing style". The two characters are named by code point so that this file passes.
# Fail if any tracked or new file contains a long dash (em or en).
[script("python3")]
no-dashes:
    import pathlib, subprocess, sys
    files = subprocess.run(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard"],
        check=True, capture_output=True, text=True,
    ).stdout.splitlines()
    found = 0
    for name in files:
        try:
            text = pathlib.Path(name).read_text(encoding="utf-8")
        except (UnicodeDecodeError, FileNotFoundError, IsADirectoryError):
            continue
        for number, line in enumerate(text.splitlines(), 1):
            if chr(0x2013) in line or chr(0x2014) in line:
                print(f"{name}:{number}: {line.strip()}")
                found += 1
    if found:
        sys.exit(f"{found} line(s) with a long dash (use a comma, a colon, brackets or a hyphen)")
