#!/usr/bin/env python3
"""Side-by-side byte-for-byte differential scoreboard against GNU patch."""
import argparse
import json
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile

def snapshot(root):
    """Every path under root: dirs, symlink targets, file bytes and modes.

    Symlinks are not followed.
    """
    out = {}

    def walk(dirpath):
        try:
            entries = list(os.scandir(dirpath))
        except OSError:
            return
        for entry in sorted(entries, key=lambda e: e.name):
            rel = str(pathlib.Path(entry.path).relative_to(root))
            if entry.is_symlink():
                out[rel] = ("symlink", os.readlink(entry.path))
            elif entry.is_dir(follow_symlinks=False):
                out[rel] = ("dir",)
                walk(entry.path)
            elif entry.is_file(follow_symlinks=False):
                mode = entry.stat(follow_symlinks=False).st_mode & 0o777
                with open(entry.path, "rb") as fh:
                    data = fh.read()
                out[rel] = ("file", mode, data)
            else:
                mode = entry.stat(follow_symlinks=False).st_mode
                out[rel] = ("other", mode)

    walk(root)
    return out


def run_bin(bin_path, args, stdin_bytes, workdir):
    env = dict(os.environ)
    env["LC_ALL"] = "C"
    env["TZ"] = "UTC0"
    exe = str(pathlib.Path(bin_path).resolve())
    timed_out = False
    try:
        proc = subprocess.run(
            ["patch", *args],
            executable=exe,
            input=stdin_bytes,
            cwd=workdir,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=10,
            env=env,
            preexec_fn=lambda: os.umask(0o022),
        )
        code, out, err = proc.returncode, proc.stdout, proc.stderr
    except subprocess.TimeoutExpired as exc:
        timed_out = True
        code = "timeout"
        out = exc.stdout or b""
        err = exc.stderr or b""
    files = snapshot(pathlib.Path(workdir))
    return timed_out, code, out, err, files


def prepare(workdir, setup_files):
    old = os.umask(0o022)
    try:
        for rel, content in setup_files.items():
            path = pathlib.Path(workdir) / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(content.encode("utf-8"))
    finally:
        os.umask(old)


def run_case(case_path, ref_bin, cand_bin):
    spec = json.loads(case_path.read_text(encoding="utf-8"))
    args = spec.get("args", [])
    stdin_bytes = spec.get("stdin", "").encode("utf-8")
    setup_files = spec.get("files", {})
    results = []
    for binary in (ref_bin, cand_bin):
        work = tempfile.mkdtemp(prefix="patch-case-")
        try:
            prepare(work, setup_files)
            results.append(run_bin(binary, args, stdin_bytes, work))
        finally:
            shutil.rmtree(work, ignore_errors=True)
    ref_r, cand_r = results
    diffs = []
    if ref_r[0] or cand_r[0]:
        diffs.append(f"timeout: ref={ref_r[0]} cand={cand_r[0]}")
    if ref_r[1] != cand_r[1]:
        diffs.append(f"exit: ref={ref_r[1]} cand={cand_r[1]}")
    if ref_r[2] != cand_r[2]:
        diffs.append(f"stdout: ref={ref_r[2]!r} cand={cand_r[2]!r}")
    if ref_r[3] != cand_r[3]:
        diffs.append(f"stderr: ref={ref_r[3]!r} cand={cand_r[3]!r}")
    if ref_r[4] != cand_r[4]:
        diffs.append(f"files: ref={ref_r[4]!r} cand={cand_r[4]!r}")
    return diffs


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--ref", default="/usr/bin/patch")
    ap.add_argument("--cand", required=True)
    ap.add_argument("--cases", default=str(pathlib.Path(__file__).parent / "cases"))
    ap.add_argument("--case", default=None)
    ns = ap.parse_args()
    cases_dir = pathlib.Path(ns.cases)
    cases = sorted(cases_dir.glob("*.json")) if cases_dir.is_dir() else []
    if ns.case:
        cases = [c for c in cases if c.stem == ns.case]
    passed = 0
    for case in cases:
        diffs = run_case(case, ns.ref, ns.cand)
        if not diffs:
            passed += 1
            print(f"PASS {case.stem}")
        else:
            print(f"FAIL {case.stem}: " + "; ".join(diffs))
    print(f"SCOREBOARD: {passed} / {len(cases)} passing")
    sys.exit(0 if passed == len(cases) else 1)


if __name__ == "__main__":
    main()
