#!/bin/sh
# Run the GNU patch 2.8 test suite against this project's patch binary.
# Prints one "passed/total" line. Every script in the upstream TESTS list
# counts toward total, including scripts that fail before any check runs.

set -u

root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
suite="$root/tests/gnu_patch_testsuite"

if [ -n "${PATCH:-}" ]; then
    patch_bin=$PATCH
elif [ -x "$root/target/debug/patch" ]; then
    patch_bin=$root/target/debug/patch
elif [ -x "$root/target/release/patch" ]; then
    patch_bin=$root/target/release/patch
else
    echo "run_gnu_testsuite: patch binary not found; build it or set PATCH" >&2
    echo "0/49"
    exit 1
fi

if [ ! -x "$patch_bin" ]; then
    echo "run_gnu_testsuite: cannot execute $patch_bin" >&2
    echo "0/49"
    exit 1
fi

# use_tmpdir writes under $abs_top_builddir/tests/. Keep that outside the
# source tree. srcdir is where the scripts source test-lib.sh.
build_dir=$(mktemp -d "${TMPDIR:-/tmp}/patch-rs-suite.XXXXXX")
mkdir -p "$build_dir/tests"
trap 'chmod -R u+rwx "$build_dir" 2>/dev/null; rm -rf "$build_dir"' EXIT INT TERM

export abs_top_builddir=$build_dir
export srcdir=$suite
export PATCH=$patch_bin

# Same order as tests/Makefile.am TESTS.
tests='
asymmetric-hunks
backup-prefix-suffix
bad-filenames
bad-usage
concat-git-diff
context-format
copy-rename
corrupt-patch
corrupt-reject-files
create-delete
create-directory
criss-cross
crlf-handling
dash-o-append
deep-directories
ed-style
empty-files
false-match
fifo
file-create-modes
file-modes
filename-choice
git-binary-diff
git-cleanup
garbage
global-reject-files
hardlinks
inname
line-numbers
merge
mangled-numbers-abort
mixed-patch-types
munged-context-format
need-filename
no-backup
no-mode-change-git-diff
no-newline-triggers-assert
preserve-c-function-names
preserve-mode-and-timestamp
quoted-filenames
read-only-files
regression-abe92e8010ab
reject-format
remember-backup-files
remember-reject-files
remove-directories
symlinks
unmodified-files
unusual-blanks
'

pass=0
fail=0
skip=0
total=0

# Seconds to let one script run before treating it as a start-up hang.
limit=${PATCH_TEST_TIMEOUT:-20}

# setsid makes the script a session and process-group leader, so a hang
# (a writer blocked on a fifo, for example) and its children can be killed
# together. Job control is not available here: there is no controlling tty.
run_script() {
    script=$1
    log=$2
    setsid /bin/sh "$script" >"$log" 2>&1 &
    pid=$!
    i=0
    ticks=$((limit * 10))
    while kill -0 "$pid" 2>/dev/null; do
        if [ "$i" -ge "$ticks" ]; then
            # /bin/kill, not the shell builtin: dash rejects "kill -- -pgid".
            /bin/kill -TERM -- "-$pid" 2>/dev/null || true
            sleep 1
            /bin/kill -KILL -- "-$pid" 2>/dev/null || true
            /bin/kill -KILL "$pid" 2>/dev/null || true
            wait "$pid" 2>/dev/null || true
            return 124
        fi
        sleep 0.1
        i=$((i + 1))
    done
    wait "$pid"
    ec=$?
    # The script can exit while a child is still blocked (for example a
    # redirection onto a fifo that nobody reads). That child stays in this
    # session; drop it so the next test starts clean.
    /bin/kill -KILL -- "-$pid" 2>/dev/null || true
    return "$ec"
}

for t in $tests; do
    total=$((total + 1))
    script=$suite/$t
    if [ ! -f "$script" ]; then
        echo "FAIL $t (startup: missing $script)"
        fail=$((fail + 1))
        continue
    fi
    # Always invoke a shell. A script that is not executable, or that the
    # shell rejects before the first check, is still one failed test.
    log=$(mktemp)
    if run_script "$script" "$log"; then
        echo "PASS $t"
        pass=$((pass + 1))
    else
        ec=$?
        if [ "$ec" -eq 77 ]; then
            echo "SKIP $t"
            skip=$((skip + 1))
        else
            echo "FAIL $t (exit $ec)"
            fail=$((fail + 1))
            if [ -n "${VERBOSE:-}" ]; then
                tail -n 30 "$log" | sed 's/^/    /'
            fi
        fi
    fi
    rm -f "$log"
done

# Skips are not passes. They stay in the total so a script that exits 77
# cannot hide, and startup failures stay in the total as well.
echo "$pass/$total"
echo "passed=$pass failed=$fail skipped=$skip total=$total" >&2

if [ "$pass" -eq "$total" ]; then
    exit 0
fi
exit 1
