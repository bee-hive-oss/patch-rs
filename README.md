# patch-rs

A faithful Rust port of [GNU patch](https://www.gnu.org/software/patch/) 2.8.
The program is meant to match GNU patch: the same messages, exit codes, and
files written. It does not add features beyond that port.

Build and run:

```sh
cargo build
./target/debug/patch --help
```

The GNU patch 2.8 test scripts live in `tests/gnu_patch_testsuite/` (see
`ORIGIN.md`). Score this binary with:

```sh
cargo build
tests/run_gnu_testsuite.sh
```

The script prints `passed/total`. A test that fails to start counts as a
failure in that total.

## License

GNU General Public License, version 3 or later. See `LICENSE`.

## Credits

Bee, Grok Bot, and Nori.
