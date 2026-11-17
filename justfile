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
# Run every hardware-free gate (host workspace, the FIBER's build, the release, scripts, no long dashes, audit).
check: check-host build-fiber check-release shellcheck check-hil no-dashes audit

# The thumbv7em build proves the pinned TOWER kernels and our own no_std crates still build for
# the north bridge (an STM32WL55, Cortex-M4 without FPU) on their own.
# Host workspace gates: fmt, clippy, tests, the no_std build, rustdoc.
check-host: doc
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets --locked -- -D warnings
    cargo test --workspace --locked
    cargo build --locked --target thumbv7em-none-eabi -p fiber-tower-air

# rust-lld links it with the target's own musl (.cargo/config.toml): no C toolchain needed.
# Build the static `fiber` program for the FIBER (arm64 Linux, musl).
build-fiber:
    cargo build --locked --release --target aarch64-unknown-linux-musl -p fiber-agent

# The archive, the stamped installer and SHA256SUMS (provision/release.py).
# Build a release of the platform into dist/.
release: build-fiber
    python3 provision/release.py --binary target/aarch64-unknown-linux-musl/release/fiber --out dist

# Builds the release twice and compares the bytes; keeps nothing.
# Check that the release archive is the same in every build.
check-release: build-fiber
    python3 provision/release.py --binary target/aarch64-unknown-linux-musl/release/fiber --self-test

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
check-converge: release
    docker build --platform linux/arm64 -q -t fiber-platform-test-systemd -f provision/tests/Containerfile.systemd provision/tests
    docker rm -f fiber-platform-converge > /dev/null 2>&1 || true
    docker run -d --name fiber-platform-converge --platform linux/arm64 --privileged --cgroupns=host \
        -v /sys/fs/cgroup:/sys/fs/cgroup:rw --tmpfs /run --tmpfs /run/lock \
        -v "{{justfile_directory()}}/dist:/dist:ro" \
        -v "{{justfile_directory()}}/provision/tests/converge-test.sh:/converge-test.sh:ro" \
        fiber-platform-test-systemd > /dev/null
    docker exec fiber-platform-converge bash /converge-test.sh; status=$?; docker rm -f fiber-platform-converge > /dev/null; exit $status

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
