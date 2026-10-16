# Releasing

dv is published to crates.io as `dv-review` (the binary is `dv`), and each version gets a
GitHub release with prebuilt binaries.

1. **Pick the version** (semver; while 0.x, a minor bump for anything that breaks how dv is
   used: keys, flags, the review file format).
2. **Update** `version` in `Cargo.toml`, run `cargo build` (so `Cargo.lock` follows), and add a
   `## [X.Y.Z] - YYYY-MM-DD` section at the top of `CHANGELOG.md` (plus its link at the
   bottom). That section becomes the GitHub release's notes.
3. **Check**: `cargo test`, `cargo clippy --all-targets`, `cargo publish --dry-run`.
4. **Commit and tag**: `git commit -am "Release vX.Y.Z"`, `git tag vX.Y.Z`,
   `git push && git push --tags`.
5. **GitHub release**: the tag starts `.github/workflows/release.yml`. It checks the tag
   matches `Cargo.toml`, builds `dv` for Linux (x86_64, aarch64) and macOS (x86_64, arm64),
   and creates the release with those archives and the changelog section. Watch it with
   `gh run watch`.
6. **crates.io**: `cargo publish` (from a clean checkout of the tag; needs `cargo login`).
   A published version can't be replaced, only yanked (`cargo yank --version X.Y.Z`), so
   publish once the GitHub release has built.
   Then `cargo clean -p dv-review`: publishing's verify build leaves `target/debug/dv`
   tracking the packaged sources, so later `cargo build`s don't rebuild it.

If the release workflow fails, fix it on `master`, then move the tag:
`git tag -f vX.Y.Z && git push -f origin vX.Y.Z` (only before `cargo publish`).
