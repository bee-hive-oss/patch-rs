//! Apply parsed diffs, write reject files, and back up originals.
//!
//! Messages, exit codes, and the files left behind follow GNU patch 2.8.
//! A backup is made for `-b`, or by default when a hunk mismatches or fails.
//! The same inode is backed up only before this run replaces it. Deletes wait
//! until every file has been applied, so a later create still sees the
//! original backup.

use crate::apply::{self, ApplyError, RejectNames, RejectStyle, format_reject_hunk};
use crate::cli::{
    self, BackupType, ConflictStyle, EXIT_FAILURE, ForcedDiff, Options, ReadOnly, RejectFormat,
    Verbosity,
};
use crate::diff::{self, DiffKind, FilePatch, Hunk, strip_name};
use crate::merge;
use std::fs::{self, File, OpenOptions};
use std::io::{self, IsTerminal, Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, UNIX_EPOCH};

const S_IFMT: u32 = 0o170000;
const S_IFREG: u32 = 0o100000;
const S_IFLNK: u32 = 0o120000;
const S_IFDIR: u32 = 0o040000;

struct Engine<'a> {
    opt: &'a Options,
    somefailed: bool,
    /// Reject paths created during this run. A later failure appends.
    reject_created: Vec<String>,
    tmp_seq: u64,
    at_root: bool,
    outfile: Option<File>,
    seen_file: bool,
    /// Inodes this run has created, overwritten, or scheduled for deletion.
    files: Vec<FileRec>,
    pending_deletes: Vec<PendingDelete>,
    /// Git diffs write these after later patches have read the old bytes.
    pending_outputs: Vec<PendingOutput>,
    /// Inodes whose queued replacement must be flushed before they are read.
    queued_inodes: Vec<(u64, u64)>,
    was_git: bool,
}

struct Names {
    old: Option<String>,
    new: Option<String>,
    index: Option<String>,
    old_time: String,
    new_time: String,
}

struct Pick {
    name: Option<String>,
    skip: bool,
}

struct InputInfo {
    data: Vec<u8>,
    exists: bool,
    len: u64,
    mode: u32,
    kind: u32,
    refused: bool,
    /// Modification time captured before the file is replaced.
    mtime: Option<(i64, u32)>,
}

struct Applied {
    output: Vec<u8>,
    failed: i64,
    hunk_count: i64,
    skip: bool,
    reverse: bool,
    reject: Vec<u8>,
    /// True when any locate needed fuzz, an offset, or failed to match.
    mismatch: bool,
    /// A `--merge` hunk wrote conflict markers.
    conflict: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum IdKind {
    Created,
    DeleteLater,
    Overwritten,
}

struct FileRec {
    dev: u64,
    ino: u64,
    kind: IdKind,
}

struct PendingDelete {
    name: String,
    dev: u64,
    ino: u64,
    kind: u32,
    backup: bool,
}

struct PendingOutput {
    path: String,
    bytes: Vec<u8>,
    mode: Option<u32>,
    backup: bool,
    /// Set after the delayed write. `None` leaves the write time alone.
    mtime: Option<(i64, u32)>,
}

#[derive(Clone, Copy)]
struct StatBuf {
    ok: bool,
    len: u64,
    kind: u32,
    dev: u64,
    ino: u64,
}

pub fn run(opt: &Options) -> i32 {
    let mut engine = Engine::new(opt);
    match engine.execute() {
        Ok(code) => code,
        Err(message) => {
            engine.fatal_print(&message);
            cli::EXIT_TROUBLE
        }
    }
}

impl<'a> Engine<'a> {
    fn new(opt: &'a Options) -> Self {
        Self {
            opt,
            somefailed: false,
            reject_created: Vec::new(),
            tmp_seq: 0,
            at_root: cwd_is_root(),
            outfile: None,
            seen_file: false,
            files: Vec::new(),
            pending_deletes: Vec::new(),
            pending_outputs: Vec::new(),
            queued_inodes: Vec::new(),
            was_git: false,
        }
    }

    fn execute(&mut self) -> Result<i32, String> {
        if let Some(path) = &self.opt.output
            && path != "-"
        {
            self.outfile = Some(self.open_output(Path::new(path))?);
        }
        let input = self.read_patch()?;
        if !self.opt.binary
            && let Some(line) = first_nul_line(&input)
        {
            return Err(format!("patch line {line} contains NUL byte"));
        }
        let need_header = self.opt.original.is_none() && !self.opt.posix;
        let force_normal = self.opt.forced_diff == ForcedDiff::Normal;
        let force_ed = self.opt.forced_diff == ForcedDiff::Ed;
        let parsed =
            diff::parse_patch_ex(&input, need_header, force_normal, force_ed, self.opt.binary)?;
        if parsed.files.is_empty() {
            return self.no_files(&input);
        }
        for file in &parsed.files {
            if self.was_git && !file.git {
                self.flush_queued_outputs()?;
            }
            self.was_git = file.git;
            if let Err(err) = self.apply_file(file) {
                let _ = self.flush_queued_outputs();
                return Err(err);
            }
        }
        if self.opt.verbosity == Verbosity::Verbose {
            self.say("done\n");
        }
        self.flush_queued_outputs()?;
        self.flush_deletes()?;
        if self.somefailed {
            Ok(EXIT_FAILURE)
        } else {
            Ok(0)
        }
    }

    fn no_files(&mut self, input: &[u8]) -> Result<i32, String> {
        if input.is_empty() {
            if let Some(from) = &self.opt.original
                && let Some(to) = &self.opt.output
            {
                if to != "-" && !self.opt.dry_run {
                    if self.opt.verbosity != Verbosity::Silent {
                        self.say(&format!(
                            "patching file {} (read from {})\n",
                            self.opt.quote(to),
                            self.opt.quote(from)
                        ));
                    }
                    let data = fs::read(from).unwrap_or_default();
                    self.write_output_bytes(Path::new(to), &data, None)?;
                } else if self.opt.verbosity != Verbosity::Silent {
                    self.say(&format!(
                        "patching file {} (read from {})\n",
                        self.opt.quote(to),
                        self.opt.quote(from)
                    ));
                }
            }
            return Ok(0);
        }
        if self.opt.verbosity == Verbosity::Verbose {
            self.say("Hmm...");
            self.say("  I can't seem to find a patch in there anywhere.\n");
        }
        Err("Only garbage was found in the patch input.".to_string())
    }

    fn apply_file(&mut self, file: &FilePatch) -> Result<(), String> {
        if self.opt.verbosity == Verbosity::Verbose {
            self.say("Hmm...");
        }
        // Context hunks are parsed while GNU is still choosing a file name.
        // A mangled first hunk dies before "patching file".
        if file.hunks.is_empty()
            && matches!(file.kind, DiffKind::Context | DiffKind::NewContext)
            && let Some(error) = &file.parse_error
        {
            return Err(error.clone());
        }
        let names = self.strip_names(file);
        let patch_type = patch_file_type(file);
        let mut reverse = self.opt.reverse;
        let mut pick = self.select_name(file, &names, patch_type, &mut reverse);
        if self.opt.verbosity == Verbosity::Verbose {
            self.say_kind(file.kind);
        }
        if pick.name.is_none() {
            self.missing_name(file);
            pick.skip = true;
        } else if self.opt.verbosity == Verbosity::Verbose {
            self.print_leading(&file.leading);
        }
        if pick.skip {
            self.somefailed = true;
        }
        if !pick.skip && patch_type == 0 {
            self.report_type_change(file, &pick, reverse);
            pick.skip = true;
            self.somefailed = true;
        }
        let outname = self.out_name(file, &names, &pick, reverse);
        if file.git
            && !pick.skip
            && self.opt.output.is_none()
            && let Some(out) = &outname
        {
            self.note_git_output_target(out)?;
        }
        if !pick.skip && file.kind == DiffKind::GitBinary {
            if let Some(name) = &outname {
                self.say(&format!(
                    "File {}: git binary diffs are not supported.\n",
                    self.opt.quote(name)
                ));
            }
            pick.skip = true;
            self.somefailed = true;
        }
        let mut info = InputInfo::missing();
        if !pick.skip
            && let Some(name) = &pick.name
        {
            info = self.load_input(name, patch_type)?;
            if info.refused {
                pick.skip = true;
                self.somefailed = true;
            }
        }
        if self.read_only_blocked(&pick, &info) {
            pick.skip = true;
            self.somefailed = true;
        }
        if file.strip_trailing_cr && self.opt.verbosity != Verbosity::Silent {
            self.say("(Stripping trailing CRs from patch; use --binary to disable.)\n");
        }
        if file.kind != DiffKind::Ed && !pick.skip && self.opt.verbosity != Verbosity::Silent {
            self.say_patching(file, &names, &pick, &outname, patch_type);
        }
        if file.hunks.is_empty()
            && let Some(error) = &file.parse_error
        {
            return Err(error.clone());
        }
        if file.kind == DiffKind::Ed {
            self.apply_ed(file, &pick, &outname, &info)?;
            self.seen_file = true;
            return Ok(());
        }
        let applied = self.apply_hunks(file, &names, &info, reverse, pick.skip)?;
        if applied.failed > 0 || applied.skip || applied.conflict {
            self.somefailed = true;
        }
        self.finish_output(file, &pick, &outname, &info, &applied)?;
        if applied.failed > 0 {
            self.report_failure(file, &outname, &applied)?;
        }
        self.seen_file = true;
        Ok(())
    }

    fn report_type_change(&mut self, file: &FilePatch, pick: &Pick, reverse: bool) {
        let Some(name) = &pick.name else {
            return;
        };
        let old_mode = side_mode(file, reverse) & S_IFMT;
        let new_mode = side_mode(file, !reverse) & S_IFMT;
        self.say(&format!(
            "File {}: can't change file type from {} to {}.\n",
            self.opt.quote(name),
            c_octal(old_mode),
            c_octal(new_mode)
        ));
    }

