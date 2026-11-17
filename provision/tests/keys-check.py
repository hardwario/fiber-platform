#!/usr/bin/env python3
"""Are the package sources' keys that the roles carry still good? (`just check-keys`)

For every `*.asc` under provision/ansible/roles/*/files, gpg reads the key; it is good when a key
that can sign (the primary key or a subkey) is valid for at least 60 more days. A subkey that has
expired is fine while another one, or the primary key, still signs. Needs gpg.
"""

import pathlib
import subprocess
import sys
import time

DAYS = 60
ROOT = pathlib.Path(__file__).resolve().parent.parent


def signing_keys(path):
    """(fingerprint, expiry or None) of every key in the file that can sign and is not revoked."""
    out = subprocess.run(
        ["gpg", "--show-keys", "--with-colons", str(path)], capture_output=True, text=True, check=True
    ).stdout
    keys, current = [], None
    for line in out.splitlines():
        fields = line.split(":")
        if fields[0] in ("pub", "sub"):
            validity, expiry, capabilities = fields[1], fields[6], fields[11]
            current = None
            if validity != "r" and "s" in capabilities.lower():
                current = [None, int(expiry) if expiry else None]
                keys.append(current)
        elif fields[0] == "fpr" and current is not None and current[0] is None:
            current[0] = fields[9]
    return keys


def main():
    limit = time.time() + DAYS * 86400
    bad = 0
    for path in sorted(ROOT.glob("ansible/roles/*/files/*.asc")):
        keys = signing_keys(path)
        good = [k for k in keys if k[1] is None or k[1] > limit]
        name = path.relative_to(ROOT.parent)
        if good:
            until = "no expiry" if any(k[1] is None for k in good) else time.strftime(
                "%Y-%m-%d", time.gmtime(max(k[1] for k in good))
            )
            print(f"ok    {name}: signs until {until}")
        else:
            bad += 1
            print(f"::warning::{name}: no key in it can sign {DAYS} days from now; fetch the new key")
            print(f"FAIL  {name}")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
