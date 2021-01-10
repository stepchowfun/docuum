#!/usr/bin/env bash
set -euo pipefail

# This script generates release artifacts in a directory called `release`. It should be run from a
# macOS machine with an x86-64 processor. Usage:
#   ./release.sh

# The release process involves four steps:
# 1. Bump the version in `Cargo.toml`, run `cargo build` to update `Cargo.lock`, and update
#    `CHANGELOG.md` with information about the new version. Ship those changes as a single pull
#    request.
# 2. Run this script on an x86-64 machine and upload the files in the `release` directory to GitHub
#    as release artifacts.
# 3. Build and upload the Docker image:
#      docker build --tag stephanmisc/docuum:latest --tag stephanmisc/docuum:X.Y.Z -f Dockerfile.unix .
#      docker push stephanmisc/docuum:latest
#      docker push stephanmisc/docuum:X.Y.Z
# 4. Update the version in `install.sh` to point to the new release.

# We wrap everything in parentheses to ensure that any working directory changes with `cd` are local
# to this script and don't affect the calling user's shell.
(
  # x86-64 macOS build
  rm -rf target/release
  cargo build --release

  # x86-64 GNU/Linux builds
  rm -rf artifacts
  toast release

  # Prepare the `release` directory.
  rm -rf release
  mkdir release

  # Copy the artifacts into the `release` directory.
  cp target/release/docuum release/docuum-x86_64-apple-darwin
  cp artifacts/docuum-x86_64-unknown-linux-gnu release/docuum-x86_64-unknown-linux-gnu
  cp artifacts/docuum-x86_64-unknown-linux-musl release/docuum-x86_64-unknown-linux-musl

  # Compute checksums of the artifacts.
  cd release
  shasum --algorithm 256 --binary docuum-x86_64-apple-darwin > docuum-x86_64-apple-darwin.sha256
  shasum --algorithm 256 --binary docuum-x86_64-unknown-linux-gnu > docuum-x86_64-unknown-linux-gnu.sha256
  shasum --algorithm 256 --binary docuum-x86_64-unknown-linux-musl > docuum-x86_64-unknown-linux-musl.sha256

  # Verify the checksums.
  shasum --algorithm 256 --check --status docuum-x86_64-apple-darwin.sha256
  shasum --algorithm 256 --check --status docuum-x86_64-unknown-linux-gnu.sha256
  shasum --algorithm 256 --check --status docuum-x86_64-unknown-linux-musl.sha256

  # Publish to crates.io.
  cargo publish
)