    fn missing_name(&mut self, file: &FilePatch) {
        if self.opt.verbosity != Verbosity::Silent {
            self.say(&format!(
                "can't find file to patch at input line {}\n",
                file.report_line
            ));
            if file.kind != DiffKind::Ed && file.kind != DiffKind::Normal {
                if self.opt.strip < 0 {
                    self.say("Perhaps you should have used the -p or --strip option?\n");
                } else {
                    self.say("Perhaps you used the wrong -p or --strip option?\n");
                }
            }
        }
        self.print_leading(&file.leading);
        if self.opt.force || self.opt.batch {
            self.say("No file to patch.  Skipping patch.\n");
        } else if self.opt.verbosity != Verbosity::Silent {
            self.say("Skipping patch.\n");
        }
    }

    fn say_kind(&mut self, kind: DiffKind) {
        let label = match kind {
            DiffKind::Unified => "a unified diff",
            DiffKind::Context => "a context diff",
            DiffKind::NewContext => "a new-style context diff",
            DiffKind::Normal => "a normal diff",
            DiffKind::Ed => "an ed script",
            DiffKind::GitBinary => "a git binary diff",
        };
        let lead = if self.seen_file {
            "The next patch l"
        } else {
            "L"
        };
        self.say(&format!("  {lead}ooks like {label} to me...\n"));
    }

    fn print_leading(&mut self, leading: &[u8]) {
        if leading.is_empty() {
            return;
        }
        self.say("The text leading up to this was:\n--------------------------\n");
        let mut start = 0usize;
        for (i, &byte) in leading.iter().enumerate() {
            if byte == b'\n' {
                self.say("|");
                self.write_stdout(&leading[start..=i]);
                start = i + 1;
            }
        }
        if start < leading.len() {
            self.say("|");
            self.write_stdout(&leading[start..]);
            self.say("\n");
        }
        self.say("--------------------------\n");
    }

    fn strip_names(&self, file: &FilePatch) -> Names {
        let old = file
            .old_name
            .as_deref()
            .and_then(|name| strip_name(name, self.opt.strip));
        let new = file
            .new_name
            .as_deref()
            .and_then(|name| strip_name(name, self.opt.strip));
        let mut index = file
            .index_name
            .as_deref()
            .and_then(|name| strip_name(name, self.opt.strip));
        if self.opt.original.is_none() && !self.opt.posix && (old.is_some() || new.is_some()) {
            index = None;
        }
        Names {
            old,
            new,
            index,
            old_time: file.old_timestr.clone(),
            new_time: file.new_timestr.clone(),
        }
    }

    fn out_name(
        &self,
        file: &FilePatch,
        names: &Names,
        pick: &Pick,
        reverse: bool,
    ) -> Option<String> {
        if let Some(path) = &self.opt.output
            && !pick.skip
        {
            return Some(path.clone());
        }
        if pick.skip {
            return None;
        }
        if file.copy || file.rename {
            side_name(names, !reverse)
        } else {
            pick.name.clone()
        }
    }

    fn say_patching(
        &mut self,
        file: &FilePatch,
        names: &Names,
        pick: &Pick,
        outname: &Option<String>,
        kind: u32,
    ) {
        let Some(out) = outname else {
            return;
        };
        let verb = if self.opt.dry_run {
            "checking"
        } else {
            "patching"
        };
        let what = if kind == S_IFLNK {
            "symbolic link"
        } else {
            "file"
        };
        let quoted = self.opt.quote(out);
        let inn = pick.name.as_deref().unwrap_or("");
        let renamed = inn != out.as_str();
        let already = !renamed && file.rename;
        if !renamed && !already {
            self.say(&format!("{verb} {what} {quoted}\n"));
            return;
        }
        let how = if file.copy {
            "copied"
        } else if file.rename {
            "renamed"
        } else {
            "read"
        };
        let from = if !already {
            inn.to_string()
        } else if inn == names.old.as_deref().unwrap_or("") {
            names.new.clone().unwrap_or_default()
        } else {
            names.old.clone().unwrap_or_default()
        };
        let extra = if already { "already " } else { "" };
        self.say(&format!(
            "{verb} {what} {quoted} ({extra}{how} from {})\n",
            self.opt.quote(&from)
        ));
    }

    fn load_input(&mut self, name: &str, patch_type: u32) -> Result<InputInfo, String> {
        let path = Path::new(name);
        let meta = if self.opt.follow_symlinks {
            fs::metadata(path)
        } else {
            fs::symlink_metadata(path)
        };
        let meta = match meta {
            Ok(meta) => meta,
            Err(_) => return Ok(InputInfo::missing()),
        };
        if self.lookup(meta.dev(), meta.ino()) == Some(IdKind::DeleteLater) {
            return Ok(InputInfo::missing());
        }
        let kind = meta_kind(&meta);
        let type_ok = (patch_type == S_IFREG || patch_type == S_IFLNK) && kind == patch_type;
        if !type_ok {
            let what = if patch_type == S_IFLNK {
                "symbolic link"
            } else {
                "regular file"
            };
            self.say(&format!(
                "File {} is not a {what} -- refusing to patch\n",
                self.opt.quote(name)
            ));
            return Ok(InputInfo {
                data: Vec::new(),
                exists: true,
                len: meta.len(),
                mode: meta_perm(&meta),
                kind,
                refused: true,
                mtime: file_mtime(&meta),
            });
        }
        if kind == S_IFLNK {
            let target = fs::read_link(path).unwrap_or_default();
            return Ok(InputInfo {
                data: target.to_string_lossy().into_owned().into_bytes(),
                exists: true,
                len: meta.len(),
                mode: meta_perm(&meta),
                kind,
                refused: false,
                mtime: file_mtime(&meta),
            });
        }
        if meta.len() == 0 {
            return Ok(InputInfo {
                data: Vec::new(),
                exists: true,
                len: 0,
                mode: meta_perm(&meta),
                kind,
                refused: false,
                mtime: file_mtime(&meta),
            });
        }
        let data = fs::read(path).map_err(|err| {
            format!(
                "Can't open file {} : {}",
                self.opt.quote(name),
                cli::io_error_text(&err)
            )
        })?;
        Ok(InputInfo {
            data,
            exists: true,
            len: meta.len(),
            mode: meta_perm(&meta),
            kind,
            refused: false,
            mtime: file_mtime(&meta),
        })
    }

    fn read_only_blocked(&mut self, pick: &Pick, info: &InputInfo) -> bool {
        if self.opt.read_only == ReadOnly::Ignore || !info.exists || info.kind == S_IFLNK {
            return false;
        }
        let Some(name) = &pick.name else {
            return false;
        };
        if can_write(Path::new(name)) {
            return false;
        }
        self.say(&format!("File {} is read-only; ", self.opt.quote(name)));
        if self.opt.read_only == ReadOnly::Warn {
            self.say("trying to patch anyway\n");
            false
        } else {
            self.say("refusing to patch\n");
            true
        }
    }
}

impl InputInfo {
    fn missing() -> Self {
        Self {
            data: Vec::new(),
            exists: false,
            len: 0,
            mode: 0o666,
            kind: S_IFREG,
            refused: false,
            mtime: None,
        }
    }
}

impl Engine<'_> {
    fn select_name(
        &mut self,
        file: &FilePatch,
        names: &Names,
        patch_type: u32,
        reverse: &mut bool,
    ) -> Pick {
        let mut warned = Vec::new();
        if let Some(explicit) = &self.opt.original {
            return self.pick_explicit(file, explicit, patch_type, reverse);
        }
        let slots = [names.old.clone(), names.new.clone(), names.index.clone()];
        let mut stats = [StatBuf::missing(), StatBuf::missing(), StatBuf::missing()];
        let mut i0: Option<usize> = None;
        let mut chosen: Option<usize> = None;
        for i in 0..3 {
            let Some(name) = slots[i].as_deref() else {
                continue;
            };
            if let Some(prev) = i0
                && slots[prev].as_deref() == Some(name)
            {
                stats[i] = stats[prev];
                i0 = Some(i);
                continue;
            }
            stats[i] = self.conceal_deleted(inspect(name, self.opt.follow_symlinks));
            if stats[i].ok && self.opt.posix && self.name_is_valid(name, &mut warned) {
                chosen = Some(i);
                i0 = Some(i);
                break;
            }
            i0 = Some(i);
        }
        if !self.opt.posix {
            let ignore = [
                slots[0].is_some() && !stats[0].ok,
                slots[1].is_some() && !stats[1].ok,
                slots[2].is_some() && !stats[2].ok,
            ];
            chosen = self.best_name(&slots, &ignore, &mut warned);
            let type_ok = match chosen {
                Some(i) => stats[i].kind == patch_type,
                None => true,
            };
            // A reversed delete of a missing file would create it
            // (CVE-2026-115207). Reject that name before maybe_reverse, and
            // only when the reversal would adopt it. An ordinary edit whose
            // other name is `..` or absolute stays on the best_name path
            // (GNU bad-filenames, `--- f` / `+++ ../g`).
            if let Some(fallback) = i0
                && (chosen.is_none() || type_ok)
            {
                let idx = chosen.unwrap_or(fallback);
                let name = slots[idx].as_deref().unwrap_or("");
                let nonexistent = chosen.is_none();
                let is_empty = nonexistent || stats[idx].len == 0;
                let dangerous_create = nonexistent
                    && reversal_looks(file, nonexistent, is_empty, *reverse)
                    && !self.name_is_valid(name, &mut warned);
                if !dangerous_create {
                    let answer = self.maybe_reverse(file, name, nonexistent, is_empty, *reverse);
                    *reverse = answer.reverse;
                    if answer.looks && chosen.is_none() {
                        chosen = Some(fallback);
                    }
                    if answer.skip {
                        let pick = self.pick_from(chosen, &slots);
                        return Pick { skip: true, ..pick };
                    }
                }
            }
            if chosen.is_none() && says(file, *reverse) != 0 {
                chosen = self.best_fewest_dirs(&slots, &mut warned);
            }
        }
        if (file.rename || file.copy) && !self.rename_names_ok(file, names, chosen, &mut warned) {
            let which = if file.rename { "rename" } else { "copy" };
            self.say(&format!(
                "Cannot {which} file without two valid file names\n"
            ));
            let pick = self.pick_from(chosen, &slots);
            return Pick { skip: true, ..pick };
        }
        self.pick_from(chosen, &slots)
    }

