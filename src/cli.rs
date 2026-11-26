//! Command-line parsing for GNU patch 2.8.
//!
//! Long and short options follow GNU `getopt_long`: options may follow
//! operands unless `POSIXLY_CORRECT` is set, `--` ends options, and long
//! options accept unambiguous prefixes.

use crate::quote::{self, ArgMatch, QuotingStyle};
use std::env;
use std::io::{self, Write};
use std::process::exit;

pub const EXIT_FAILURE: i32 = 1;
pub const EXIT_TROUBLE: i32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verbosity {
    Default,
    Silent,
    Verbose,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadOnly {
    Ignore,
    Warn,
    Fail,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectFormat {
    Unspecified,
    Context,
    Unified,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForcedDiff {
    Unspecified,
    Context,
    Ed,
    Normal,
    Unified,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackupType {
    /// `none` / `off`: do not create backup files.
    None,
    /// `simple` / `never`.
    Simple,
    /// `numbered` / `t`.
    Numbered,
    /// `existing` / `nil`, and the default when version control is unset.
    Existing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictStyle {
    Merge,
    Diff3,
}

#[derive(Clone, Debug)]
pub struct Options {
    pub program: String,
    pub strip: i64,
    pub fuzz: i64,
    pub ignore_whitespace: bool,
    pub forced_diff: ForcedDiff,
    pub forward: bool,
    pub reverse: bool,
    pub input: Option<String>,
    pub output: Option<String>,
    pub reject_file: Option<String>,
    pub ifdef: Option<String>,
    pub merge: bool,
    pub conflict_style: ConflictStyle,
    pub remove_empty: bool,
    pub set_utc: bool,
    pub set_time: bool,
    pub quoting: QuotingStyle,
    pub backup: bool,
    pub backup_if_mismatch: bool,
    pub backup_if_mismatch_specified: bool,
    pub version_control: Option<String>,
    pub version_control_context: String,
    pub backup_prefix: Option<String>,
    pub backup_basename_prefix: Option<String>,
    pub backup_suffix: Option<String>,
    pub simple_backup_suffix: String,
    pub backup_type: BackupType,
    pub patch_get: i64,
    pub batch: bool,
    pub force: bool,
    pub verbosity: Verbosity,
    pub dry_run: bool,
    pub posix: bool,
    pub reject_format: RejectFormat,
    pub binary: bool,
    pub read_only: ReadOnly,
    pub follow_symlinks: bool,
    pub debug: u16,
    pub original: Option<String>,
    pub patch_file: Option<String>,
}

impl Options {
    pub fn quote(&self, text: &str) -> String {
        quote::quote(self.quoting, text)
    }
}

struct LongOpt {
    name: &'static str,
    arg: ArgKind,
    code: Opt,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ArgKind {
    None,
    Required,
    Optional,
}

#[derive(Clone, Copy)]
enum Opt {
    Backup,
    Prefix,
    Context,
    Directory,
    Ifdef,
    Ed,
    RemoveEmpty,
    Force,
    Fuzz,
    Get,
    Input,
    IgnoreWs,
    Merge,
    Normal,
    Forward,
    Output,
    Strip,
    RejectFile,
    Reverse,
    Quiet,
    Batch,
    SetTime,
    Unified,
    Version,
    VersionControl,
    Debug,
    BasenamePrefix,
    Suffix,
    SetUtc,
    DryRun,
    Verbose,
    Binary,
    Help,
    BackupIfMismatch,
    NoBackupIfMismatch,
    Posix,
    QuotingStyle,
    RejectFormat,
    ReadOnly,
    FollowSymlinks,
}

fn long_opts() -> &'static [LongOpt] {
    &[
        LongOpt {
            name: "backup",
            arg: ArgKind::None,
            code: Opt::Backup,
        },
        LongOpt {
            name: "prefix",
            arg: ArgKind::Required,
            code: Opt::Prefix,
        },
        LongOpt {
            name: "context",
            arg: ArgKind::None,
            code: Opt::Context,
        },
        LongOpt {
            name: "directory",
            arg: ArgKind::Required,
            code: Opt::Directory,
        },
        LongOpt {
            name: "ifdef",
            arg: ArgKind::Required,
            code: Opt::Ifdef,
        },
        LongOpt {
            name: "ed",
            arg: ArgKind::None,
            code: Opt::Ed,
        },
        LongOpt {
            name: "remove-empty-files",
            arg: ArgKind::None,
            code: Opt::RemoveEmpty,
        },
        LongOpt {
            name: "force",
            arg: ArgKind::None,
            code: Opt::Force,
        },
        LongOpt {
            name: "fuzz",
            arg: ArgKind::Required,
            code: Opt::Fuzz,
        },
        LongOpt {
            name: "get",
            arg: ArgKind::Required,
            code: Opt::Get,
        },
        LongOpt {
            name: "input",
            arg: ArgKind::Required,
            code: Opt::Input,
        },
        LongOpt {
            name: "ignore-whitespace",
            arg: ArgKind::None,
            code: Opt::IgnoreWs,
        },
        LongOpt {
            name: "merge",
            arg: ArgKind::Optional,
            code: Opt::Merge,
        },
        LongOpt {
            name: "normal",
            arg: ArgKind::None,
            code: Opt::Normal,
        },
        LongOpt {
            name: "forward",
            arg: ArgKind::None,
            code: Opt::Forward,
        },
        LongOpt {
            name: "output",
            arg: ArgKind::Required,
            code: Opt::Output,
        },
        LongOpt {
            name: "strip",
            arg: ArgKind::Required,
            code: Opt::Strip,
        },
        LongOpt {
            name: "reject-file",
            arg: ArgKind::Required,
            code: Opt::RejectFile,
        },
        LongOpt {
            name: "reverse",
            arg: ArgKind::None,
            code: Opt::Reverse,
        },
        LongOpt {
            name: "quiet",
            arg: ArgKind::None,
            code: Opt::Quiet,
        },
        LongOpt {
            name: "silent",
            arg: ArgKind::None,
            code: Opt::Quiet,
        },
        LongOpt {
            name: "batch",
            arg: ArgKind::None,
            code: Opt::Batch,
        },
        LongOpt {
            name: "set-time",
            arg: ArgKind::None,
            code: Opt::SetTime,
        },
        LongOpt {
            name: "unified",
            arg: ArgKind::None,
            code: Opt::Unified,
        },
        LongOpt {
            name: "version",
            arg: ArgKind::None,
            code: Opt::Version,
        },
        LongOpt {
            name: "version-control",
            arg: ArgKind::Required,
            code: Opt::VersionControl,
        },
        LongOpt {
            name: "debug",
            arg: ArgKind::Required,
            code: Opt::Debug,
        },
        LongOpt {
            name: "basename-prefix",
            arg: ArgKind::Required,
            code: Opt::BasenamePrefix,
        },
        LongOpt {
            name: "suffix",
            arg: ArgKind::Required,
            code: Opt::Suffix,
        },
        LongOpt {
            name: "set-utc",
            arg: ArgKind::None,
            code: Opt::SetUtc,
        },
        LongOpt {
            name: "dry-run",
            arg: ArgKind::None,
            code: Opt::DryRun,
        },
        LongOpt {
            name: "verbose",
            arg: ArgKind::None,
            code: Opt::Verbose,
        },
        LongOpt {
            name: "binary",
            arg: ArgKind::None,
            code: Opt::Binary,
        },
        LongOpt {
            name: "help",
            arg: ArgKind::None,
            code: Opt::Help,
        },
        LongOpt {
            name: "backup-if-mismatch",
            arg: ArgKind::None,
            code: Opt::BackupIfMismatch,
        },
        LongOpt {
            name: "no-backup-if-mismatch",
            arg: ArgKind::None,
            code: Opt::NoBackupIfMismatch,
        },
        LongOpt {
            name: "posix",
            arg: ArgKind::None,
            code: Opt::Posix,
        },
        LongOpt {
            name: "quoting-style",
            arg: ArgKind::Required,
            code: Opt::QuotingStyle,
        },
        LongOpt {
            name: "reject-format",
            arg: ArgKind::Required,
            code: Opt::RejectFormat,
        },
        LongOpt {
            name: "read-only",
            arg: ArgKind::Required,
            code: Opt::ReadOnly,
        },
        LongOpt {
            name: "follow-symlinks",
            arg: ArgKind::None,
            code: Opt::FollowSymlinks,
        },
    ]
}

#[derive(Debug)]
pub enum EarlyExit {
    Help,
    Version,
}

#[derive(Debug)]
pub struct CliFailure {
    pub message: Option<String>,
    pub usage: bool,
}

pub fn short_name(program: &str) -> &str {
    program.rsplit('/').next().unwrap_or(program)
}

pub fn help_text(program: &str) -> String {
    format!(
        "\
Usage: {program} [OPTION]... [ORIGFILE [PATCHFILE]]

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
    )
}

pub fn version_text() -> &'static str {
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

/// Environment variables GNU patch reads before it parses options.
///
/// Tests build this directly. The program reads it from the process
/// environment so unit tests do not have to change global state.
#[derive(Clone, Debug)]
pub struct ProcessEnv {
    pub posixly_correct: bool,
    pub quoting_style: Option<String>,
    pub patch_get: Option<String>,
    pub simple_backup_suffix: Option<String>,
    pub patch_version_control: Option<String>,
    pub version_control: Option<String>,
}

impl ProcessEnv {
    pub fn from_process() -> Self {
        Self {
            posixly_correct: env::var_os("POSIXLY_CORRECT").is_some(),
            quoting_style: env::var("QUOTING_STYLE").ok(),
            patch_get: env::var("PATCH_GET").ok(),
            simple_backup_suffix: env::var("SIMPLE_BACKUP_SUFFIX").ok(),
            patch_version_control: env::var("PATCH_VERSION_CONTROL").ok(),
            version_control: env::var("VERSION_CONTROL").ok(),
        }
    }
}

/// Parse `argv` (including argv[0]) the way GNU patch does.
///
/// On `-d`, this changes the process working directory, matching GNU patch.
pub fn parse(argv: &[String]) -> Result<Options, ParseStop> {
    parse_with(argv, &ProcessEnv::from_process())
}

pub fn parse_with(argv: &[String], proc_env: &ProcessEnv) -> Result<Options, ParseStop> {
    let program = argv.first().cloned().unwrap_or_else(|| "patch".to_string());
    // `POSIXLY_CORRECT` stops getopt permutation. `--posix` does not.
    let posix_env = proc_env.posixly_correct;
    let quoting = match proc_env.quoting_style.as_deref() {
        Some(value) => QuotingStyle::parse(value).unwrap_or(QuotingStyle::Shell),
        None => QuotingStyle::Shell,
    };
    let patch_get = match proc_env.patch_get.as_deref() {
        Some(value) => numeric_string(&program, &quoting, value, true, "PATCH_GET value")?,
        None => 0,
    };
    let simple_backup_suffix = match proc_env.simple_backup_suffix.as_deref() {
        Some(value) if !value.is_empty() => value.to_string(),
        _ => ".orig".to_string(),
    };
    let (version_control, version_control_context) =
        if let Some(value) = proc_env.patch_version_control.clone() {
            (Some(value), "$PATCH_VERSION_CONTROL".to_string())
        } else if let Some(value) = proc_env.version_control.clone() {
            (Some(value), "$VERSION_CONTROL".to_string())
        } else {
            (None, String::new())
        };

    let mut opt = Options {
        program: program.clone(),
        strip: -1,
        fuzz: 2,
        ignore_whitespace: false,
        forced_diff: ForcedDiff::Unspecified,
        forward: false,
        reverse: false,
        input: None,
        output: None,
        reject_file: None,
        ifdef: None,
        merge: false,
        conflict_style: ConflictStyle::Merge,
        remove_empty: false,
        set_utc: false,
        set_time: false,
        quoting,
        backup: false,
        backup_if_mismatch: false,
        backup_if_mismatch_specified: false,
        version_control,
        version_control_context,
        backup_prefix: None,
        backup_basename_prefix: None,
        backup_suffix: None,
        simple_backup_suffix,
        backup_type: BackupType::Existing,
        patch_get,
        batch: false,
        force: false,
        verbosity: Verbosity::Default,
        dry_run: false,
        posix: posix_env,
        reject_format: RejectFormat::Unspecified,
        binary: false,
        read_only: ReadOnly::Warn,
        follow_symlinks: false,
        debug: 0,
        original: None,
        patch_file: None,
    };

    let mut operands: Vec<String> = Vec::new();
    let mut index = 1usize;
    let mut ended = false;
    while index < argv.len() {
        let arg = &argv[index];
        if ended || arg == "-" || !arg.starts_with('-') {
            if posix_env || ended {
                operands.extend(argv[index..].iter().cloned());
                break;
            }
            operands.push(arg.clone());
            index += 1;
            continue;
        }
        if arg == "--" {
            ended = true;
            index += 1;
            continue;
        }
        if let Some(rest) = arg.strip_prefix("--") {
            let (name, inline) = match rest.split_once('=') {
                Some((name, value)) => (name.to_string(), Some(value.to_string())),
                None => (rest.to_string(), None),
            };
            let matches: Vec<&LongOpt> = long_opts()
                .iter()
                .filter(|item| item.name.starts_with(&name))
                .collect();
            let chosen = if matches.iter().any(|item| item.name == name) {
                matches.into_iter().find(|item| item.name == name)
            } else if matches.len() == 1 {
                matches.into_iter().next()
            } else if matches.is_empty() {
                return Err(fail(
                    &program,
                    Some(format!("unrecognized option '{arg}'")),
                    true,
                ));
            } else {
                let mut list = String::new();
                for item in matches {
                    list.push_str(" '--");
                    list.push_str(item.name);
                    list.push('\'');
                }
                return Err(fail(
                    &program,
                    Some(format!("option '{arg}' is ambiguous; possibilities:{list}")),
                    true,
                ));
            };
            let chosen = chosen.expect("match selected");
            let value = match chosen.arg {
                ArgKind::None => {
                    if inline.is_some() {
                        return Err(fail(
                            &program,
                            Some(format!(
                                "option '--{}' doesn't allow an argument",
                                chosen.name
                            )),
                            true,
                        ));
                    }
                    None
                }
                ArgKind::Required => {
                    if let Some(value) = inline {
                        Some(value)
                    } else {
                        index += 1;
                        if index >= argv.len() {
                            return Err(fail(
                                &program,
                                Some(format!("option '--{}' requires an argument", chosen.name)),
                                true,
                            ));
                        }
                        Some(argv[index].clone())
                    }
                }
                ArgKind::Optional => inline,
            };
            apply_opt(&mut opt, chosen.code, value.as_deref(), false)?;
            index += 1;
            continue;
        }
        // Short cluster.
        let chars: Vec<char> = arg.chars().skip(1).collect();
        let mut cursor = 0usize;
        while cursor < chars.len() {
            let ch = chars[cursor];
            let code = match short_opt(ch) {
                Some(code) => code,
                None => {
                    return Err(fail(
                        &program,
                        Some(format!("invalid option -- '{ch}'")),
                        true,
                    ));
                }
            };
            let kind = short_arg_kind(code);
            let value = match kind {
                ArgKind::None => {
                    cursor += 1;
                    None
                }
                ArgKind::Required => {
                    let rest: String = chars[cursor + 1..].iter().collect();
                    if rest.is_empty() {
                        index += 1;
                        if index >= argv.len() {
                            return Err(fail(
                                &program,
                                Some(format!("option requires an argument -- '{ch}'")),
                                true,
                            ));
                        }
                        cursor = chars.len();
                        Some(argv[index].clone())
                    } else {
                        cursor = chars.len();
                        Some(rest)
                    }
                }
                ArgKind::Optional => None,
            };
            // The obsolete `-b SUFFIX ORIGFILE PATCHFILE` form.
            if matches!(code, Opt::Backup) && arg == "-b" && cursor >= chars.len() {
                let rest = &argv[index + 1..];
                if rest.len() == 3 && rest.iter().all(|item| !looks_like_option(item)) {
                    let suffix = rest[0].clone();
                    if opt.verbosity != Verbosity::Silent {
                        println!(
                            "warning: the '-b {suffix}' option is obsolete; use '-b -z {suffix}' instead"
                        );
                    }
                    opt.backup = true;
                    require_backup_piece(&opt.program, "suffix", &suffix)?;
                    opt.backup_suffix = Some(suffix);
                    operands.push(rest[1].clone());
                    operands.push(rest[2].clone());
                    index = argv.len();
                    break;
                }
            }
            apply_opt(&mut opt, code, value.as_deref(), true)?;
            if cursor >= chars.len() {
                break;
            }
        }
        index += 1;
    }

    if operands.len() > 2 {
        let extra = opt.quote(&operands[2]);
        return Err(fail(
            &program,
            Some(format!("{extra}: extra operand")),
            true,
        ));
    }
    if let Some(name) = operands.first() {
        opt.original = Some(name.clone());
    }
    if operands.len() == 2 {
        opt.patch_file = Some(operands[1].clone());
    }
    if !opt.backup_if_mismatch_specified {
        opt.backup_if_mismatch = !opt.posix;
    }
    if opt.backup || opt.backup_if_mismatch {
        opt.backup_type = backup_type_from(
            &opt.program,
            &opt.version_control_context,
            opt.version_control.as_deref(),
        )?;
    }
    Ok(opt)
}

fn looks_like_option(arg: &str) -> bool {
    arg.starts_with('-') && arg.len() > 1
}

fn short_opt(ch: char) -> Option<Opt> {
    Some(match ch {
        'b' => Opt::Backup,
        'B' => Opt::Prefix,
        'c' => Opt::Context,
        'd' => Opt::Directory,
        'D' => Opt::Ifdef,
        'e' => Opt::Ed,
        'E' => Opt::RemoveEmpty,
        'f' => Opt::Force,
        'F' => Opt::Fuzz,
        'g' => Opt::Get,
        'i' => Opt::Input,
        'l' => Opt::IgnoreWs,
        'n' => Opt::Normal,
        'N' => Opt::Forward,
        'o' => Opt::Output,
        'p' => Opt::Strip,
        'r' => Opt::RejectFile,
        'R' => Opt::Reverse,
        's' => Opt::Quiet,
        't' => Opt::Batch,
        'T' => Opt::SetTime,
        'u' => Opt::Unified,
        'v' => Opt::Version,
        'V' => Opt::VersionControl,
        'x' => Opt::Debug,
        'Y' => Opt::BasenamePrefix,
        'z' => Opt::Suffix,
        'Z' => Opt::SetUtc,
        _ => return None,
    })
}

fn short_arg_kind(code: Opt) -> ArgKind {
    match code {
        Opt::Prefix
        | Opt::Directory
        | Opt::Ifdef
        | Opt::Fuzz
        | Opt::Get
        | Opt::Input
        | Opt::Output
        | Opt::Strip
        | Opt::RejectFile
        | Opt::VersionControl
        | Opt::Debug
        | Opt::BasenamePrefix
        | Opt::Suffix => ArgKind::Required,
        _ => ArgKind::None,
    }
}

fn apply_opt(
    opt: &mut Options,
    code: Opt,
    value: Option<&str>,
    _short: bool,
) -> Result<(), ParseStop> {
    match code {
        Opt::Backup => opt.backup = true,
        Opt::Prefix => {
            let value = value.unwrap_or("");
            require_backup_piece(&opt.program, "prefix", value)?;
            opt.backup_prefix = Some(value.to_string());
        }
        Opt::Context => opt.forced_diff = ForcedDiff::Context,
        Opt::Directory => {
            let dir = value.unwrap_or("");
            if let Err(err) = env::set_current_dir(dir) {
                let quoted = opt.quote(dir);
                return Err(ParseStop::Trouble(pfatal_message(
                    &opt.program,
                    &format!("Can't change to directory {quoted}"),
                    &err,
                )));
            }
        }
        Opt::Ifdef => opt.ifdef = Some(value.unwrap_or("").to_string()),
        Opt::Ed => opt.forced_diff = ForcedDiff::Ed,
        Opt::RemoveEmpty => opt.remove_empty = true,
        Opt::Force => opt.force = true,
        Opt::Fuzz => {
            opt.fuzz = numeric_string(
                &opt.program,
                &opt.quoting,
                value.unwrap_or(""),
                false,
                "fuzz factor",
            )?;
        }
        Opt::Get => {
            opt.patch_get = numeric_string(
                &opt.program,
                &opt.quoting,
                value.unwrap_or(""),
                true,
                "get option value",
            )?;
        }
        Opt::Input => opt.input = Some(value.unwrap_or("").to_string()),
        Opt::IgnoreWs => opt.ignore_whitespace = true,
        Opt::Merge => {
            opt.merge = true;
            opt.conflict_style = match value {
                None | Some("merge") => ConflictStyle::Merge,
                Some("diff3") => ConflictStyle::Diff3,
                Some(_) => return Err(fail(&opt.program, None, true)),
            };
        }
        Opt::Normal => opt.forced_diff = ForcedDiff::Normal,
        Opt::Forward => opt.forward = true,
        Opt::Output => opt.output = Some(value.unwrap_or("").to_string()),
        Opt::Strip => {
            opt.strip = numeric_string(
                &opt.program,
                &opt.quoting,
                value.unwrap_or(""),
                false,
                "strip count",
            )?;
        }
        Opt::RejectFile => opt.reject_file = Some(value.unwrap_or("").to_string()),
        Opt::Reverse => opt.reverse = true,
        Opt::Quiet => opt.verbosity = Verbosity::Silent,
        Opt::Batch => opt.batch = true,
        Opt::SetTime => opt.set_time = true,
        Opt::Unified => opt.forced_diff = ForcedDiff::Unified,
        Opt::Version => return Err(ParseStop::Early(EarlyExit::Version)),
        Opt::VersionControl => {
            opt.version_control = Some(value.unwrap_or("").to_string());
            opt.version_control_context = "--version-control or -V option".to_string();
        }
        Opt::Debug => {
            let number = numeric_string(
                &opt.program,
                &opt.quoting,
                value.unwrap_or(""),
                true,
                "debugging option",
            )?;
            opt.debug = number.clamp(0, i64::from(u16::MAX)) as u16;
        }
        Opt::BasenamePrefix => {
            let value = value.unwrap_or("");
            require_backup_piece(&opt.program, "basename prefix", value)?;
            opt.backup_basename_prefix = Some(value.to_string());
        }
        Opt::Suffix => {
            let value = value.unwrap_or("");
            require_backup_piece(&opt.program, "suffix", value)?;
            opt.backup_suffix = Some(value.to_string());
        }
        Opt::SetUtc => opt.set_utc = true,
        Opt::DryRun => opt.dry_run = true,
        Opt::Verbose => opt.verbosity = Verbosity::Verbose,
        Opt::Binary => opt.binary = true,
        Opt::Help => return Err(ParseStop::Early(EarlyExit::Help)),
        Opt::BackupIfMismatch => {
            opt.backup_if_mismatch = true;
            opt.backup_if_mismatch_specified = true;
        }
        Opt::NoBackupIfMismatch => {
            opt.backup_if_mismatch = false;
            opt.backup_if_mismatch_specified = true;
        }
        Opt::Posix => opt.posix = true,
        Opt::QuotingStyle => {
            let value = value.unwrap_or("");
            match QuotingStyle::parse(value) {
                Ok(style) => opt.quoting = style,
                Err(problem) => {
                    let which = match problem {
                        ArgMatch::Invalid => "invalid",
                        ArgMatch::Ambiguous => "ambiguous",
                    };
                    let shown = quote::quote(QuotingStyle::Locale, value);
                    let context = quote::quote(QuotingStyle::Locale, "quoting style");
                    let short = short_name(&opt.program);
                    return Err(fail(
                        &opt.program,
                        Some(format!("{short}: {which} argument {shown} for {context}")),
                        true,
                    ));
                }
            }
        }
        Opt::RejectFormat => match value.unwrap_or("") {
            "context" => opt.reject_format = RejectFormat::Context,
            "unified" => opt.reject_format = RejectFormat::Unified,
            _ => return Err(fail(&opt.program, None, true)),
        },
        Opt::ReadOnly => match value.unwrap_or("") {
            "ignore" => opt.read_only = ReadOnly::Ignore,
            "warn" => opt.read_only = ReadOnly::Warn,
            "fail" => opt.read_only = ReadOnly::Fail,
            _ => return Err(fail(&opt.program, None, true)),
        },
        Opt::FollowSymlinks => opt.follow_symlinks = true,
    }
    Ok(())
}

fn require_backup_piece(program: &str, kind: &str, value: &str) -> Result<(), ParseStop> {
    if value.is_empty() {
        return Err(ParseStop::Trouble(format!(
            "{program}: **** backup {kind} is empty\n"
        )));
    }
    Ok(())
}

pub fn numeric_string(
    program: &str,
    quoting: &QuotingStyle,
    text: &str,
    negative_allowed: bool,
    what: &str,
) -> Result<i64, ParseStop> {
    let quoted = quote::quote(*quoting, text);
    let mut chars = text.chars().peekable();
    let negative = matches!(chars.peek(), Some('-'));
    if matches!(chars.peek(), Some('-' | '+')) {
        chars.next();
    }
    let digits: String = chars.collect();
    if digits.is_empty() || !digits.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(ParseStop::Trouble(format!(
            "{program}: **** {what} {quoted} is not a number\n"
        )));
    }
    let mut value: i64 = 0;
    let mut overflow = false;
    for ch in digits.chars() {
        let digit = i64::from(ch as u8 - b'0');
        overflow |= value.checked_mul(10).is_none();
        value = value.saturating_mul(10);
        if negative {
            overflow |= value.checked_sub(digit).is_none();
            value = value.saturating_sub(digit);
        } else {
            overflow |= value.checked_add(digit).is_none();
            value = value.saturating_add(digit);
        }
    }
    if value < 0 && !negative_allowed {
        return Err(ParseStop::Trouble(format!(
            "{program}: **** {what} {quoted} is negative\n"
        )));
    }
    if overflow {
        Ok(if negative { i64::MIN } else { i64::MAX })
    } else {
        Ok(value)
    }
}

#[derive(Debug)]
pub enum ParseStop {
    Early(EarlyExit),
    Trouble(String),
    Usage(String),
}

fn fail(program: &str, message: Option<String>, usage: bool) -> ParseStop {
    let mut text = String::new();
    if let Some(message) = message {
        // argmatch messages already include the short program name.
        if message.contains(": invalid argument ") || message.contains(": ambiguous argument ") {
            text.push_str(&message);
            text.push('\n');
        } else {
            text.push_str(program);
            text.push_str(": ");
            text.push_str(&message);
            text.push('\n');
        }
    }
    if usage {
        text.push_str(program);
        text.push_str(": Try '");
        text.push_str(program);
        text.push_str(" --help' for more information.\n");
        return ParseStop::Usage(text);
    }
    ParseStop::Trouble(text)
}

fn pfatal_message(program: &str, message: &str, err: &io::Error) -> String {
    format!("{program}: **** {message} : {}\n", io_error_text(err))
}

/// GNU `pfatal` prints `strerror(errno)`. Rust appends ` (os error N)`.
pub(crate) fn io_error_text(err: &io::Error) -> String {
    // EILSEQ. GNU patch special-cases this instead of using strerror.
    if err.raw_os_error() == Some(84) {
        return "Invalid byte sequence".to_string();
    }
    let text = err.to_string();
    if let Some(code) = err.raw_os_error() {
        let suffix = format!(" (os error {code})");
        if let Some(stripped) = text.strip_suffix(&suffix) {
            return stripped.to_string();
        }
    }
    text
}

pub fn report(stop: ParseStop) -> ! {
    match stop {
        ParseStop::Early(EarlyExit::Help) => {
            let program = env::args().next().unwrap_or_else(|| "patch".to_string());
            print!("{}", help_text(&program));
            exit(0);
        }
        ParseStop::Early(EarlyExit::Version) => {
            print!("{}", version_text());
            exit(0);
        }
        ParseStop::Trouble(text) | ParseStop::Usage(text) => {
            let mut err = io::stderr().lock();
            let _ = err.write_all(text.as_bytes());
            exit(EXIT_TROUBLE);
        }
    }
}

fn backup_type_from(
    program: &str,
    context: &str,
    version: Option<&str>,
) -> Result<BackupType, ParseStop> {
    match version {
        None | Some("") => Ok(BackupType::Existing),
        Some(text) => match backup_word(text) {
            Ok(kind) => Ok(kind),
            Err(problem) => {
                let which = match problem {
                    ArgMatch::Invalid => "invalid",
                    ArgMatch::Ambiguous => "ambiguous",
                };
                let shown = quote::quote(QuotingStyle::Locale, text);
                let ctx = if context.is_empty() {
                    "version control"
                } else {
                    context
                };
                let quoted_ctx = quote::quote(QuotingStyle::Locale, ctx);
                let short = short_name(program);
                let mut message = format!(
                    "{short}: {which} argument {shown} for {quoted_ctx}\nValid arguments are:\n",
                );
                message.push_str("  - 'none', 'off'\n");
                message.push_str("  - 'simple', 'never'\n");
                message.push_str("  - 'existing', 'nil'\n");
                message.push_str("  - 'numbered', 't'\n");
                Err(ParseStop::Trouble(message))
            }
        },
    }
}

fn backup_word(text: &str) -> Result<BackupType, ArgMatch> {
    quote::argmatch(
        text,
        &[
            ("none", BackupType::None),
            ("off", BackupType::None),
            ("simple", BackupType::Simple),
            ("never", BackupType::Simple),
            ("existing", BackupType::Existing),
            ("nil", BackupType::Existing),
            ("numbered", BackupType::Numbered),
            ("t", BackupType::Numbered),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    fn trouble(items: &[&str]) -> String {
        match parse(&argv(items)) {
            Err(ParseStop::Trouble(text) | ParseStop::Usage(text)) => text,
            other => panic!("expected failure, got {other:?}"),
        }
    }

    #[test]
    fn strip_count_not_a_number() {
        let text = trouble(&["/usr/bin/patch", "-px"]);
        assert_eq!(text, "/usr/bin/patch: **** strip count x is not a number\n");
    }

    #[test]
    fn strip_count_negative() {
        let text = trouble(&["patch", "-p", "-1"]);
        assert_eq!(text, "patch: **** strip count -1 is negative\n");
    }

    #[test]
    fn unknown_long_and_short() {
        let text = trouble(&["/usr/bin/patch", "--foo"]);
        assert_eq!(
            text,
            "\
/usr/bin/patch: unrecognized option '--foo'
/usr/bin/patch: Try '/usr/bin/patch --help' for more information.\n"
        );
        let text = trouble(&["/usr/bin/patch", "-Q"]);
        assert_eq!(
            text,
            "\
/usr/bin/patch: invalid option -- 'Q'
/usr/bin/patch: Try '/usr/bin/patch --help' for more information.\n"
        );
    }

    #[test]
    fn missing_argument_messages() {
        let text = trouble(&["/usr/bin/patch", "--strip"]);
        assert!(text.starts_with("/usr/bin/patch: option '--strip' requires an argument\n"));
        let text = trouble(&["/usr/bin/patch", "-p"]);
        assert!(text.starts_with("/usr/bin/patch: option requires an argument -- 'p'\n"));
    }

    #[test]
    fn ambiguous_backup_prefix() {
        let text = trouble(&["/usr/bin/patch", "--ba"]);
        assert!(text.contains("option '--ba' is ambiguous"));
        assert!(text.contains("'--backup'"));
        assert!(text.contains("'--batch'"));
        assert!(text.contains("'--basename-prefix'"));
        assert!(text.contains("'--backup-if-mismatch'"));
    }

    #[test]
    fn extra_operand_is_shell_quoted() {
        let text = trouble(&["/usr/bin/patch", "a", "b", "c d"]);
        assert!(text.starts_with("/usr/bin/patch: 'c d': extra operand\n"));
        let text = trouble(&["/usr/bin/patch", "a", "b", "c"]);
        assert!(text.starts_with("/usr/bin/patch: c: extra operand\n"));
    }

    #[test]
    fn quoting_style_uses_short_name() {
        let text = trouble(&["/usr/bin/patch", "--quoting-style=nope"]);
        assert!(text.starts_with("patch: invalid argument 'nope' for 'quoting style'\n"));
    }

    #[test]
    fn empty_backup_prefix() {
        let text = trouble(&["patch", "-B", ""]);
        assert_eq!(text, "patch: **** backup prefix is empty\n");
    }

    #[test]
    fn parses_common_flags() {
        let opt = parse(&argv(&[
            "patch",
            "-p1",
            "-R",
            "-N",
            "--dry-run",
            "-s",
            "--merge=diff3",
            "orig",
            "diff",
        ]))
        .unwrap();
        assert_eq!(opt.strip, 1);
        assert!(opt.reverse && opt.forward && opt.dry_run);
        assert_eq!(opt.verbosity, Verbosity::Silent);
        assert_eq!(opt.conflict_style, ConflictStyle::Diff3);
        assert_eq!(opt.original.as_deref(), Some("orig"));
        assert_eq!(opt.patch_file.as_deref(), Some("diff"));
    }

    fn env_with(posixly_correct: bool) -> ProcessEnv {
        ProcessEnv {
            posixly_correct,
            quoting_style: None,
            patch_get: None,
            simple_backup_suffix: None,
            patch_version_control: None,
            version_control: None,
        }
    }

    #[test]
    fn posix_env_stops_at_operand() {
        let opt = parse_with(
            &argv(&["patch", "-p0", "file", "--dry-run"]),
            &env_with(true),
        )
        .unwrap();
        assert!(!opt.dry_run);
        assert_eq!(opt.original.as_deref(), Some("file"));
        assert_eq!(opt.patch_file.as_deref(), Some("--dry-run"));
    }

    #[test]
    fn posix_flag_still_permutes() {
        let opt = parse_with(
            &argv(&["patch", "--posix", "-p0", "file", "--dry-run"]),
            &env_with(false),
        )
        .unwrap();
        assert!(opt.posix);
        assert!(opt.dry_run);
        assert_eq!(opt.original.as_deref(), Some("file"));
        assert!(opt.patch_file.is_none());
        assert!(!opt.backup_if_mismatch);
    }

    #[test]
    fn backup_if_mismatch_defaults() {
        let clean = env_with(false);
        let opt = parse_with(&argv(&["patch"]), &clean).unwrap();
        assert!(opt.backup_if_mismatch);
        let opt = parse_with(&argv(&["patch", "--no-backup-if-mismatch"]), &clean).unwrap();
        assert!(!opt.backup_if_mismatch);
        let opt = parse_with(&argv(&["patch", "--posix", "--backup-if-mismatch"]), &clean).unwrap();
        assert!(opt.backup_if_mismatch);
        let opt = parse_with(&argv(&["patch", "-V", "none", "-b"]), &clean).unwrap();
        assert_eq!(opt.backup_type, BackupType::None);
        let opt = parse_with(&argv(&["patch", "-V", "nu"]), &clean).unwrap();
        assert_eq!(opt.backup_type, BackupType::Numbered);
        let opt = parse_with(&argv(&["patch", "--posix", "-V", "nope"]), &clean).unwrap();
        assert_eq!(opt.backup_type, BackupType::Existing);
    }

    #[test]
    fn version_control_lists_arguments_without_try_line() {
        let text = trouble(&["/usr/bin/patch", "-V", "nope"]);
        assert_eq!(
            text,
            "\
patch: invalid argument 'nope' for '--version-control or -V option'
Valid arguments are:
  - 'none', 'off'
  - 'simple', 'never'
  - 'existing', 'nil'
  - 'numbered', 't'
"
        );
        let text = trouble(&["/usr/bin/patch", "-V", "n"]);
        assert!(
            text.starts_with(
                "patch: ambiguous argument 'n' for '--version-control or -V option'\n"
            )
        );
        assert!(!text.contains("Try"));
    }

    #[test]
    fn ambiguous_includes_the_full_token() {
        let text = trouble(&["/usr/bin/patch", "--ba=x"]);
        assert!(text.starts_with(
            "/usr/bin/patch: option '--ba=x' is ambiguous; possibilities: '--backup' '--batch' '--basename-prefix' '--backup-if-mismatch'\n"
        ));
    }

    #[test]
    fn version_and_help_text() {
        assert!(version_text().starts_with("GNU patch 2.8\n"));
        assert!(version_text().contains("Written by Larry Wall and Paul Eggert\n"));
        let help = help_text("patch");
        assert!(help.starts_with("Usage: patch [OPTION]... [ORIGFILE [PATCHFILE]]\n"));
        assert!(help.contains("--merge"));
        assert!(help.ends_with("Report bugs to <bug-patch@gnu.org>.\n"));
    }
}
