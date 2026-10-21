# Releasing

[Lumenwright](../README.md)

## Releasing

Releases are published by CI only, never from a workstation. The `release` workflow
(`.github/workflows/release.yml`) is run by hand on `main` with the version to release. It
refuses unless that version is the version of all three crates and none is marked
`publish = false`, `main`'s CI succeeded at the commit, none of `lumenwright-kernel`,
`lumenwright` and `lumenwright-cli` has that version on crates.io, no tag `v<version>`
exists, and `CHANGELOG.md` has a dated `## [<version>] - YYYY-MM-DD` section (a section still
marked Unreleased is refused). It then runs the tests, publishes `lumenwright-kernel`,
`lumenwright` and `lumenwright-cli` in that order, each once the crates.io index serves the
one it depends on, with the repository secret `CARGO_REGISTRY_TOKEN` (a crates.io API token
for the three crates), and tags `v<version>` with a GitHub Release whose notes are that
changelog section.

Before a release, `cargo publish --dry-run --workspace` packages the three crates and builds
each from its package against the others as packaged; it needs no token.