    fn pick_explicit(
        &mut self,
        file: &FilePatch,
        name: &str,
        patch_type: u32,
        reverse: &mut bool,
    ) -> Pick {
        let stat = self.conceal_deleted(inspect(name, self.opt.follow_symlinks));
        if !stat.ok || stat.kind == patch_type {
            let answer =
                self.maybe_reverse(file, name, !stat.ok, !stat.ok || stat.len == 0, *reverse);
            *reverse = answer.reverse;
            return Pick {
                name: Some(name.to_string()),
                skip: answer.skip,
            };
        }
        Pick {
            name: Some(name.to_string()),
            skip: false,
        }
    }

    fn pick_from(&self, chosen: Option<usize>, slots: &[Option<String>; 3]) -> Pick {
        let Some(i) = chosen else {
            return Pick {
                name: None,
                skip: false,
            };
        };
        Pick {
            name: slots[i].clone(),
            skip: false,
        }
    }

    fn rename_names_ok(
        &mut self,
        file: &FilePatch,
        names: &Names,
        chosen: Option<usize>,
        warned: &mut Vec<String>,
    ) -> bool {
        if !(file.rename || file.copy) {
            return true;
        }
        let Some(i) = chosen else {
            return false;
        };
        if i > 1 {
            return false;
        }
        let Some(old) = names.old.as_deref() else {
            return false;
        };
        let Some(new) = names.new.as_deref() else {
            return false;
        };
        self.name_is_valid(old, warned) && self.name_is_valid(new, warned)
    }

    fn best_fewest_dirs(
        &mut self,
        slots: &[Option<String>; 3],
        warned: &mut Vec<String>,
    ) -> Option<usize> {
        let mut dirs = [0i64; 3];
        let mut have = [false; 3];
        let mut min = i64::MAX;
        for i in 0..3 {
            if let Some(name) = slots[i].as_deref() {
                dirs[i] = prefix_components(name, false) - prefix_components(name, true);
                have[i] = true;
                if dirs[i] < min {
                    min = dirs[i];
                }
            }
        }
        let above = [
            have[0] && min < dirs[0],
            have[1] && min < dirs[1],
            have[2] && min < dirs[2],
        ];
        self.best_name(slots, &above, warned)
    }

    fn best_name(
        &mut self,
        names: &[Option<String>; 3],
        ignore: &[bool; 3],
        warned: &mut Vec<String>,
    ) -> Option<usize> {
        let mut components = [0i64; 3];
        let mut components_min = i64::MAX;
        let mut base_len = [None; 3];
        let mut base_min = i64::MAX;
        let mut full_len = [None; 3];
        let mut full_min = i64::MAX;
        for i in 0..3 {
            let Some(name) = names[i].as_deref() else {
                continue;
            };
            if ignore[i] {
                continue;
            }
            let comps = prefix_components(name, false);
            components[i] = comps;
            if components_min < comps {
                continue;
            }
            components_min = comps;
            let base = basename_len(name);
            base_len[i] = Some(base);
            if base_min < base {
                continue;
            }
            base_min = base;
            let full = name.len() as i64;
            full_len[i] = Some(full);
            if full_min < full {
                continue;
            }
            full_min = full;
        }
        for i in 0..3 {
            let Some(name) = names[i].as_deref() else {
                continue;
            };
            if ignore[i] {
                continue;
            }
            let Some(base) = base_len[i] else {
                continue;
            };
            let Some(full) = full_len[i] else {
                continue;
            };
            if components[i] == components_min
                && base == base_min
                && full == full_min
                && self.name_is_valid(name, warned)
            {
                return Some(i);
            }
        }
        None
    }

    fn name_is_valid(&mut self, name: &str, warned: &mut Vec<String>) -> bool {
        if diff::name_is_safe(name) || self.at_root {
            return true;
        }
        if !warned.iter().any(|seen| seen == name) {
            self.say(&format!(
                "Ignoring potentially dangerous file name {}\n",
                self.opt.quote(name)
            ));
            warned.push(name.to_string());
        }
        false
    }

    fn maybe_reverse(
        &mut self,
        file: &FilePatch,
        name: &str,
        nonexistent: bool,
        is_empty: bool,
        reverse: bool,
    ) -> ReverseAnswer {
        if !reversal_looks(file, nonexistent, is_empty, reverse) {
            return ReverseAnswer {
                looks: false,
                reverse,
                skip: false,
            };
        }
        let action = if nonexistent {
            "delete"
        } else if is_empty {
            "empty out"
        } else {
            "create"
        };
        let state = if nonexistent {
            "does not exist"
        } else if is_empty {
            "is already empty"
        } else {
            "already exists"
        };
        let when = if reverse { ", when reversed," } else { "" };
        let detected = format!(
            "The next patch{when} would {action} the file {},\nwhich {state}!",
            self.opt.quote(name)
        );
        let (flip, skip) = self.ok_to_reverse(&detected, reverse);
        ReverseAnswer {
            looks: true,
            reverse: reverse ^ flip,
            skip,
        }
    }

    fn ok_to_reverse(&mut self, detected: &str, reverse: bool) -> (bool, bool) {
        let quiet_force = self.opt.force && self.opt.verbosity == Verbosity::Silent;
        if self.opt.forward || !quiet_force {
            self.say(detected);
        }
        if self.opt.forward {
            self.say("  Skipping patch.\n");
            return (false, true);
        }
        if self.opt.force {
            if self.opt.verbosity != Verbosity::Silent {
                self.say("  Applying it anyway.\n");
            }
            return (false, false);
        }
        if self.opt.batch {
            if reverse {
                self.say("  Ignoring -R.\n");
            } else {
                self.say("  Assuming -R.\n");
            }
            return (true, false);
        }
        let prompt = if reverse {
            "  Ignore -R? [n] "
        } else {
            "  Assume -R? [n] "
        };
        if self.ask(prompt).starts_with('y') {
            return (true, false);
        }
        if !self.ask("Apply anyway? [n] ").starts_with('y') {
            if self.opt.verbosity != Verbosity::Silent {
                self.say("Skipping patch.\n");
            }
            return (false, true);
        }
        (false, false)
    }

    fn ask(&mut self, prompt: &str) -> String {
        self.say(prompt);
        if (self.opt.posix || io::stdout().is_terminal())
            && let Ok(mut tty) = File::open("/dev/tty")
        {
            let mut buf = [0u8; 256];
            if let Ok(n) = tty.read(&mut buf)
                && n > 0
            {
                return String::from_utf8_lossy(&buf[..n]).into_owned();
            }
        }
        self.say("\n");
        "\n".to_string()
    }
}

struct ReverseAnswer {
    looks: bool,
    reverse: bool,
    skip: bool,
}

