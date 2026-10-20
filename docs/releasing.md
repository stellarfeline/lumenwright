# Releasing

[Lumenwright](../README.md)

## Releasing

Releases are published by CI only, never from a workstation. The `release` workflow
(`.github/workflows/release.yml`) is run by hand on `main` with the version to release. It
refuses unless that version is the workspace version, `main`'s CI succeeded at the commit,
neither crate has that version on crates.io, no tag `v<version>` exists, and `CHANGELOG.md`
has a dated `## [<version>] - YYYY-MM-DD` section. It then runs the tests, publishes
`lumenwright-kernel` and then `lumenwright` with the repository secret
`CARGO_REGISTRY_TOKEN` (a crates.io API token for both crates), and tags `v<version>` with a
GitHub Release whose notes are that changelog section. `lumenwright-cli` is not published.
