//! Faithful Rust port of GNU patch 2.8.
//!
//! This stub handles `--help` and `--version` only. Later commits fill in
//! option parsing and patch application.

fn program_name() -> String {
    std::env::args()
        .next()
        .unwrap_or_else(|| "patch".to_string())
}

fn version_text() -> &'static str {
    "\
GNU patch 2.8
Copyright 1989-2025 Free Software Foundation, Inc.
Copyright 1984-1988 Larry Wall

License GPLv3+: GNU GPL version 3 or later <http://gnu.org/licenses/gpl.html>.
This is free software: you are free to change and redistribute it.
There is NO WARRANTY, to the extent permitted by law.

Written by Larry Wall and Paul Eggert
"
}

fn help_body() -> &'static str {
    "\
Input options:

  -p NUM  --strip=NUM  Strip NUM leading components from file names.
  -F LINES  --fuzz LINES  Set the fuzz factor to LINES for inexact matching.
  -l  --ignore-whitespace  Ignore white space changes between patch and input.

  -c  --context  Interpret the patch as a context difference.
  -e  --ed  Interpret the patch as an ed script.
  -n  --normal  Interpret the patch as a normal difference.
  -u  --unified  Interpret the patch as a unified difference.

  -N  --forward  Ignore patches that appear to be reversed or already applied.
  -R  --reverse  Assume patches were created with old and new files swapped.

  -i PATCHFILE  --input=PATCHFILE  Read patch from PATCHFILE instead of stdin.

Output options:

  -o FILE  --output=FILE  Output patched files to FILE.
  -r FILE  --reject-file=FILE  Output rejects to FILE.

  -D NAME  --ifdef=NAME  Make merged if-then-else output using NAME.
  --merge  Merge using conflict markers instead of creating reject files.
  -E  --remove-empty-files  Remove output files that are empty after patching.

  -Z  --set-utc  Set times of patched files, assuming diff uses UTC (GMT).
  -T  --set-time  Likewise, assuming local time.

  --quoting-style=WORD   output file names using quoting style WORD.
    Valid WORDs are: literal, shell, shell-always, c, escape.
    Default is taken from QUOTING_STYLE env variable, or 'shell' if unset.

Backup and version control options:

  -b  --backup  Back up the original contents of each file.
  --backup-if-mismatch  Back up if the patch does not match exactly.
  --no-backup-if-mismatch  Back up mismatches only if otherwise requested.

  -V STYLE  --version-control=STYLE  Use STYLE version control.
	STYLE is either 'simple', 'numbered', or 'existing'.
  -B PREFIX  --prefix=PREFIX  Prepend PREFIX to backup file names.
  -Y PREFIX  --basename-prefix=PREFIX  Prepend PREFIX to backup file basenames.
  -z SUFFIX  --suffix=SUFFIX  Append SUFFIX to backup file names.

  -g NUM  --get=NUM  Get files from RCS etc. if positive; ask if negative.

Miscellaneous options:

  -t  --batch  Ask no questions; skip bad-Prereq patches; assume reversed.
  -f  --force  Like -t, but ignore bad-Prereq patches, and assume unreversed.
  -s  --quiet  --silent  Work silently unless an error occurs.
  --verbose  Output extra information about the work being done.
  --dry-run  Do not actually change any files; just print what would happen.
  --posix  Conform to the POSIX standard.

  -d DIR  --directory=DIR  Change the working directory to DIR first.
  --reject-format=FORMAT  Create 'context' or 'unified' rejects.
  --binary  Read and write data in binary mode.
  --read-only=BEHAVIOR  How to handle read-only input files: 'ignore' that they
                        are read-only, 'warn' (default), or 'fail'.

  -v  --version  Output version info.
  --help  Output this help.

Report bugs to <bug-patch@gnu.org>.
"
}

fn print_help(prog: &str) {
    println!("Usage: {prog} [OPTION]... [ORIGFILE [PATCHFILE]]\n");
    print!("{}", help_body());
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let prog = program_name();
    if args.iter().any(|a| a == "--help") {
        print_help(&prog);
        return;
    }
    if args.iter().any(|a| a == "--version" || a == "-v") {
        print!("{}", version_text());
        return;
    }
    // Not yet a patch implementation. Fail the way a missing operand does
    // only when asked for help-style usage; otherwise exit 2 so the suite
    // records a failure rather than a false pass.
    if args.len() > 1 {
        eprintln!("{prog}: Try '{prog} --help' for more information.");
        std::process::exit(2);
    }
    std::process::exit(2);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_matches_gnu_patch_2_8() {
        let text = version_text();
        assert!(text.starts_with("GNU patch 2.8\n"));
        assert!(text.contains("Written by Larry Wall and Paul Eggert\n"));
    }

    #[test]
    fn help_lists_merge_and_bug_address() {
        let body = help_body();
        assert!(body.contains("--merge"));
        assert!(body.contains("Report bugs to <bug-patch@gnu.org>.\n"));
    }
}