impl Engine<'_> {
    fn apply_hunks(
        &mut self,
        file: &FilePatch,
        names: &Names,
        info: &InputInfo,
        mut reverse: bool,
        mut skip: bool,
    ) -> Result<Applied, String> {
        let cli_reverse = self.opt.reverse;
        let mut hunks = file.hunks.clone();
        if reverse {
            for hunk in &mut hunks {
                hunk.swap();
            }
        }
        let lines = apply::split_file(&info.data);
        let mut output = Vec::new();
        let mut frozen = 0i64;
        let mut in_offset = 0i64;
        let mut out_offset = 0i64;
        let mut failed = 0i64;
        let mut mismatch = false;
        let mut conflict = false;
        let mut header = true;
        let mut reject = Vec::new();
        let mut apply_anyway = false;
        let style = reject_style(self.opt, file.kind);
        let hunk_count = hunks.len() as i64;

        for index in 0..hunks.len() {
            let hunk_no = index as i64 + 1;
            let mut where_line = None;
            let mut fuzz_used = 0i64;
            if !skip {
                let mut cursor = LocateCursor {
                    hunks: &mut hunks,
                    lines: &lines,
                    frozen,
                    in_offset: &mut in_offset,
                    reverse: &mut reverse,
                    cli_reverse,
                    apply_anyway: &mut apply_anyway,
                    skip: &mut skip,
                    mismatch: &mut mismatch,
                };
                let located = self.locate_with_fuzz(index, &mut cursor);
                where_line = located.where_line;
                fuzz_used = located.fuzz;
            }
            let hunk = &hunks[index];
            let base = where_line.unwrap_or(hunk.old_first);
            let newwhere = base + out_offset;
            let says_old = says(file, reverse);
            let blocked = where_line == Some(1) && says_old == 2 && info.len != 0;
            let mut hunk_failed = skip || blocked || where_line.is_none();
            if self.opt.merge && !skip {
                let located = where_line.unwrap_or(0);
                let diff3 = self.opt.conflict_style == ConflictStyle::Diff3;
                let verbose = self.opt.verbosity == Verbosity::Verbose;
                let silent = self.opt.verbosity == Verbosity::Silent;
                let ignore_ws = self.opt.ignore_whitespace;
                match merge::merge_hunk(
                    hunk,
                    &lines,
                    located,
                    hunk_no,
                    diff3,
                    verbose,
                    silent,
                    ignore_ws,
                    in_offset,
                    &mut output,
                    &mut frozen,
                    &mut out_offset,
                    &mut |text| self.say(text),
                ) {
                    Ok(had_conflict) => {
                        hunk_failed = false;
                        if had_conflict {
                            conflict = true;
                        }
                    }
                    Err(ApplyError::Misordered) => hunk_failed = true,
                    Err(ApplyError::Mangled { old, new }) => {
                        return Err(format!(
                            "Out-of-sync patch, lines {old},{new} -- mangled text or line numbers, maybe?"
                        ));
                    }
                }
            } else if !hunk_failed {
                let at = where_line.unwrap_or(1);
                match apply::apply_hunk(hunk, &lines, at, &mut output, &mut frozen) {
                    Ok(()) => out_offset += hunk.repl_lines - hunk.ptrn_lines,
                    Err(ApplyError::Misordered) => hunk_failed = true,
                    Err(ApplyError::Mangled { old, new }) => {
                        return Err(format!(
                            "Out-of-sync patch, lines {old},{new} -- mangled text or line numbers, maybe?"
                        ));
                    }
                }
            }
            if hunk_failed {
                let rej_names = reject_names(names, reverse);
                reject.extend(format_reject_hunk(
                    hunk, file.kind, style, out_offset, &rej_names, header,
                ));
                header = false;
                failed += 1;
                if self.opt.verbosity == Verbosity::Verbose
                    || (!skip && self.opt.verbosity != Verbosity::Silent)
                {
                    let what = if skip { "ignored" } else { "FAILED" };
                    let endings = if !skip && apply::different_line_endings(hunk, &lines, newwhere)
                    {
                        " (different line endings)"
                    } else {
                        ""
                    };
                    self.say(&format!("Hunk #{hunk_no} {what} at {newwhere}{endings}.\n"));
                }
            } else if !self.opt.merge
                && (self.opt.verbosity == Verbosity::Verbose
                    || (self.opt.verbosity != Verbosity::Silent
                        && (fuzz_used != 0 || in_offset != 0)))
            {
                self.say_success(hunk_no, newwhere, fuzz_used, in_offset);
            }
        }
        if let Some(error) = &file.parse_error {
            return Err(error.clone());
        }
        if !skip && !apply::spew_rest(&lines, &mut output, &mut frozen) {
            self.say("Skipping patch.\n");
            skip = true;
        }
        Ok(Applied {
            output,
            failed,
            hunk_count,
            skip,
            reverse,
            reject,
            mismatch,
            conflict,
        })
    }

    fn locate_with_fuzz(&mut self, index: usize, cursor: &mut LocateCursor<'_>) -> Located {
        let context = cursor.hunks[index]
            .prefix_context
            .max(cursor.hunks[index].suffix_context)
            .max(0);
        let max_fuzz = if self.opt.merge {
            0
        } else {
            self.opt.fuzz.min(context)
        };
        let mut fuzz = 0i64;
        loop {
            let mut incr = true;
            let mut trial = *cursor.in_offset;
            let found = apply::locate_hunk(
                &cursor.hunks[index],
                cursor.lines,
                fuzz,
                &mut trial,
                cursor.frozen,
                self.opt.ignore_whitespace,
            );
            if found.is_some() {
                *cursor.in_offset = trial;
            }
            if found.is_none() || fuzz != 0 || *cursor.in_offset != 0 {
                *cursor.mismatch = true;
            }
            let mut where_line = found;
            if index == 0
                && where_line.is_none()
                && !self.opt.force
                && !*cursor.apply_anyway
                && *cursor.reverse == cursor.cli_reverse
            {
                let mut swapped = cursor.hunks[index].clone();
                swapped.swap();
                let mut rev_offset = *cursor.in_offset;
                let rev_found = apply::locate_hunk(
                    &swapped,
                    cursor.lines,
                    fuzz,
                    &mut rev_offset,
                    cursor.frozen,
                    self.opt.ignore_whitespace,
                );
                if rev_found.is_some() {
                    *cursor.in_offset = rev_offset;
                }
                let detected = if *cursor.reverse {
                    "Unreversed patch detected!"
                } else {
                    "Reversed (or previously applied) patch detected!"
                };
                let (flip, now_skip) = if rev_found.is_some() {
                    self.ok_to_reverse(detected, *cursor.reverse)
                } else {
                    (false, false)
                };
                if now_skip {
                    *cursor.skip = true;
                }
                if rev_found.is_some() && flip {
                    *cursor.reverse = !*cursor.reverse;
                    cursor.hunks[index] = swapped;
                    for later in cursor.hunks.iter_mut().skip(index + 1) {
                        later.swap();
                    }
                    where_line = rev_found;
                } else if rev_found.is_some() {
                    *cursor.apply_anyway = true;
                    incr = false;
                    where_line = None;
                }
            }
            if *cursor.skip || where_line.is_some() {
                return Located { where_line, fuzz };
            }
            if incr {
                fuzz += 1;
            }
            if fuzz > max_fuzz {
                return Located {
                    where_line: None,
                    fuzz,
                };
            }
        }
    }

    fn say_success(&mut self, hunk_no: i64, newwhere: i64, fuzz: i64, in_offset: i64) {
        self.say(&format!("Hunk #{hunk_no} succeeded at {newwhere}"));
        if fuzz != 0 {
            self.say(&format!(" with fuzz {fuzz}"));
        }
        if in_offset != 0 {
            let plural = if in_offset == 1 { "" } else { "s" };
            self.say(&format!(" (offset {in_offset} line{plural})"));
        }
        self.say(".\n");
    }

    fn finish_output(
        &mut self,
        file: &FilePatch,
        pick: &Pick,
        outname: &Option<String>,
        info: &InputInfo,
        applied: &Applied,
    ) -> Result<(), String> {
        let skip = pick.skip || applied.skip;
        if self.opt.output.is_some() {
            if let Some(out) = self.outfile.as_mut() {
                out.write_all(&applied.output)
                    .map_err(|err| format!("write error : {}", cli::io_error_text(&err)))?;
            }
            return Ok(());
        }
        if skip {
            return Ok(());
        }
        let Some(out) = outname else {
            return Ok(());
        };
        let says_new = says(file, !applied.reverse);
        let zero = applied.output.is_empty();
        let delete = zero
            && (self.opt.remove_empty
                || (!self.opt.posix && says_new == 2)
                || info.kind == S_IFLNK);
        let backup = self.wants_backup(applied);
        if delete {
            if !self.opt.dry_run && info.exists {
                self.queue_delete(out, backup)?;
            }
            return Ok(());
        }
        if !zero
            && says_new == 2
            && (self.opt.remove_empty || !self.opt.posix)
            && !(self.opt.merge && self.somefailed)
        {
            self.somefailed = true;
            if self.opt.verbosity != Verbosity::Silent {
                self.say(&format!(
                    "Not deleting file {} as content differs from patch\n",
                    self.opt.quote(out)
                ));
            }
        }
        let old_mode = side_mode(file, applied.reverse);
        let new_mode = side_mode(file, !applied.reverse);
        let set_mode = new_mode != 0 && old_mode != new_mode;
        let replace = applied.failed < applied.hunk_count || set_mode || file.copy || file.rename;
        if self.opt.dry_run {
            return Ok(());
        }
        if !replace {
            if backup {
                self.backup_unmodified(out)?;
            }
            return Ok(());
        }
        let mode = if info.exists || set_mode {
            Some(if set_mode {
                new_mode & 0o7777
            } else {
                info.mode
            })
        } else {
            None
        };
        let previous = file_ids(out, self.opt.follow_symlinks);
        // A git diff that edits an existing file keeps the old bytes readable
        // until a later patch in the series needs this output inode.
        let delay = file.git && says(file, applied.reverse) != 2 && self.opt.output.is_none();
        let mtime = self.chosen_mtime(file, info, applied, out);
        if delay {
            self.pending_outputs.push(PendingOutput {
                path: out.to_string(),
                bytes: applied.output.clone(),
                mode,
                backup,
                mtime,
            });
        } else {
            if backup {
                self.create_backup(out, previous.is_some(), false)?;
            }
            self.reject_newline_name(Path::new(out))?;
            self.place(Path::new(out), &applied.output, mode)
                .map_err(|err| format!("write error : {}", cli::io_error_text(&err)))?;
            if let Some((sec, nsec)) = mtime {
                set_mtime(Path::new(out), sec, nsec)
                    .map_err(|err| format!("write error : {}", cli::io_error_text(&err)))?;
            }
            self.note_replaced(out, previous);
        }
        if file.rename
            && let Some(src) = &pick.name
            && src != out
        {
            self.queue_delete(src, backup)?;
        }
        Ok(())
    }

    /// If this output inode already has a queued replacement, write queued
    /// files first. Then remember the inode so the next patch can do the same.
    fn note_git_output_target(&mut self, out: &str) -> Result<(), String> {
        if let Some((dev, ino)) = file_ids(out, self.opt.follow_symlinks)
            && self.queued_inodes.contains(&(dev, ino))
        {
            self.flush_queued_outputs()?;
        }
        if let Some((dev, ino)) = file_ids(out, self.opt.follow_symlinks)
            && !self.queued_inodes.contains(&(dev, ino))
        {
            self.queued_inodes.push((dev, ino));
        }
        Ok(())
    }

    fn flush_queued_outputs(&mut self) -> Result<(), String> {
        let pending = std::mem::take(&mut self.pending_outputs);
        for item in pending {
            let previous = file_ids(&item.path, self.opt.follow_symlinks);
            if item.backup {
                self.create_backup(&item.path, previous.is_some(), false)?;
            }
            self.reject_newline_name(Path::new(&item.path))?;
            self.place(Path::new(&item.path), &item.bytes, item.mode)
                .map_err(|err| format!("write error : {}", cli::io_error_text(&err)))?;
            if let Some((sec, nsec)) = item.mtime {
                set_mtime(Path::new(&item.path), sec, nsec)
                    .map_err(|err| format!("write error : {}", cli::io_error_text(&err)))?;
            }
            self.note_replaced(&item.path, previous);
        }
        Ok(())
    }

    /// Header time to apply, or `None` when GNU would leave the write time.
    ///
    /// A skip prints "Not setting time of file …" before the failure summary.
    fn chosen_mtime(
        &mut self,
        file: &FilePatch,
        info: &InputInfo,
        applied: &Applied,
        out: &str,
    ) -> Option<(i64, u32)> {
        if self.opt.dry_run || !(self.opt.set_time || self.opt.set_utc) {
            return None;
        }
        let assume_utc = self.opt.set_utc || tz_is_utc();
        let new_time = header_time(side_timestr(file, !applied.reverse), assume_utc)?;
        let old_time = header_time(side_timestr(file, applied.reverse), assume_utc);
        let quoted = self.opt.quote(out);
        if !self.opt.force
            && info.exists
            && says(file, applied.reverse) != 2
            && let Some(old) = old_time
            && info.mtime.is_some_and(|cur| cur != old)
        {
            self.say(&format!(
                "Not setting time of file {quoted} (time mismatch)\n"
            ));
            None
        } else if !self.opt.force && (applied.mismatch || applied.failed > 0) {
            self.say(&format!(
                "Not setting time of file {quoted} (contents mismatch)\n"
            ));
            None
        } else {
            Some(new_time)
        }
    }

    fn wants_backup(&self, applied: &Applied) -> bool {
        self.opt.backup || (self.opt.backup_if_mismatch && (applied.mismatch || applied.failed > 0))
    }

    /// Copy a backup and leave the original inode alone. Used when no hunk applied.
    fn backup_unmodified(&mut self, path: &str) -> Result<(), String> {
        if file_ids(path, self.opt.follow_symlinks).is_none() {
            self.say(&format!("Cannot stat file {path}, skipping backup\n"));
            return Ok(());
        }
        self.create_backup(path, true, true)
    }

    fn create_backup(
        &mut self,
        path: &str,
        exists: bool,
        leave_original: bool,
    ) -> Result<(), String> {
        if exists {
            let meta = match metadata_of(path, self.opt.follow_symlinks) {
                Ok(meta) => meta,
                Err(_) => {
                    self.say(&format!("Cannot stat file {path}, skipping backup\n"));
                    return Ok(());
                }
            };
            if self.lookup(meta.dev(), meta.ino()) == Some(IdKind::Created) {
                return Ok(());
            }
            let kind = meta_kind(&meta);
            if kind != S_IFREG && kind != S_IFLNK {
                let quoted = self.opt.quote(path);
                return Err(format!(
                    "File {quoted} is not a regular file -- refusing to create backup"
                ));
            }
        }
        let bak = self.backup_name(path);
        let dest = Path::new(&bak);
        if !exists {
            return self.create_empty_backup(dest);
        }
        let source = Path::new(path);
        if leave_original {
            self.copy_backup(source, dest)
        } else {
            self.rename_backup(source, dest)
        }
    }

    fn backup_name(&self, path: &str) -> String {
        let prefix = self.opt.backup_prefix.as_deref();
        let base_prefix = self.opt.backup_basename_prefix.as_deref();
        let suffix = self.opt.backup_suffix.as_deref();
        if prefix.is_some() || base_prefix.is_some() || suffix.is_some() {
            return manual_backup_name(path, prefix, base_prefix, suffix);
        }
        // GNU's `none` still reaches the numbered-name search.
        let kind = match self.opt.backup_type {
            BackupType::Simple => BackupType::Simple,
            BackupType::Existing => BackupType::Existing,
            BackupType::Numbered | BackupType::None => BackupType::Numbered,
        };
        versioned_backup_name(path, kind, &self.opt.simple_backup_suffix)
    }

    fn create_empty_backup(&mut self, dest: &Path) -> Result<(), String> {
        let _ = fs::remove_file(dest);
        self.ensure_backup_dir(dest)?;
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o666)
            .open(dest)
            .map(|_| ())
            .map_err(|err| {
                format!(
                    "Can't create file {} : {}",
                    self.opt.quote(&dest.display().to_string()),
                    cli::io_error_text(&err)
                )
            })
    }

    fn copy_backup(&mut self, source: &Path, dest: &Path) -> Result<(), String> {
        self.ensure_backup_dir(dest)?;
        fs::copy(source, dest).map(|_| ()).map_err(|err| {
            format!(
                "Can't create file {} : {}",
                self.opt.quote(&dest.display().to_string()),
                cli::io_error_text(&err)
            )
        })
    }

    fn rename_backup(&mut self, source: &Path, dest: &Path) -> Result<(), String> {
        self.ensure_backup_dir(dest)?;
        match fs::rename(source, dest) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::CrossesDevices => {
                self.copy_backup(source, dest)?;
                fs::remove_file(source).map_err(|err| {
                    format!(
                        "Can't remove file {} : {}",
                        self.opt.quote(&source.display().to_string()),
                        cli::io_error_text(&err)
                    )
                })
            }
            Err(err) => Err(format!(
                "Can't rename file {} to {} : {}",
                self.opt.quote(&source.display().to_string()),
                self.opt.quote(&dest.display().to_string()),
                cli::io_error_text(&err)
            )),
        }
    }

    fn ensure_backup_dir(&self, dest: &Path) -> Result<(), String> {
        let Some(parent) = dest.parent() else {
            return Ok(());
        };
        if parent.as_os_str().is_empty() || parent.exists() {
            return Ok(());
        }
        fs::create_dir_all(parent).map_err(|err| {
            format!(
                "Can't create file {} : {}",
                self.opt.quote(&dest.display().to_string()),
                cli::io_error_text(&err)
            )
        })
    }

    fn queue_delete(&mut self, name: &str, backup: bool) -> Result<(), String> {
        let meta = match metadata_of(name, self.opt.follow_symlinks) {
            Ok(meta) => meta,
            Err(_) => return Ok(()),
        };
        let dev = meta.dev();
        let ino = meta.ino();
        self.remember(dev, ino, IdKind::DeleteLater);
        self.pending_deletes.push(PendingDelete {
            name: name.to_string(),
            dev,
            ino,
            kind: meta_kind(&meta),
            backup,
        });
        Ok(())
    }

    fn flush_deletes(&mut self) -> Result<(), String> {
        let pending = std::mem::take(&mut self.pending_deletes);
        for item in pending {
            if self.lookup(item.dev, item.ino) != Some(IdKind::DeleteLater) {
                continue;
            }
            if self.opt.verbosity == Verbosity::Verbose {
                let what = if item.kind == S_IFLNK {
                    "symbolic link"
                } else {
                    "file"
                };
                self.say(&format!("Removing {what} {}\n", self.opt.quote(&item.name)));
            }
            let previous = file_ids(&item.name, self.opt.follow_symlinks);
            if item.backup {
                self.create_backup(&item.name, previous.is_some(), false)?;
            } else if let Err(err) = fs::remove_file(&item.name)
                && err.kind() != io::ErrorKind::NotFound
            {
                return Err(format!(
                    "Can't remove file {} : {}",
                    self.opt.quote(&item.name),
                    cli::io_error_text(&err)
                ));
            }
            if let Some((dev, ino)) = previous {
                self.remember(dev, ino, IdKind::Overwritten);
            }
            self.removedirs(&item.name);
        }
        Ok(())
    }

    /// Drop empty parents of a deleted file. `.` and `..` are left alone.
    fn removedirs(&mut self, name: &str) {
        let bytes = name.as_bytes();
        let mut index = bytes.len();
        while index > 0 {
            index -= 1;
            if bytes[index] != b'/' || skip_removedir(bytes, index) {
                continue;
            }
            let dir = &name[..index];
            if fs::remove_dir(dir).is_ok() && self.opt.verbosity == Verbosity::Verbose {
                self.say(&format!(
                    "Removed empty directory {}\n",
                    self.opt.quote(dir)
                ));
            }
        }
    }

    fn note_replaced(&mut self, path: &str, previous: Option<(u64, u64)>) {
        if let Some((dev, ino)) = previous {
            self.remember(dev, ino, IdKind::Overwritten);
        }
        if let Some((dev, ino)) = file_ids(path, self.opt.follow_symlinks) {
            self.remember(dev, ino, IdKind::Created);
        }
    }

    fn conceal_deleted(&self, mut stat: StatBuf) -> StatBuf {
        if stat.ok && self.lookup(stat.dev, stat.ino) == Some(IdKind::DeleteLater) {
            stat.ok = false;
            stat.len = 0;
        }
        stat
    }

    fn lookup(&self, dev: u64, ino: u64) -> Option<IdKind> {
        self.files
            .iter()
            .find(|rec| rec.dev == dev && rec.ino == ino)
            .map(|rec| rec.kind)
    }

    fn remember(&mut self, dev: u64, ino: u64, kind: IdKind) {
        if let Some(rec) = self
            .files
            .iter_mut()
            .find(|rec| rec.dev == dev && rec.ino == ino)
        {
            rec.kind = kind;
            return;
        }
        self.files.push(FileRec { dev, ino, kind });
    }

    fn report_failure(
        &mut self,
        file: &FilePatch,
        outname: &Option<String>,
        applied: &Applied,
    ) -> Result<(), String> {
        if file.kind == DiffKind::Ed {
            return Ok(());
        }
        let plural = if applied.hunk_count == 1 {
            "hunk"
        } else {
            "hunks"
        };
        let what = if applied.skip { "ignored" } else { "FAILED" };
        self.say(&format!(
            "{} out of {} {plural} {what}",
            applied.failed, applied.hunk_count
        ));
        let disabled = outname.is_none() || self.opt.reject_file.as_deref() == Some("-");
        if disabled || self.opt.dry_run {
            self.say("\n");
            return Ok(());
        }
        let Some(out) = outname else {
            self.say("\n");
            return Ok(());
        };
        let rej = self.reject_path(out);
        self.say(&format!(
            " -- saving rejects to file {}\n",
            self.opt.quote(&rej)
        ));
        self.write_reject(&rej, &applied.reject)
    }

    fn reject_path(&self, outname: &str) -> String {
        if let Some(path) = &self.opt.reject_file
            && path != "-"
        {
            return path.clone();
        }
        let mut rej = format!("{outname}.rej");
        if rej.ends_with('~') {
            rej.pop();
            rej.push('#');
        }
        rej
    }

    fn write_reject(&mut self, path: &str, data: &[u8]) -> Result<(), String> {
        let append = self.reject_created.iter().any(|seen| seen == path);
        let result = if append {
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .and_then(|mut file| file.write_all(data))
        } else {
            self.place(Path::new(path), data, None).map(|_| ())
        };
        result.map_err(|err| format!("write error : {}", cli::io_error_text(&err)))?;
        if !append {
            self.reject_created.push(path.to_string());
        }
        Ok(())
    }

    fn apply_ed(
        &mut self,
        file: &FilePatch,
        pick: &Pick,
        outname: &Option<String>,
        info: &InputInfo,
    ) -> Result<(), String> {
        if pick.skip || self.opt.dry_run {
            return Ok(());
        }
        self.tmp_seq += 1;
        let dir = std::env::temp_dir();
        let script_path = dir.join(format!(
            "patch-rs-eds-{}-{}",
            std::process::id(),
            self.tmp_seq
        ));
        let out_path = dir.join(format!(
            "patch-rs-edo-{}-{}",
            std::process::id(),
            self.tmp_seq
        ));
        let mut body = file.ed_script.clone();
        if !body.ends_with(b"\n") {
            body.push(b'\n');
        }
        body.extend_from_slice(b"w\nq\n");
        fs::write(&script_path, &body).map_err(|err| {
            format!(
                "Can't create file {} : {}",
                self.opt.quote(&script_path.display().to_string()),
                cli::io_error_text(&err)
            )
        })?;
        let initial = if info.exists {
            info.data.as_slice()
        } else {
            b""
        };
        fs::write(&out_path, initial).map_err(|err| {
            format!(
                "Can't create file {} : {}",
                self.opt.quote(&out_path.display().to_string()),
                cli::io_error_text(&err)
            )
        })?;
        let script = File::open(&script_path).map_err(|_| "ed FAILED".to_string())?;
        let status = Command::new("ed")
            .arg("-")
            .arg(&out_path)
            .stdin(script)
            .status();
        let _ = fs::remove_file(&script_path);
        let status = status.map_err(|_| "ed FAILED".to_string())?;
        if !status.success() {
            let _ = fs::remove_file(&out_path);
            return Err("ed FAILED".to_string());
        }
        let data = fs::read(&out_path).unwrap_or_default();
        let _ = fs::remove_file(&out_path);
        if self.opt.output.is_some() {
            if let Some(out) = self.outfile.as_mut() {
                out.write_all(&data)
                    .map_err(|err| format!("write error : {}", cli::io_error_text(&err)))?;
            }
            return Ok(());
        }
        if let Some(name) = outname {
            let mode = if info.exists { Some(info.mode) } else { None };
            self.reject_newline_name(Path::new(name))?;
            self.place(Path::new(name), &data, mode)
                .map_err(|err| format!("write error : {}", cli::io_error_text(&err)))?;
        }
        Ok(())
    }

    fn place(&mut self, path: &Path, data: &[u8], mode: Option<u32>) -> Result<(), io::Error> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
            && !parent.exists()
        {
            fs::create_dir_all(parent)?;
        }
        self.tmp_seq += 1;
        let tmp_name = format!(".patch-rs-{}-{}", std::process::id(), self.tmp_seq);
        let tmp = match path.file_name() {
            Some(_) => path.with_file_name(tmp_name),
            None => PathBuf::from(tmp_name),
        };
        let write_result = (|| -> io::Result<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&tmp)?;
            file.write_all(data)?;
            if let Some(mode) = mode {
                file.set_permissions(fs::Permissions::from_mode(mode))?;
            }
            Ok(())
        })();
        if let Err(err) = write_result {
            let _ = fs::remove_file(&tmp);
            return Err(err);
        }
        if let Err(err) = fs::rename(&tmp, path) {
            let _ = fs::remove_file(&tmp);
            return Err(err);
        }
        Ok(())
    }

    fn write_output_bytes(
        &mut self,
        path: &Path,
        data: &[u8],
        mode: Option<u32>,
    ) -> Result<(), String> {
        if let Some(out) = self.outfile.as_mut() {
            return out
                .write_all(data)
                .map_err(|err| format!("write error : {}", cli::io_error_text(&err)));
        }
        self.place(path, data, mode)
            .map_err(|err| format!("write error : {}", cli::io_error_text(&err)))
    }

    fn reject_newline_name(&mut self, path: &Path) -> Result<(), String> {
        if !last_has_newline(path) {
            return Ok(());
        }
        self.tmp_seq += 1;
        let tmp = gnu_temp_label(path, self.tmp_seq);
        Err(format!(
            "Can't rename file {} to {} : Invalid byte sequence",
            self.opt.quote(&tmp),
            self.opt.quote(&path.display().to_string())
        ))
    }

    fn open_output(&self, path: &Path) -> Result<File, String> {
        if last_has_newline(path) {
            return Err(format!(
                "Can't create file {} : Invalid byte sequence",
                self.opt.quote(&path.display().to_string())
            ));
        }
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
            && !parent.exists()
            && !self.opt.dry_run
        {
            fs::create_dir_all(parent).map_err(|err| {
                format!(
                    "Can't create file {} : {}",
                    self.opt.quote(&path.display().to_string()),
                    cli::io_error_text(&err)
                )
            })?;
        }
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .map_err(|err| {
                format!(
                    "Can't create file {} : {}",
                    self.opt.quote(&path.display().to_string()),
                    cli::io_error_text(&err)
                )
            })
    }

    fn read_patch(&self) -> Result<Vec<u8>, String> {
        let name = self.opt.patch_file.as_ref().or(self.opt.input.as_ref());
        match name {
            None => read_stdin(),
            Some(path) if path.is_empty() || path == "-" => read_stdin(),
            Some(path) => fs::read(path).map_err(|err| {
                format!(
                    "Can't open patch file {} : {}",
                    self.opt.quote(path),
                    cli::io_error_text(&err)
                )
            }),
        }
    }

    fn say(&mut self, text: &str) {
        self.write_stdout(text.as_bytes());
    }

    fn write_stdout(&mut self, bytes: &[u8]) {
        let mut out = io::stdout().lock();
        let _ = out.write_all(bytes);
        let _ = out.flush();
    }

    fn fatal_print(&self, message: &str) {
        let _ = io::stdout().flush();
        let mut err = io::stderr().lock();
        let _ = writeln!(err, "{}: **** {message}", self.opt.program);
        let _ = err.flush();
    }
}

