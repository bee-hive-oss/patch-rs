# patch-rs scoreboard

How far the Rust port is, measured after merges to `main`. Rows are only ever
added at the bottom, never edited or removed, so the history stays readable.

Each row is measured from a clean clone in a rootless Podman image (Rust
1.100.0, GNU patch 2.8 built from GNU's checksummed tarball, no network):

- **Cargo tests**: unit tests passing in `cargo test`.
- **Clippy**: `cargo clippy --all-targets -- -D warnings`.
- **Self-check**: `tests/run_gnu_testsuite.sh` run against real GNU patch 2.8,
  which proves the yardstick. GNU patch 2.8 itself scores 47/49: upstream
  marks `dash-o-append` and `context-format` as expected failures
  (`XFAIL_TESTS` in `tests/gnu_patch_testsuite/Makefile.am`), and the
  script counts them as failures.
- **Starter cases**: patch-rs has no starter-case harness yet, so this is `n/a`.
- **GNU patch testsuite**: the official GNU patch 2.8 suite
  (`tests/run_gnu_testsuite.sh`) run against `target/release/patch`.

| Date (CT) | Commit | Cargo tests | Clippy | Self-check | Starter cases | GNU patch testsuite | Note |
|---|---|---|---|---|---|---|---|
| 2026-11-26 08:45 | `3c65e03` | 27 pass | 27 lints | 47/49 | n/a | 44/49 | initial port by Grok Build, pushed by Nori Nov 26; failing: context-format, dash-o-append (both upstream XFAIL), merge, preserve-mode-and-timestamp, symlinks; clippy 0.1.100 reports 22 collapsible_if, 2 manual contains, 3 sliced_string_as_bytes |
| 2026-11-26 08:55 | `1dc8fc5` | 27 pass | clean | 47/49 | n/a | 44/49 | PR #2 merged Nov 26: rustfmt pass and the 27 clippy 0.1.100 lints fixed, no behavior change |

## Upstream C bugs fixed

Bugs in the original C tool (the same pinned GNU version as the table
above) that this Rust port has fixed. A bug counts only when a test
fails on the real GNU tool and passes on the Rust port. Link the
upstream bug report when one exists. Rows are only ever added at the
bottom, never edited or removed.

Count so far: 0.

| Date (CT) | Commit | GNU version | Bug | Proof test | Upstream report |
|---|---|---|---|---|---|
