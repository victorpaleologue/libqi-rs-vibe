# Releasing

A release is a git tag `vX.Y.Z` on `main` whose version equals the workspace version in
`Cargo.toml`. Pushing the tag runs `.github/workflows/release.yml`, which:

1. checks that the tag, the workspace version and a `## [X.Y.Z]` section of `CHANGELOG.md`
   agree, runs the tests and a publish dry run;
2. builds `qi-cli` and `naoqi-sim` for Linux (x86_64, aarch64), macOS (x86_64, aarch64) and
   Windows (x86_64), and attaches the archives and their checksums to a GitHub release
   whose notes are the changelog section;
3. publishes to crates.io the crates of the workspace that are not there yet at that
   version, in dependency order (`libqi-macros-vibe`, then `libqi-vibe`), using the `CARGO_REGISTRY_TOKEN` secret of the
   `crates.io` environment. Crates already published at that version are skipped, so the
   workflow can be re-run and the first release can be published by hand.

## Every release

```sh
# 1. Bump the version (one number for the whole workspace) and the changelog.
sed -i 's/^version = ".*"/version = "0.2.0"/' Cargo.toml            # [workspace.package]
sed -i 's/version = "0.1.0" }/version = "0.2.0" }/' Cargo.toml       # the workspace crates
$EDITOR CHANGELOG.md                                                 # move Unreleased under ## [0.2.0] - date
cargo update --workspace                                             # refresh Cargo.lock
cargo test --workspace && cargo publish --workspace --dry-run
git commit -am "Release 0.2.0" && git push

# 2. Tag once the commit is on main and CI is green.
git tag -a v0.2.0 -m "libqi-rs-vibe 0.2.0"
git push origin v0.2.0
```

Then watch the "Release" workflow. A failure in the binaries or the GitHub release step can
be fixed and re-run from the Actions tab; the crates.io step never publishes twice.

## One-time setup

- **crates.io token.** Create an API token on <https://crates.io/settings/tokens> scoped to
  `publish-new` and `publish-update` (and, when the crates exist, restrict it to them).
  Store it as the secret `CARGO_REGISTRY_TOKEN` of a GitHub environment named `crates.io`
  (Settings, Environments): the environment lets you require a manual approval before the
  publish job runs.
- **Runners.** The matrix uses the `ubuntu-24.04-arm` and `macos-15-intel` hosted runners,
  free for public repositories. On a private repository, drop those two targets or replace
  them with cross-compilation.

## Publishing from a machine

The crates can always be published by hand, which is how the first release establishes
ownership under your crates.io account:

```sh
cargo login                              # paste a token with publish-new scope
cargo publish --workspace --dry-run      # packages, verifies, publishes nothing
cargo publish --workspace                # publishes both crates in dependency order
```

`cargo publish --workspace` needs Cargo 1.90 or later. Once the crates exist, add other
owners with `cargo owner --add <github-login> libqi-vibe` (and `libqi-macros-vibe`).