struct Located {
    where_line: Option<i64>,
    fuzz: i64,
}

struct LocateCursor<'a> {
    hunks: &'a mut [Hunk],
    lines: &'a [Vec<u8>],
    frozen: i64,
    in_offset: &'a mut i64,
    reverse: &'a mut bool,
    cli_reverse: bool,
    apply_anyway: &'a mut bool,
    skip: &'a mut bool,
    mismatch: &'a mut bool,
}

fn reject_style(opt: &Options, kind: DiffKind) -> RejectStyle {
    let unified = match opt.reject_format {
        RejectFormat::Unified => true,
        RejectFormat::Context => false,
        RejectFormat::Unspecified => kind == DiffKind::Unified,
    };
    if unified {
        RejectStyle::Unified
    } else {
        let new_style = !matches!(kind, DiffKind::Context | DiffKind::Normal | DiffKind::Ed);
        RejectStyle::Context { new_style }
    }
}

fn reject_names(names: &Names, reverse: bool) -> RejectNames<'_> {
    if reverse {
        RejectNames {
            index: names.index.as_deref(),
            old_name: names.new.as_deref().unwrap_or(""),
            new_name: names.old.as_deref().unwrap_or(""),
            old_time: &names.new_time,
            new_time: &names.old_time,
        }
    } else {
        RejectNames {
            index: names.index.as_deref(),
            old_name: names.old.as_deref().unwrap_or(""),
            new_name: names.new.as_deref().unwrap_or(""),
            old_time: &names.old_time,
            new_time: &names.new_time,
        }
    }
}

