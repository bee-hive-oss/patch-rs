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
| 2026-11-26 11:25 | `6353d53` | 28 pass | clean | 47/49 | n/a | 44/49 | PR #22 merged Nov 26: Reject absolute out-of-tree creates on reverse delete (CVE-2026-115207 / GNU patch 2.8) |

Coverage columns added 2026-11-26. New rows include C lines %, C branches %, Rust lines %, and Rust branches % from the GitHub coverage run on that commit (Rust figures are the GNU test suite alone). Earlier rows keep their original columns.

| Date (CT) | Commit | Cargo tests | Clippy | Self-check | Starter cases | GNU patch testsuite | C lines % | C branches % | Rust lines % | Rust branches % | Note |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 2026-11-26 16:42 | `7a9c511` | 28 pass | clean | 47/49 | n/a | 44/49 | 76.4% | 65.3% | 81.6% | 70.2% | pushed to main Nov 26: Opt patch-rs in to public check.sh on GitHub (repo-index.toml) |
| 2026-11-26 17:05 | `f2883d4` | 28 pass | clean | 47/49 | n/a | 44/49 | 76.4% | 65.3% | 81.6% | 70.2% | pushed to main Nov 26: Declare patch-rs's public test command in repo-index.toml |
| 2026-11-26 19:57 | `7b80169` | 28 pass | clean | RED | n/a | 44/49 | 76.4% | 65.3% | 81.6% | 70.2% | PR #35 merged Nov 26: Build the starter-case harness that compares patch-rs with GNU patch 2.8 |
| 2026-11-26 20:19 | `779173f` | 28 pass | clean | 47/49 | n/a | 44/49 | 76.4% | 65.3% | 81.6% | 70.2% | PR #38 merged Nov 26: Starter cases for normal diffs: change, add and delete |
| 2026-11-26 20:22 | `54bce0e` | 28 pass | clean | 47/49 | n/a | 44/49 | 76.4% | 65.3% | 81.6% | 70.2% | PR #39 merged Nov 26: Starter cases for ed-style diffs: change, append and delete |
| 2026-11-26 20:32 | `f94a520` | 28 pass | clean | RED | n/a | 44/49 | 76.4% | 65.3% | 81.6% | 70.2% | PR #41 merged Nov 26: Starter cases for fuzz: fuzz 1 applies and fuzz 0 rejects |
| 2026-11-26 20:38 | `225d043` | 28 pass | clean | 47/49 | n/a | 44/49 | 76.4% | 65.3% | 81.6% | 70.2% | PR #42 merged Nov 26: Starter cases for offsets: positive and negative line offsets |
| 2026-11-26 20:48 | `0559708` | 28 pass | clean | 47/49 | n/a | 45/49 | 76.4% | 65.3% | 81.1% | 69.2% | PR #44 merged Nov 26: Port --merge conflict output so the GNU merge script passes |
| 2026-11-26 20:54 | `10d9f96` | 28 pass | clean | RED | n/a | 46/49 | 76.4% | 65.3% | 80.6% | 67.8% | PR #46 merged Nov 26: Set file times for --set-time and --set-utc so the GNU preserve-mode-and-timestamp script passes |
| 2026-11-27 13:13 | `778d9f4` | 28 pass | clean | RED | n/a | 46/49 | 76.4% | 65.3% | 80.6% | 67.8% | PR #47 merged Nov 27: Switch GNU patch tarball fetch from ftp.gnu.org to ftpmirror.gnu.org |

## Upstream C bugs fixed

Bugs in the original C tool (the same pinned GNU version as the table
above) that this Rust port has fixed. A bug counts only when a test
fails on the real GNU tool and passes on the Rust port. Link the
upstream bug report when one exists. Rows are only ever added at the
bottom, never edited or removed.

Count so far: 0.

| Date (CT) | Commit | GNU version | Bug | Proof test | Upstream report |
|---|---|---|---|---|---|
