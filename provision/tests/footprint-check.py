#!/usr/bin/env python3
"""The footprint test: is every path the platform added accounted for?

    footprint-check.py <before.txt> <after.txt> <footprint.yaml> [--report] [--uninstalled]

before and after are sorted lists of the paths on the system (`find`). A new path is accounted for
when a Debian package owns it (the file lists in /var/lib/dpkg/info), when a package's own scripts
make it (the few kinds listed below), when it is in the platform's own places, or when
`provision/footprint.yaml` lists it or a directory above it. The rest is printed, and the test
fails. With --report it only prints. With --uninstalled (after `fiber platform uninstall`) only
the footprint's data and changed package files count, and the platform's own places must be
gone.
"""

import fnmatch
import glob
import os
import sys

import yaml  # python3-yaml comes with ansible-core

OWN = ("/opt/fiber", "/etc/fiber", "/var/lib/fiber")

# What packages' own scripts make on install, and what their services make when they run, which
# their file lists do not name. A link to a file a package owns counts as the package's too.
PACKAGE_MADE = [
    "*/__pycache__", "*/__pycache__/*",
    "/etc/alternatives/*",
    "/etc/ssl/certs/*",
    "/etc/rc?.d/*",
    "/etc/hosts.allow", "/etc/hosts.deny",
    "/etc/systemd/*/*.wants", "/etc/systemd/*/*.wants/*",
    "/var/lib/systemd/deb-systemd-*", "/var/lib/systemd/deb-systemd-*/*",
    "/var/lib/dpkg/*",
    "/var/lib/ucf/*",
    "/etc/ld.so.cache",
    "/usr/lib/python3/dist-packages/*.dist-info/*",
    # The services' own data.
    "/var/lib/redis", "/var/lib/redis/*",
    "/var/lib/mosquitto/*",
]


def read_lines(path):
    with open(path) as f:
        return {line.rstrip("\n") for line in f if line.strip()}


def main():
    before, after, footprint_file = sys.argv[1:4]
    report = "--report" in sys.argv
    uninstalled = "--uninstalled" in sys.argv
    owned = set()
    for listing in glob.glob("/var/lib/dpkg/info/*.list"):
        owned |= read_lines(listing)
    with open(footprint_file) as f:
        entries = yaml.safe_load(f)["paths"]
    if uninstalled:
        footprint = [e["path"] for e in entries if e.get("kind") in ("data", "changed")]
    else:
        footprint = [e["path"] for e in entries]

    def accounted(path):
        if path in owned:
            return True
        if not uninstalled and any(path == own or path.startswith(own + "/") for own in OWN):
            return True
        if any(fnmatch.fnmatch(path, pattern) for pattern in PACKAGE_MADE):
            return True
        if os.path.islink(path) and os.path.realpath(path) in owned:
            return True
        return any(path == p or path.startswith(p.rstrip("/") + "/") or fnmatch.fnmatch(path, p) for p in footprint)

    new = sorted(read_lines(after) - read_lines(before))
    unaccounted = [p for p in new if not accounted(p)]
    for path in unaccounted:
        print(f"not accounted for: {path}")
    print(f"{len(new)} new paths, {len(unaccounted)} not accounted for")
    if unaccounted and not report:
        sys.exit(1)


if __name__ == "__main__":
    main()