fn side_timestr(file: &FilePatch, which_new: bool) -> &str {
    if which_new {
        &file.new_timestr
    } else {
        &file.old_timestr
    }
}

fn file_mtime(meta: &fs::Metadata) -> Option<(i64, u32)> {
    let modified = meta.modified().ok()?;
    let dur = modified.duration_since(UNIX_EPOCH).ok()?;
    Some((dur.as_secs() as i64, dur.subsec_nanos()))
}

fn set_mtime(path: &Path, sec: i64, nsec: u32) -> io::Result<()> {
    if sec < 0 {
        return Err(io::Error::other("time out of range"));
    }
    let when = UNIX_EPOCH.checked_add(Duration::new(sec as u64, nsec)).ok_or_else(|| {
        io::Error::other("time out of range")
    })?;
    let file = File::open(path)?;
    file.set_modified(when)
}

fn tz_is_utc() -> bool {
    matches!(
        std::env::var("TZ").as_deref(),
        Ok("UTC" | "UTC0" | "GMT" | "GMT0" | "UCT")
    )
}

/// Parse a unified or context header time. A numeric zone is absolute.
/// A zone-less time is UTC when `--set-utc` is set or `TZ` is UTC.
fn header_time(timestr: &str, assume_utc: bool) -> Option<(i64, u32)> {
    let bytes = timestr.trim_start_matches([' ', '\t']).as_bytes();
    if bytes.len() < 19 {
        return None;
    }
    let digit = |i: usize| bytes.get(i).is_some_and(u8::is_ascii_digit);
    if !(digit(0)
        && digit(1)
        && digit(2)
        && digit(3)
        && bytes[4] == b'-'
        && digit(5)
        && digit(6)
        && bytes[7] == b'-'
        && digit(8)
        && digit(9)
        && matches!(bytes[10], b' ' | b'T')
        && digit(11)
        && digit(12)
        && bytes[13] == b':'
        && digit(14)
        && digit(15)
        && bytes[16] == b':'
        && digit(17)
        && digit(18))
    {
        return None;
    }
    let num = |a: usize, b: usize| {
        std::str::from_utf8(&bytes[a..b])
            .ok()
            .and_then(|text| text.parse::<i64>().ok())
    };
    let year = num(0, 4)?;
    let month = num(5, 7)?;
    let day = num(8, 10)?;
    let hour = num(11, 13)?;
    let min = num(14, 16)?;
    let sec = num(17, 19)?;
    let mut i = 19usize;
    let mut nsec = 0u32;
    if bytes.get(i) == Some(&b'.') {
        i += 1;
        let mut digits = 0u32;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            if digits < 9 {
                nsec = nsec * 10 + u32::from(bytes[i] - b'0');
                digits += 1;
            }
            i += 1;
        }
        while digits < 9 {
            nsec *= 10;
            digits += 1;
        }
    }
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
        i += 1;
    }
    let offset = if i < bytes.len() && matches!(bytes[i], b'+' | b'-' | b'Z' | b'z') {
        parse_zone(&bytes[i..])?
    } else if assume_utc {
        0
    } else {
        return None;
    };
    let secs = civil_utc_secs(year, month, day, hour, min, sec)?;
    Some((secs - offset, nsec))
}

