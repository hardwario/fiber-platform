# Releasing the FIBER platform

How a release is made. What a release holds and how the installer uses it is in
[`provisioning.md`](provisioning.md), "The release archive and the installer".

**Status:** the build and the checks are written (`.github/workflows/release.yml`); publishing
is not written yet. D22 says how: one job of its own with the rights to write. No release has
been made yet.

## What a release is

One version for everything: the version in `agent/Cargo.toml` is the version of the `fiber`
program and of the release, and the tag is `v` and that version, as in `v0.2.0`. A release
holds the archive, the stamped `install.sh` and `SHA256SUMS`, built from the tagged commit. A
tag is never moved and a release is never changed: a release whose build fails uses up its
version.

## The steps

1. **On `main`, with green CI,** set the version in `agent/Cargo.toml` (and the lock file:
   `cargo update -p fiber-agent`), and commit: "FIBER Platform 0.2.0".
2. **Dry run:** Actions > Release > Run workflow, on `main`. It builds the release, builds it a
   second time and compares, tries the program in a bare container and the installer in a
   container like Raspberry Pi OS, and keeps the files as a workflow artifact. It publishes
   nothing. Do it after any change to the build.
3. **Tag:** `git tag v0.2.0 && git push origin v0.2.0`. The workflow checks that the tag is the
   version, that the commit is on `main` and that its CI went green, then builds and tries it
   all again.
4. **Publish:** not written yet. One job of the workflow, the only one with the rights to write
   (D22), uploads everything to a draft release, then publishes it.
5. **FIBER Setup** names the new installer by its version and sha256 in its catalog (T12), so a
   new FIBER installs it only with a release of FIBER Setup (D18).

## The weekly canary

`.github/workflows/canary.yml` runs every Monday, and by hand: it converges every component
against the live package sources, as a new FIBER would today, and checks that the keys the
roles carry can sign for 60 more days (`just check-keys`). A failed run shows on the Actions page
and is mailed to whoever last changed its schedule. Not run yet.
