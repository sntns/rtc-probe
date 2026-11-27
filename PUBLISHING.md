# Publishing

Releases go to crates.io with [cargo-release](https://github.com/crate-ci/cargo-release), configured in `Cargo.toml` under `[package.metadata.release]`.

## Prerequisite

The secret `CARGO_REGISTRY_TOKEN`: a crates.io token allowed to publish `rtc-probe`.

## Cutting a release

In GitHub: **Actions → Release and Publish → Run workflow**, then pick the level (patch, minor, major).

The workflow (`.github/workflows/release.yml`):

1. bumps the version in `Cargo.toml`;
2. turns the changelog's `## Unreleased` into `## X.Y.Z - date`, under a new empty `## Unreleased`;
3. publishes the crate to crates.io;
4. commits (`chore: release vX.Y.Z`), tags `vX.Y.Z` and pushes both to `main`.

Before cutting a release, list its changes under `## Unreleased` in `CHANGELOG.md`.
