#!/bin/sh
# Single entry point for patch-rs proofs.
#
# Runs, in order:
#   1) cargo test
#   2) cargo clippy --all-targets -- -D warnings
#   3) cargo build --release
#   4) tests/run_gnu_testsuite.sh against real GNU patch 2.8 (self-check:
#      exactly the upstream XFAIL_TESTS may fail)
#   5) tests/run_gnu_testsuite.sh against target/release/patch
#
# The port is not finished, so step 5 does not require 49/49. It requires
# no regressions: every test listed in tests/gnu_baseline_passing.txt must
# still pass. Fixing a test is reported but does not raise the bar on its
# own; add the test to the baseline file in the same PR (owner review).
#
# Usage:
#   ./check.sh              build patch-rs and prove it against the GNU suite
#   ./check.sh --self-check prove only the harness against GNU patch 2.8
#
# GNU_PATCH names the reference binary (default /usr/local/bin/patch, which
# must report GNU patch 2.8). If cargo is not on PATH, everything runs in a
# rootless podman container built from ./Containerfile. No root needed.

set -eu

ROOT=$(CDPATH= cd -- "$(dirname "$0")" && pwd)
cd "$ROOT"

SELF_CHECK=0
IN_CONTAINER=0
GNU_PATCH=${GNU_PATCH:-/usr/local/bin/patch}
GNU_PATCH_VERSION="GNU patch 2.8"
BASELINE="$ROOT/tests/gnu_baseline_passing.txt"
# tests/gnu_patch_testsuite/Makefile.am XFAIL_TESTS (non-Haiku): GNU patch
# 2.8 itself fails these.
UPSTREAM_XFAIL="context-format dash-o-append"

usage() {
	sed -n '2,23p' "$0" | sed 's/^# \{0,1\}//'
}

for arg in "$@"; do
	case "$arg" in
		--self-check) SELF_CHECK=1 ;;
		--in-container) IN_CONTAINER=1 ;;
		-h|--help) usage; exit 0 ;;
		*)
			echo "unknown option: $arg" >&2
			usage >&2
			exit 2
			;;
	esac
done

log() {
	printf '==> %s\n' "$*"
}

have_cmd() {
	command -v "$1" >/dev/null 2>&1
}

IMAGE_NAME="localhost/patch-rs-check:latest"

if ! have_cmd cargo && [ "$IN_CONTAINER" -eq 0 ]; then
	if ! have_cmd podman; then
		echo "cargo is not on PATH, and podman is not available to run ./Containerfile" >&2
		exit 2
	fi
	log "building rootless podman image ${IMAGE_NAME} from Containerfile"
	podman build -t "$IMAGE_NAME" -f "$ROOT/Containerfile" "$ROOT"
	set --
	[ "$SELF_CHECK" -eq 1 ] && set -- --self-check
	exec podman run --rm --network=none --userns=keep-id \
		-e CARGO_HOME=/tmp/cargo-home \
		-v "$ROOT:/work:rw" -w /work \
		"$IMAGE_NAME" ./check.sh --in-container "$@"
fi
if ! have_cmd cargo; then
	echo "cargo is not on PATH inside the container" >&2
	exit 2
fi

# Run the suite from an empty scratch directory: the upstream require
# checks create temp files in the current directory, and a read-only
# working directory would turn real tests into skips.
run_suite() {
	# $1 = patch binary, $2 = output file for PASS/FAIL lines.
	# Prints the lines, then sets TALLY to the final passed/total line.
	scratch=$(mktemp -d "${TMPDIR:-/tmp}/patch-rs-cwd.XXXXXX")
	rc=0
	(cd "$scratch" && PATCH=$1 "$ROOT/tests/run_gnu_testsuite.sh") >"$2" 2>&1 || rc=$?
	rm -rf "$scratch"
	cat "$2"
	TALLY=$(grep -E '^[0-9]+/[0-9]+$' "$2" | tail -n 1 || true)
	if [ -z "$TALLY" ]; then
		echo "run_gnu_testsuite.sh printed no passed/total line (exit $rc)" >&2
		exit 1
	fi
}

failing_tests() {
	sed -n 's/^\(FAIL\|SKIP\) \([^ ]*\).*/\2/p' "$1" | sort
}

self_check() {
	if [ ! -x "$GNU_PATCH" ] || ! "$GNU_PATCH" --version | head -n 1 | grep -qx "$GNU_PATCH_VERSION"; then
		echo "$GNU_PATCH is not $GNU_PATCH_VERSION; build it from GNU's tarball (see Containerfile)" >&2
		exit 2
	fi
	log "run_gnu_testsuite.sh --self-check against $GNU_PATCH ($GNU_PATCH_VERSION)"
	out=$(mktemp)
	run_suite "$GNU_PATCH" "$out"
	tally=$TALLY
	got=$(failing_tests "$out" | tr '\n' ' ' | sed 's/ $//')
	want=$(printf '%s\n' $UPSTREAM_XFAIL | sort | tr '\n' ' ' | sed 's/ $//')
	echo "GNU_PATCH_SELFCHECK: ${tally%/*} / ${tally#*/} passing"
	rm -f "$out"
	if [ "$got" != "$want" ]; then
		echo "self-check: GNU patch 2.8 should fail exactly [$want], got [$got]" >&2
		exit 1
	fi
}

port_check() {
	cand="$ROOT/target/release/patch"
	if [ ! -x "$cand" ]; then
		echo "patch-rs binary not found at target/release/patch" >&2
		exit 1
	fi
	if [ ! -f "$BASELINE" ]; then
		echo "missing $BASELINE" >&2
		exit 1
	fi
	log "run_gnu_testsuite.sh against $cand"
	out=$(mktemp)
	run_suite "$cand" "$out"
	tally=$TALLY
	pass=${tally%/*}
	total=${tally#*/}
	base=$(grep -cv '^[[:space:]]*\(#\|$\)' "$BASELINE" || true)
	sed -n 's/^PASS \([^ ]*\)$/\1/p' "$out" | sort >"$out.pass"
	grep -v '^[[:space:]]*\(#\|$\)' "$BASELINE" | sort >"$out.base"
	regressed=$(comm -23 "$out.base" "$out.pass" | tr '\n' ' ' | sed 's/ $//')
	fixed=$(comm -13 "$out.base" "$out.pass" | tr '\n' ' ' | sed 's/ $//')
	rm -f "$out" "$out.pass" "$out.base"
	echo "GNU_PATCH_TESTSUITE: $pass / $total passing"
	echo "GNU_PATCH_BASELINE: $base / $total required"
	if [ -n "$fixed" ]; then
		echo "newly passing, not yet in the baseline: $fixed"
	fi
	if [ -n "$regressed" ]; then
		echo "regressions against tests/gnu_baseline_passing.txt: $regressed" >&2
		exit 1
	fi
}

log "patch-rs check (self_check=${SELF_CHECK}) in $ROOT"
log "cargo test"
cargo test
log "cargo clippy --all-targets -- -D warnings"
cargo clippy --all-targets -- -D warnings
if [ "$SELF_CHECK" -eq 0 ]; then
	log "cargo build --release"
	cargo build --release
fi
self_check
if [ "$SELF_CHECK" -eq 0 ]; then
	port_check
fi
log "all checks passed"
