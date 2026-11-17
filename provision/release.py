#!/usr/bin/env python3
"""Builds a release of the FIBER platform into a directory: the archive, the installer stamped with
its version and the archive's sha256, and SHA256SUMS.

    python3 provision/release.py --binary target/aarch64-unknown-linux-musl/release/fiber --out dist

The archive is the same byte for byte for the same inputs: its entries sorted, every date the
commit's, everything owned by root, and gzip without a name or a date. `--self-test` builds it
twice and compares. Python's standard library only.
"""

import argparse
import gzip
import hashlib
import io
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
PROVISION = ROOT / "provision"

# What of `provision/` stays out of the archive: the tools that build and test it. The installer
# is published beside the archive, stamped.
LEFT_OUT = {"release.py", "install.sh", "tests", "__pycache__"}

# The ansible-core series the roles are written for: Debian trixie's.
ANSIBLE_CORE = "2.19"

MANIFEST_SCHEMA = 1


def fail(message):
    sys.exit(f"error: {message}")


def version():
    """The version of the `fiber` program, which is the version of the platform."""
    text = (ROOT / "agent" / "Cargo.toml").read_text()
    found = re.search(r'^version = "([^"]+)"', text, re.MULTILINE)
    if not found:
        fail("agent/Cargo.toml has no version")
    return found.group(1)


def git(*args):
    return subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True, check=True).stdout.strip()


def check_binary(data):
    """The program must be for the FIBER: a 64-bit ELF for AArch64 (e_machine 183)."""
    if data[:4] != b"\x7fELF" or data[4] != 2 or int.from_bytes(data[18:20], "little") != 183:
        fail("the binary is not an arm64 Linux program; build it with `just build-fiber`")


def compile_overlay(source):
    """A device tree overlay, compiled as the firmware loads it (`dtc -@`)."""
    if not shutil.which("dtc"):
        fail("dtc is missing (Debian: device-tree-compiler; macOS: brew install dtc)")
    result = subprocess.run(
        ["dtc", "-@", "-q", "-I", "dts", "-O", "dtb", "-o", "-", str(source)], capture_output=True, check=False
    )
    if result.returncode != 0:
        fail(f"dtc could not compile {source}:\n{result.stderr.decode()}")
    return result.stdout


def collect(binary):
    """Every file of the archive but the manifest: {path in the archive: (bytes, mode)}. Each
    overlay source `*.dts` goes in compiled as `*.dtbo` too."""
    files = {}
    data = binary.read_bytes()
    check_binary(data)
    files["bin/fiber"] = (data, 0o755)
    for path in sorted(PROVISION.rglob("*")):
        rel = path.relative_to(PROVISION)
        if rel.parts[0] in LEFT_OUT or "__pycache__" in rel.parts or path.is_dir():
            continue
        mode = 0o755 if os.access(path, os.X_OK) else 0o644
        files[f"provision/{rel.as_posix()}"] = (path.read_bytes(), mode)
        if path.suffix == ".dts":
            files[f"provision/{rel.with_suffix('.dtbo').as_posix()}"] = (compile_overlay(path), 0o644)
    return files


def manifest(files, release_version, commit):
    return {
        "schema": MANIFEST_SCHEMA,
        "version": release_version,
        "commit": commit,
        "ansible_core": ANSIBLE_CORE,
        "files": {path: hashlib.sha256(data).hexdigest() for path, (data, _) in sorted(files.items())},
    }


def archive(entries, epoch):
    """The `.tar.gz` of {path: (bytes, mode)}, as bytes, with a directory entry for every
    directory."""
    dirs = set()
    for path in entries:
        parts = path.split("/")[:-1]
        for i in range(1, len(parts) + 1):
            dirs.add("/".join(parts[:i]))

    tar_bytes = io.BytesIO()
    with tarfile.open(fileobj=tar_bytes, mode="w", format=tarfile.PAX_FORMAT) as tar:
        for name in sorted(dirs | set(entries)):
            info = tarfile.TarInfo(name)
            info.mtime = epoch
            info.uid = info.gid = 0
            info.uname = info.gname = "root"
            if name in dirs:
                info.type = tarfile.DIRTYPE
                info.mode = 0o755
                tar.addfile(info)
            else:
                data, mode = entries[name]
                info.mode = mode
                info.size = len(data)
                tar.addfile(info, io.BytesIO(data))

    out = io.BytesIO()
    with gzip.GzipFile(filename="", mode="wb", fileobj=out, mtime=0, compresslevel=9) as gz:
        gz.write(tar_bytes.getvalue())
    return out.getvalue()


def stamp(release_version, archive_sha256):
    """`install.sh` with its version and the archive's sha256 in place of the two marks."""
    text = (PROVISION / "install.sh").read_text()
    for mark, value in (("@VERSION@", release_version), ("@ARCHIVE_SHA256@", archive_sha256)):
        if text.count(f'"{mark}"') != 1:
            fail(f"provision/install.sh must have the mark {mark} once, in quotes")
        text = text.replace(f'"{mark}"', f'"{value}"')
    return text


def build(binary, out, release_version, commit, epoch):
    files = collect(binary)
    files["manifest.json"] = (
        (json.dumps(manifest(files, release_version, commit), indent=2, sort_keys=True) + "\n").encode(),
        0o644,
    )
    name = f"fiber-platform-{release_version}-linux-arm64.tar.gz"
    data = archive(files, epoch)
    digest = hashlib.sha256(data).hexdigest()
    installer = stamp(release_version, digest).encode()

    out.mkdir(parents=True, exist_ok=True)
    (out / name).write_bytes(data)
    (out / "install.sh").write_bytes(installer)
    (out / "install.sh").chmod(0o755)
    sums = f"{digest}  {name}\n{hashlib.sha256(installer).hexdigest()}  install.sh\n"
    (out / "SHA256SUMS").write_text(sums)
    return name, digest


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--binary", type=pathlib.Path, required=True, help="the static arm64 `fiber`")
    parser.add_argument("--out", type=pathlib.Path, default=ROOT / "dist")
    parser.add_argument("--self-test", action="store_true", help="build twice and compare, keep nothing")
    parser.add_argument(
        "--version",
        help="another version number, for a test release (the program in it still says its own)",
    )
    args = parser.parse_args()

    release_version = args.version or version()
    commit = git("rev-parse", "HEAD")
    if git("status", "--porcelain"):
        commit += "-dirty"
    epoch = int(os.environ.get("SOURCE_DATE_EPOCH") or git("log", "-1", "--format=%ct"))

    if args.self_test:
        with tempfile.TemporaryDirectory() as tmp:
            first = build(args.binary, pathlib.Path(tmp) / "a", release_version, commit, epoch)
            second = build(args.binary, pathlib.Path(tmp) / "b", release_version, commit, epoch)
            for name in (first[0], "install.sh", "SHA256SUMS"):
                if (pathlib.Path(tmp) / "a" / name).read_bytes() != (pathlib.Path(tmp) / "b" / name).read_bytes():
                    fail(f"{name} differs between two builds")
        print(f"release self-test passed: {first[0]} is the same in two builds ({first[1][:12]})")
        return

    name, digest = build(args.binary, args.out, release_version, commit, epoch)
    print(f"{args.out / name}\n  sha256 {digest}\n{args.out / 'install.sh'}\n{args.out / 'SHA256SUMS'}")


if __name__ == "__main__":
    main()
