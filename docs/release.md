# Releasing the FIBER platform

How a release is made. What a release holds and how the installer uses it is in
[`provisioning.md`](provisioning.md), "The release archive and the installer".

**Status:** written (`.github/workflows/release.yml`); the publishing has not run yet, as no
release has been made. The repository's *immutable releases* setting is on (since 2026-11-18).

## What a release is

One version for everything: the version in `agent/Cargo.toml` is the version of the `fiber`
program and of the release, and the tag is `v` and that version, as in `v0.2.0`. A release
holds the archive, the stamped `install.sh`, the ready-made image
(`fiber-platform-<version>-image-arm64.img.xz`, [`provisioning.md`](provisioning.md), "The
ready-made image") and `SHA256SUMS`, built from the tagged commit. A
tag is never moved and a release is never changed: a release whose build fails uses up its
version.

## The steps

0. **Once, done:** *immutable releases* is on in the repository's settings (Settings > General >
   Releases, or `gh api -X PUT repos/hardwario/fiber-platform/immutable-releases`), so that a
   published release and its tag cannot be changed.

1. **On `main`, with green CI,** set the version in `agent/Cargo.toml` (and the lock file:
   `cargo update -p fiber-agent`), and commit: "FIBER Platform 0.2.0".
2. **Dry run:** Actions > Release > Run workflow, on `main`. It builds the release, builds it a
   second time and compares, tries the program in a bare container and the installer in a
   container like Raspberry Pi OS, builds the ready-made image from those files and tries its
   first start without a network (`just image`, `just image-test`), and keeps the files as
   workflow artifacts. It publishes nothing. Do it after any change to the build.
3. **Tag:** `git tag v0.2.0 && git push origin v0.2.0`. The workflow checks that the tag is the
   version, that the commit is on `main` and that its CI went green, then builds and tries it
   all again.
4. **Publish:** the workflow's `publish` job does it, the only job with rights to write (D22),
   and only after the gate and the build passed. It checks the files against `SHA256SUMS`, makes a
   provenance attestation of each, uploads them to a draft release, and publishes it as the
   latest. Anyone can check a file: `gh attestation verify <file> --repo hardwario/fiber-platform`.
5. **FIBER Setup** names the new installer by its version and sha256 in its catalog (T12), so a
   new FIBER installs it only with a release of FIBER Setup (D18).

## The weekly canary

`.github/workflows/canary.yml` runs every Monday, and by hand: it converges every component
against the live package sources, as a new FIBER would today, and checks that the keys the
roles carry can sign for 60 more days (`just check-keys`). When a Monday's run fails, its
`report` job, the only one with rights to write (D22), opens the issue "Canary: a package source
or a key broke", or comments on it while it is still open; close it once the cause is fixed. A
run started by hand fails only on the Actions page. It ran by hand once and passed (44200466387);
the `report` job has not run yet.