fn parse_zone(bytes: &[u8]) -> Option<i64> {
    if bytes.first().is_some_and(|b| *b == b'Z' || *b == b'z') {
        return Some(0);
    }
    let sign = match bytes.first() {
        Some(b'+') => 1i64,
        Some(b'-') => -1,
        _ => return None,
    };
    let b = &bytes[1..];
    if b.len() < 2 || !b[0].is_ascii_digit() || !b[1].is_ascii_digit() {
        return None;
    }
    let hour = i64::from(b[0] - b'0') * 10 + i64::from(b[1] - b'0');
    let mut i = 2usize;
    let mut min = 0i64;
    let mut sec = 0i64;
    if b.get(i) == Some(&b':') {
        i += 1;
    }
    if i + 1 < b.len() && b[i].is_ascii_digit() && b[i + 1].is_ascii_digit() {
        min = i64::from(b[i] - b'0') * 10 + i64::from(b[i + 1] - b'0');
        i += 2;
        if b.get(i) == Some(&b':') {
            i += 1;
        }
        if i + 1 < b.len() && b[i].is_ascii_digit() && b[i + 1].is_ascii_digit() {
            sec = i64::from(b[i] - b'0') * 10 + i64::from(b[i + 1] - b'0');
        }
    }
    Some(sign * (hour * 3600 + min * 60 + sec))
}

fn civil_utc_secs(year: i64, month: i64, day: i64, hour: i64, min: i64, sec: i64) -> Option<i64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + hour * 3600 + min * 60 + sec)
}

fn says(file: &FilePatch, which_new: bool) -> u8 {
    if which_new {
        file.new_nonexistent
    } else {
        file.old_nonexistent
    }
}

/// Whether this patch looks reversed, using the same test as `maybe_reverse`.
///
/// `says` is 0 when that side is present, 1 when it is absent, and 2 when a
/// diff --git index says it is empty. An empty file the patch already marks
/// empty is not a reversal. `select_name` calls this first so it can reject
/// a reversed delete of a missing absolute or `..` name before `maybe_reverse`
/// prints "Assuming -R." and creates the file (CVE-2026-115207).
fn reversal_looks(file: &FilePatch, nonexistent: bool, is_empty: bool, reverse: bool) -> bool {
    let left = says(file, reverse ^ nonexistent);
    let right = says(file, !reverse ^ nonexistent);
    if is_empty && left == 1 && right == 2 {
        return false;
    }
    u8::from(!is_empty) < says(file, reverse ^ is_empty)
}

fn side_name(names: &Names, which_new: bool) -> Option<String> {
    if which_new {
        names.new.clone()
    } else {
        names.old.clone()
    }
}

fn side_mode(file: &FilePatch, which_new: bool) -> u32 {
    if which_new {
        file.new_mode
    } else {
        file.old_mode
    }
}

fn patch_file_type(file: &FilePatch) -> u32 {
    let old = file.old_mode & S_IFMT;
    let new = file.new_mode & S_IFMT;
    if old != 0 {
        if new != 0 && old != new { 0 } else { old }
    } else if new != 0 {
        new
    } else {
        S_IFREG
    }
}

fn meta_kind(meta: &fs::Metadata) -> u32 {
    let kind = meta.file_type();
    if kind.is_symlink() {
        S_IFLNK
    } else if kind.is_file() {
        S_IFREG
    } else if kind.is_dir() {
        S_IFDIR
    } else {
        0
    }
}

fn meta_perm(meta: &fs::Metadata) -> u32 {
    meta.permissions().mode() & 0o7777
}

fn inspect(path: &str, follow: bool) -> StatBuf {
    let meta = if follow {
        fs::metadata(path)
    } else {
        fs::symlink_metadata(path)
    };
    match meta {
        Ok(meta) => StatBuf {
            ok: true,
            len: meta.len(),
            kind: meta_kind(&meta),
            dev: meta.dev(),
            ino: meta.ino(),
        },
        Err(_) => StatBuf::missing(),
    }
}

impl StatBuf {
    fn missing() -> Self {
        Self {
            ok: false,
            len: 0,
            kind: S_IFREG,
            dev: 0,
            ino: 0,
        }
    }
}

fn prefix_components(filename: &str, checkdirs: bool) -> i64 {
    let bytes = filename.as_bytes();
    if bytes.len() < 2 {
        return 0;
    }
    let mut count = 0i64;
    let mut index = 1usize;
    while index < bytes.len() {
        if bytes[index] == b'/' && bytes[index - 1] != b'/' {
            if checkdirs {
                let prefix = &filename[..index];
                match fs::metadata(prefix) {
                    Ok(meta) if meta.is_dir() => {}
                    _ => break,
                }
            }
            count += 1;
        }
        index += 1;
    }
    count
}

fn basename_len(name: &str) -> i64 {
    match name.rfind('/') {
        Some(pos) => (name.len() - pos - 1) as i64,
        None => name.len() as i64,
    }
}

fn c_octal(value: u32) -> String {
    if value == 0 {
        "0".to_string()
    } else {
        format!("0{value:o}")
    }
}

fn cwd_is_root() -> bool {
    let Ok(dot) = fs::metadata(".") else {
        return false;
    };
    let Ok(root) = fs::metadata("/") else {
        return false;
    };
    dot.dev() == root.dev() && dot.ino() == root.ino()
}

fn can_write(path: &Path) -> bool {
    OpenOptions::new().write(true).open(path).is_ok()
}

fn read_stdin() -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    io::stdin()
        .read_to_end(&mut buf)
        .map_err(|err| format!("read error : {}", cli::io_error_text(&err)))?;
    Ok(buf)
}

fn first_nul_line(input: &[u8]) -> Option<i64> {
    let mut line = 1i64;
    for &byte in input {
        if byte == 0 {
            return Some(line);
        }
        if byte == b'\n' {
            line += 1;
        }
    }
    None
}

fn metadata_of(path: &str, follow: bool) -> io::Result<fs::Metadata> {
    if follow {
        fs::metadata(path)
    } else {
        fs::symlink_metadata(path)
    }
}

fn file_ids(path: &str, follow: bool) -> Option<(u64, u64)> {
    metadata_of(path, follow)
        .ok()
        .map(|meta| (meta.dev(), meta.ino()))
}

fn manual_backup_name(
    path: &str,
    prefix: Option<&str>,
    base_prefix: Option<&str>,
    suffix: Option<&str>,
) -> String {
    let (dir, base) = split_dir_base(path);
    format!(
        "{}{dir}{}{base}{}",
        prefix.unwrap_or(""),
        base_prefix.unwrap_or(""),
        suffix.unwrap_or("")
    )
}

/// `simple` appends the suffix. `existing` does that too unless `base.~N~`
/// is already present. `numbered` always uses the next `base.~N~`.
fn versioned_backup_name(path: &str, kind: BackupType, simple_suffix: &str) -> String {
    let (dir, base) = split_dir_base(path);
    if kind == BackupType::Simple {
        return format!("{path}{simple_suffix}");
    }
    let numbered = next_numbered_name(dir, base);
    if kind == BackupType::Existing && numbered.is_none() {
        format!("{path}{simple_suffix}")
    } else {
        let name = numbered.unwrap_or_else(|| format!("{base}.~1~"));
        format!("{dir}{name}")
    }
}

/// True when the slash at `index` ends a `.` or `..` component, or a doubled slash.
fn skip_removedir(bytes: &[u8], index: usize) -> bool {
    if index == 0 || bytes[index - 1] == b'/' {
        return true;
    }
    if bytes[index - 1] != b'.' {
        return false;
    }
    if index == 1 || bytes[index - 2] == b'/' {
        return true;
    }
    bytes[index - 2] == b'.' && (index == 2 || bytes[index - 3] == b'/')
}

fn split_dir_base(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(index) => (&path[..=index], &path[index + 1..]),
        None => ("", path),
    }
}

/// The highest `base.~N~` in `dir`, incremented. `None` when no numbered backup exists.
fn next_numbered_name(dir: &str, base: &str) -> Option<String> {
    let folder = if dir.is_empty() {
        "."
    } else {
        dir.trim_end_matches('/')
    };
    let entries = fs::read_dir(folder).ok()?;
    let mut best: Option<String> = None;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(version) = numbered_version(base, &name) else {
            continue;
        };
        best = Some(match best {
            Some(prev) if version_less(&version, &prev) => prev,
            _ => version,
        });
    }
    best.map(|version| format!("{base}.~{}~", increment_digits(&version)))
}

fn numbered_version(base: &str, name: &str) -> Option<String> {
    let rest = name.strip_prefix(base)?;
    let rest = rest.strip_prefix(".~")?;
    let digits = rest.strip_suffix('~')?;
    let bytes = digits.as_bytes();
    if bytes.is_empty() || bytes[0] < b'1' || bytes[0] > b'9' {
        return None;
    }
    if !bytes.iter().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(digits.to_string())
}

fn version_less(left: &str, right: &str) -> bool {
    left.len() < right.len() || (left.len() == right.len() && left < right)
}

fn increment_digits(digits: &str) -> String {
    let mut bytes = digits.as_bytes().to_vec();
    let mut index = bytes.len();
    while index > 0 {
        index -= 1;
        if bytes[index] < b'9' {
            bytes[index] += 1;
            return String::from_utf8(bytes).expect("digits");
        }
        bytes[index] = b'0';
    }
    bytes.insert(0, b'1');
    String::from_utf8(bytes).expect("digits")
}

fn last_has_newline(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.to_string_lossy().contains('\n'))
}

/// GNU output temps drop newlines from the final component and append `.o` plus
/// six characters. `bad-filenames` rewrites that shape to `ab.*`.
fn gnu_temp_label(path: &Path, seq: u64) -> String {
    let name = path
        .file_name()
        .map(|text| text.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stripped: String = name.chars().filter(|&ch| ch != '\n').collect();
    let file = format!("{stripped}.o{seq:06}");
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => {
            parent.join(file).to_string_lossy().into_owned()
        }
        _ => file,
    }
}

#[cfg(test)]
mod tests {
    use super::run;
    use crate::cli::{self, ProcessEnv};
    use std::fs;
    use std::io::{self, Read, Write};
    use std::os::unix::io::FromRawFd;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    fn clean_env() -> ProcessEnv {
        ProcessEnv {
            posixly_correct: false,
            quoting_style: None,
            patch_get: None,
            simple_backup_suffix: None,
            patch_version_control: None,
            version_control: None,
        }
    }

    fn apply_in_dir(patch: &Path, args: &[&str]) -> (i32, String) {
        let mut argv = vec!["patch".to_string()];
        argv.extend(args.iter().copied().map(str::to_string));
        argv.push("-i".to_string());
        argv.push(patch.display().to_string());
        let opt = cli::parse_with(&argv, &clean_env()).unwrap_or_else(|_| panic!("parse failed"));
        let mut status = 0;
        let output = capture_stdout(|| {
            status = run(&opt);
        });
        (status, output)
    }

    /// `say` writes to the real stdout fd. The caller holds the test lock.
    fn capture_stdout(body: impl FnOnce()) -> String {
        let capture = StdoutCapture::start();
        body();
        capture.finish()
    }

    struct StdoutCapture {
        saved: i32,
        read: i32,
        active: bool,
    }

    impl StdoutCapture {
        fn start() -> Self {
            let _ = io::stdout().flush();
            let mut fds = [0i32; 2];
            assert_eq!(unsafe { pipe(fds.as_mut_ptr()) }, 0, "pipe");
            let saved = unsafe { dup(1) };
            assert!(saved >= 0, "dup stdout");
            assert_eq!(unsafe { dup2(fds[1], 1) }, 1, "redirect stdout");
            assert_eq!(unsafe { close(fds[1]) }, 0, "close pipe write");
            Self {
                saved,
                read: fds[0],
                active: true,
            }
        }

        fn finish(mut self) -> String {
            let _ = io::stdout().flush();
            self.restore();
            let fd = self.read;
            self.read = -1;
            let mut file = unsafe { fs::File::from_raw_fd(fd) };
            let mut out = Vec::new();
            file.read_to_end(&mut out).unwrap();
            String::from_utf8(out)
                .unwrap_or_else(|err| String::from_utf8_lossy(err.as_bytes()).into_owned())
        }

        fn restore(&mut self) {
            if !self.active {
                return;
            }
            unsafe {
                dup2(self.saved, 1);
                close(self.saved);
            }
            self.active = false;
        }
    }

    impl Drop for StdoutCapture {
        fn drop(&mut self) {
            self.restore();
            if self.read >= 0 {
                unsafe { close(self.read) };
                self.read = -1;
            }
        }
    }

    unsafe extern "C" {
        fn pipe(fds: *mut i32) -> i32;
        fn dup(fd: i32) -> i32;
        fn dup2(oldfd: i32, newfd: i32) -> i32;
        fn close(fd: i32) -> i32;
    }

    struct CwdGuard(PathBuf);

    impl Drop for CwdGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.0);
        }
    }

    struct TmpGuard(PathBuf);

    impl Drop for TmpGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn reverse_delete_rejects_out_of_tree_names() {
        static LOCK: Mutex<()> = Mutex::new(());
        let _lock = LOCK.lock().unwrap_or_else(|err| err.into_inner());

        let root = std::env::temp_dir().join(format!("patch-rs-cve-115207-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let work = root.join("w");
        fs::create_dir_all(&work).unwrap();
        let original = std::env::current_dir().unwrap();
        let _tmp = TmpGuard(root.clone());
        let _cwd = CwdGuard(original);
        std::env::set_current_dir(&work).unwrap();

        let abs = root.join("absout");
        let abs_text = abs.display().to_string();
        let patch = root.join("q.diff");
        fs::write(
            &patch,
            format!("--- {abs_text}\n+++ {abs_text}\n@@ -1 +0,0 @@\n-data\n"),
        )
        .unwrap();
        let (status, output) = apply_in_dir(&patch, &["-p0", "-t"]);
        let expected = format!(
            "\
Ignoring potentially dangerous file name {abs_text}
can't find file to patch at input line 3
Perhaps you used the wrong -p or --strip option?
The text leading up to this was:
--------------------------
|--- {abs_text}
|+++ {abs_text}
--------------------------
No file to patch.  Skipping patch.
1 out of 1 hunk ignored
"
        );
        assert_eq!(output, expected);
        assert_eq!(status, 1);
        assert!(!abs.exists(), "absolute out-of-tree file was created");
        assert!(!root.join("absout.rej").exists());
        assert!(!root.join("absout.orig").exists());

        let rel_patch = root.join("r.diff");
        fs::write(&rel_patch, "--- relout\n+++ relout\n@@ -1 +0,0 @@\n-data\n").unwrap();
        let (status, output) = apply_in_dir(&rel_patch, &["-p0", "-t"]);
        assert_eq!(status, 0, "{output}");
        assert_eq!(fs::read(work.join("relout")).unwrap(), b"data\n");
        assert!(output.contains("Assuming -R.\n"));
        assert!(output.contains("patching file relout\n"));
        assert!(!output.contains("dangerous"));

        let dot_patch = root.join("d.diff");
        fs::write(
            &dot_patch,
            "--- ../dotout\n+++ ../dotout\n@@ -1 +0,0 @@\n-data\n",
        )
        .unwrap();
        let (status, output) = apply_in_dir(&dot_patch, &["-p0", "-t"]);
        assert_eq!(status, 1, "{output}");
        assert!(!root.join("dotout").exists());
        assert!(output.starts_with("Ignoring potentially dangerous file name ../dotout\n"));
        assert!(!output.contains("patching file"));

        // GNU bad-filenames: `+++ ../g` on a normal edit is not a create.
        let edit = root.join("edit.diff");
        fs::write(&edit, "--- f\n+++ ../g\n@@ -1 +1 @@\n-1\n+1\n").unwrap();
        let (status, output) = apply_in_dir(&edit, &["-f", "-p0", "--dry-run"]);
        assert_eq!(status, 1, "{output}");
        assert!(!output.contains("dangerous"), "{output}");
        assert!(output.contains("can't find file to patch"));
        assert!(!root.join("g").exists());

        fs::write(work.join("f"), "1\n").unwrap();
        let (status, output) = apply_in_dir(&edit, &["-f", "-p0", "--dry-run"]);
        assert_eq!(status, 0, "{output}");
        assert_eq!(output, "checking file f\n");
        assert_eq!(fs::read(work.join("f")).unwrap(), b"1\n");
        assert!(!root.join("g").exists());
    }
}
