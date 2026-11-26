//! Parse unified, context, normal, and ed diffs the way GNU patch does.
//!
//! Hunks are stored in the same internal shape GNU patch uses: a `*` header,
//! the old lines, a `=` separator, the new lines, and a `^` stopper.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffKind {
    Unified,
    Context,
    NewContext,
    Normal,
    Ed,
    GitBinary,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HunkLine {
    pub ch: u8,
    pub text: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    pub old_first: i64,
    pub new_first: i64,
    pub ptrn_lines: i64,
    pub repl_lines: i64,
    pub prefix_context: i64,
    pub suffix_context: i64,
    pub lines: Vec<HunkLine>,
    /// Text after `@@`, including the leading space, without a newline.
    pub c_function: Vec<u8>,
}

impl Hunk {
    pub fn end(&self) -> i64 {
        self.lines.len() as i64 - 2
    }

    pub fn ch(&self, index: i64) -> u8 {
        self.lines
            .get(index as usize)
            .map(|line| line.ch)
            .unwrap_or(b'^')
    }

    pub fn text(&self, index: i64) -> &[u8] {
        self.lines
            .get(index as usize)
            .map(|line| line.text.as_slice())
            .unwrap_or(b"")
    }

    /// Swap old and new sides, matching `pch_swap`.
    ///
    /// Prefix and suffix context stay put, as they do in GNU patch.
    pub fn swap(&mut self) {
        std::mem::swap(&mut self.old_first, &mut self.new_first);

        let end = self.end();
        let mut swapped = Vec::new();
        let mut i = self.ptrn_lines + 1;
        let blank = self.ch(i) == b'\n';
        if blank {
            i += 1;
        }
        while i <= end {
            let mut ch = self.ch(i);
            if ch == b'+' {
                ch = b'-';
            }
            let mut text = self.text(i).to_vec();
            if swapped.is_empty() {
                ch = b'*';
                for byte in &mut text {
                    if *byte == b'-' {
                        *byte = b'*';
                    }
                }
            }
            swapped.push(HunkLine { ch, text });
            i += 1;
        }
        if blank {
            let at = self.ptrn_lines + 1;
            swapped.push(HunkLine {
                ch: self.ch(at),
                text: self.text(at).to_vec(),
            });
        }
        let mut old_i = 0i64;
        while swapped.len() as i64 <= end {
            let mut ch = self.ch(old_i);
            let mut text = self.text(old_i).to_vec();
            if old_i == 0 {
                ch = b'=';
                for byte in &mut text {
                    if *byte == b'*' {
                        *byte = b'-';
                    }
                }
            } else if ch == b'-' {
                ch = b'+';
            }
            swapped.push(HunkLine { ch, text });
            old_i += 1;
        }
        swapped.push(HunkLine {
            ch: b'^',
            text: Vec::new(),
        });
        std::mem::swap(&mut self.ptrn_lines, &mut self.repl_lines);
        self.lines = swapped;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilePatch {
    pub kind: DiffKind,
    pub old_name: Option<String>,
    pub new_name: Option<String>,
    pub index_name: Option<String>,
    pub old_timestr: String,
    pub new_timestr: String,
    pub old_nonexistent: u8,
    pub new_nonexistent: u8,
    /// Timestamp seconds; -1 means absent. Epoch (0) marks `/dev/null`.
    pub old_stamp: i64,
    pub new_stamp: i64,
    pub old_mode: u32,
    pub new_mode: u32,
    pub git: bool,
    pub rename: bool,
    pub copy: bool,
    pub hunks: Vec<Hunk>,
    /// Ed script, without the `w`/`q` that patch appends itself.
    pub ed_script: Vec<u8>,
    /// Bytes before the first hunk, printed with a `|` when no file is chosen.
    pub leading: Vec<u8>,
    /// 1-based input line of the first hunk (or of the diff header).
    pub report_line: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parsed {
    pub files: Vec<FilePatch>,
    pub only_garbage: bool,
    pub trailing_garbage: bool,
}

#[derive(Clone, Debug)]
struct SrcLine {
    num: i64,
    raw: Vec<u8>,
    nl: bool,
}

pub fn parse_patch(input: &[u8]) -> Result<Parsed, String> {
    let lines = split_lines(input);
    if lines.is_empty() {
        return Ok(Parsed {
            files: Vec::new(),
            only_garbage: false,
            trailing_garbage: false,
        });
    }
    let mut files = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        if lines[index..].iter().all(|line| line.raw.is_empty()) {
            break;
        }
        let (file, next) = parse_one(&lines, index)?;
        if file.is_none() && files.is_empty() {
            return Ok(Parsed {
                files,
                only_garbage: true,
                trailing_garbage: false,
            });
        }
        if file.is_none() {
            return Ok(Parsed {
                files,
                only_garbage: false,
                trailing_garbage: true,
            });
        }
        files.push(file.unwrap());
        if next <= index {
            break;
        }
        index = next;
    }
    Ok(Parsed {
        files,
        only_garbage: false,
        trailing_garbage: false,
    })
}

fn split_lines(input: &[u8]) -> Vec<SrcLine> {
    let mut lines = Vec::new();
    if input.is_empty() {
        return lines;
    }
    let mut start = 0usize;
    let mut num = 1i64;
    for (i, &byte) in input.iter().enumerate() {
        if byte == b'\n' {
            lines.push(SrcLine {
                num,
                raw: input[start..i].to_vec(),
                nl: true,
            });
            start = i + 1;
            num += 1;
        }
    }
    if start < input.len() {
        lines.push(SrcLine {
            num,
            raw: input[start..].to_vec(),
            nl: false,
        });
    }
    lines
}

fn line_str(line: &SrcLine) -> String {
    String::from_utf8_lossy(&line.raw).into_owned()
}

fn starts_with(line: &SrcLine, prefix: &str) -> bool {
    line.raw.starts_with(prefix.as_bytes())
}

fn parse_one(lines: &[SrcLine], start: usize) -> Result<(Option<FilePatch>, usize), String> {
    let mut index = start;
    let mut old_name = None;
    let mut new_name = None;
    let mut index_name = None;
    let mut old_timestr = String::new();
    let mut new_timestr = String::new();
    let mut old_stamp = -1i64;
    let mut new_stamp = -1i64;
    let mut old_mode = 0u32;
    let mut new_mode = 0u32;
    let mut old_nonexistent = 0u8;
    let mut new_nonexistent = 0u8;
    let mut git = false;
    let mut rename = false;
    let mut copy = false;
    let mut extended = false;
    let mut kind = None;
    let mut hunks = Vec::new();
    let mut ed_script = Vec::new();
    let mut saw_header = false;

    while index < lines.len() {
        let line = &lines[index];
        let text = line_str(line);
        let trimmed = trim_indent(&text);

        if trimmed.starts_with("diff --git ") {
            if saw_header || extended || !hunks.is_empty() {
                break;
            }
            if let Some((old, new)) = git_pair(trimmed.as_bytes()) {
                old_name = Some(old);
                new_name = Some(new);
            }
            git = true;
            saw_header = true;
            index += 1;
            continue;
        }
        if git && trimmed.starts_with("index ") {
            if let Some((old_sha, new_sha, mode)) = parse_index(trimmed.as_bytes()) {
                old_nonexistent = sha_nonexistent(&old_sha);
                new_nonexistent = sha_nonexistent(&new_sha);
                if let Some(mode) = mode {
                    old_mode = mode;
                    new_mode = mode;
                }
                extended = true;
            }
            index += 1;
            continue;
        }
        if git {
            if let Some(rest) = trimmed.strip_prefix("new file mode ") {
                new_mode = fetch_mode(rest.as_bytes());
                old_nonexistent = 2;
                extended = true;
                index += 1;
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("deleted file mode ") {
                old_mode = fetch_mode(rest.as_bytes());
                new_nonexistent = 2;
                extended = true;
                index += 1;
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("old mode ") {
                old_mode = fetch_mode(rest.as_bytes());
                extended = true;
                index += 1;
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("new mode ") {
                new_mode = fetch_mode(rest.as_bytes());
                extended = true;
                index += 1;
                continue;
            }
        }
        if git && (trimmed.starts_with("rename from ") || trimmed.starts_with("rename to ")) {
            rename = true;
            extended = true;
            index += 1;
            continue;
        }
        if git && (trimmed.starts_with("copy from ") || trimmed.starts_with("copy to ")) {
            copy = true;
            extended = true;
            index += 1;
            continue;
        }
        if git && trimmed.starts_with("GIT binary patch") {
            kind = Some(DiffKind::GitBinary);
            index += 1;
            while index < lines.len() && !starts_with(&lines[index], "diff --git ") {
                index += 1;
            }
            break;
        }
        if let Some(rest) = trimmed.strip_prefix("Index:") {
            let (name, _) = fetch_name(rest.as_bytes());
            index_name = name;
            saw_header = true;
            index += 1;
            continue;
        }
        if trimmed.starts_with("Prereq:") {
            index += 1;
            continue;
        }
        if is_minus_header(trimmed.as_bytes()) {
            if kind.is_some() || !hunks.is_empty() {
                break;
            }
            // `+++` is stored first and swapped when `@@` is seen, matching
            // GNU. A `---` line is the name that becomes the old name.
            let (name, timestr, stamp) = fetch_name_stamp(trimmed[4..].as_bytes());
            new_name = name;
            new_timestr = timestr;
            new_stamp = stamp;
            saw_header = true;
            index += 1;
            continue;
        }
        if is_plus_header(trimmed.as_bytes()) {
            let (name, timestr, stamp) = fetch_name_stamp(trimmed[4..].as_bytes());
            old_name = name;
            old_timestr = timestr;
            old_stamp = stamp;
            saw_header = true;
            index += 1;
            continue;
        }
        if is_star_header(trimmed.as_bytes()) {
            let (name, timestr, stamp) = fetch_name_stamp(trimmed[4..].as_bytes());
            old_name = name;
            old_timestr = timestr;
            old_stamp = stamp;
            saw_header = true;
            index += 1;
            continue;
        }
        if trimmed.starts_with("@@ -") {
            std::mem::swap(&mut old_name, &mut new_name);
            std::mem::swap(&mut old_timestr, &mut new_timestr);
            std::mem::swap(&mut old_stamp, &mut new_stamp);
            kind = Some(DiffKind::Unified);
            note_range_nonexistent(
                trimmed.as_bytes(),
                old_stamp,
                new_stamp,
                &mut old_nonexistent,
                &mut new_nonexistent,
            );
            break;
        }
        if trimmed.starts_with("***************") || looks_like_context_range(trimmed.as_bytes()) {
            kind = Some(DiffKind::Context);
            break;
        }
        if is_normal_command(trimmed.as_bytes()) {
            // Look ahead a little: `<` / `>` means normal, a lone `.` means ed.
            if looks_normal_body(lines, index) {
                kind = Some(DiffKind::Normal);
                break;
            }
            if looks_ed_body(lines, index) {
                kind = Some(DiffKind::Ed);
                break;
            }
        }
        if is_ed_command(trimmed.as_bytes()) && looks_ed_body(lines, index) {
            kind = Some(DiffKind::Ed);
            break;
        }
        if trimmed.starts_with("diff ") {
            saw_header = true;
            index += 1;
            continue;
        }
        // Leading commentary before a real header.
        index += 1;
        if index - start > lines.len() {
            break;
        }
    }

    if kind.is_none() {
        if extended || git {
            kind = Some(DiffKind::Unified);
        } else if saw_header {
            // Headers without a hunk are still a (empty) diff when they came
            // from git. A bare `---` with nothing after it is garbage.
            if !git {
                return Ok((None, start));
            }
            kind = Some(DiffKind::Unified);
        } else {
            return Ok((None, start));
        }
    }

    let kind = kind.unwrap();
    let hunk_at = index;
    let report_line = lines.get(hunk_at).map(|line| line.num).unwrap_or(1);
    let leading = join_lines(&lines[start..hunk_at.min(lines.len())]);

    match kind {
        DiffKind::Unified => {
            while index < lines.len() && line_str(&lines[index]).trim_start().starts_with("@@ -") {
                let (hunk, next) = parse_unified_hunk(lines, index)?;
                hunks.push(hunk);
                index = next;
            }
        }
        DiffKind::Context | DiffKind::NewContext => {
            let (parsed, next, new_context) = parse_context_hunks(lines, index)?;
            hunks = parsed;
            index = next;
            let kind = if new_context {
                DiffKind::NewContext
            } else {
                kind
            };
            return Ok((
                Some(finish(
                    kind,
                    old_name,
                    new_name,
                    index_name,
                    old_timestr,
                    new_timestr,
                    old_nonexistent,
                    new_nonexistent,
                    old_stamp,
                    new_stamp,
                    old_mode,
                    new_mode,
                    git,
                    rename,
                    copy,
                    hunks,
                    ed_script,
                    leading,
                    report_line,
                )),
                index,
            ));
        }
        DiffKind::Normal => {
            while index < lines.len() && is_normal_command(line_str(&lines[index]).as_bytes()) {
                let (hunk, next) = parse_normal_hunk(lines, index)?;
                hunks.push(hunk);
                index = next;
            }
        }
        DiffKind::Ed => {
            while index < lines.len() {
                let text = line_str(&lines[index]);
                let trimmed = trim_indent(&text);
                if (is_minus_header(trimmed.as_bytes()) || trimmed.starts_with("diff "))
                    && !ed_script.is_empty()
                {
                    break;
                }
                if is_ed_command(trimmed.as_bytes()) || !ed_script.is_empty() {
                    push_raw(&mut ed_script, &lines[index]);
                    let done = trimmed == ".";
                    index += 1;
                    if done {
                        // Keep reading until the next file header.
                        while index < lines.len() {
                            let next = trim_indent(&line_str(&lines[index]));
                            if is_minus_header(next.as_bytes())
                                || next.starts_with("diff ")
                                || is_star_header(next.as_bytes())
                            {
                                break;
                            }
                            if is_ed_command(next.as_bytes()) {
                                break;
                            }
                            index += 1;
                        }
                        break;
                    }
                    continue;
                }
                index += 1;
            }
        }
        DiffKind::GitBinary => {}
    }

    Ok((
        Some(finish(
            kind,
            old_name,
            new_name,
            index_name,
            old_timestr,
            new_timestr,
            old_nonexistent,
            new_nonexistent,
            old_stamp,
            new_stamp,
            old_mode,
            new_mode,
            git,
            rename,
            copy,
            hunks,
            ed_script,
            leading,
            report_line,
        )),
        index,
    ))
}

#[allow(clippy::too_many_arguments)]
fn finish(
    kind: DiffKind,
    old_name: Option<String>,
    new_name: Option<String>,
    index_name: Option<String>,
    old_timestr: String,
    new_timestr: String,
    old_nonexistent: u8,
    new_nonexistent: u8,
    old_stamp: i64,
    new_stamp: i64,
    old_mode: u32,
    new_mode: u32,
    git: bool,
    rename: bool,
    copy: bool,
    hunks: Vec<Hunk>,
    ed_script: Vec<u8>,
    leading: Vec<u8>,
    report_line: i64,
) -> FilePatch {
    FilePatch {
        kind,
        old_name,
        new_name,
        index_name,
        old_timestr,
        new_timestr,
        old_nonexistent,
        new_nonexistent,
        old_stamp,
        new_stamp,
        old_mode,
        new_mode,
        git,
        rename,
        copy,
        hunks,
        ed_script,
        leading,
        report_line,
    }
}

fn push_raw(buf: &mut Vec<u8>, line: &SrcLine) {
    buf.extend_from_slice(&line.raw);
    if line.nl {
        buf.push(b'\n');
    }
}

fn join_lines(lines: &[SrcLine]) -> Vec<u8> {
    let mut out = Vec::new();
    for line in lines {
        push_raw(&mut out, line);
    }
    out
}

fn trim_indent(text: &str) -> String {
    text.trim_start_matches([' ', '\t', 'X']).to_string()
}

fn parse_unified_hunk(lines: &[SrcLine], start: usize) -> Result<(Hunk, usize), String> {
    let header = line_str(&lines[start]);
    let header = trim_indent(&header);
    let bytes = header.as_bytes();
    if bytes.len() < 4 {
        return Err(malformed(lines, start));
    }
    let mut s = 4usize; // past "@@ -"
    let (old_first, next) = scan_linenum(bytes, s).ok_or_else(|| malformed(lines, start))?;
    s = next;
    let (mut old_first, ptrn_lines) = if s < bytes.len() && bytes[s] == b',' {
        let (count, next) = scan_linenum(bytes, s + 1).ok_or_else(|| malformed(lines, start))?;
        s = next;
        (old_first, count)
    } else {
        (old_first, 1)
    };
    if s < bytes.len() && bytes[s] == b' ' {
        s += 1;
    }
    if s >= bytes.len() || bytes[s] != b'+' {
        return Err(malformed(lines, start));
    }
    s += 1;
    let (new_first, next) = scan_linenum(bytes, s).ok_or_else(|| malformed(lines, start))?;
    s = next;
    let (mut new_first, repl_lines) = if s < bytes.len() && bytes[s] == b',' {
        let (count, next) = scan_linenum(bytes, s + 1).ok_or_else(|| malformed(lines, start))?;
        s = next;
        (new_first, count)
    } else {
        (new_first, 1)
    };
    if s < bytes.len() && bytes[s] == b' ' {
        s += 1;
    }
    if s >= bytes.len() || bytes[s] != b'@' {
        return Err(malformed(lines, start));
    }
    s += 1;
    let c_function = if s < bytes.len() && bytes[s] == b'@' {
        s += 1;
        if s < bytes.len() && bytes[s] == b' ' {
            bytes[s..].to_vec()
        } else {
            Vec::new()
        }
    } else if s < bytes.len() && bytes[s] == b'@' {
        Vec::new()
    } else {
        // A single `@` is enough for GNU after the counts; a second is usual.
        Vec::new()
    };
    if ptrn_lines == 0 {
        old_first += 1;
    }
    if repl_lines == 0 {
        new_first += 1;
    }

    let mut body: Vec<(u8, Vec<u8>, bool)> = Vec::new();
    let mut index = start + 1;
    let mut got_old = 0i64;
    let mut got_new = 0i64;
    let mut context_run = 0i64;
    let mut prefix_context = -1i64;
    while got_old < ptrn_lines || got_new < repl_lines {
        if index >= lines.len() {
            if ptrn_lines + repl_lines - got_old - got_new < 3 {
                break;
            }
            return Err("unexpected end of file in patch".to_string());
        }
        let mut raw = lines[index].raw.clone();
        let nl = lines[index].nl;
        index += 1;
        if raw.first() == Some(&b'\\') {
            // "\ No newline..." applies to the previous body line.
            if let Some(last) = body.last_mut() {
                if last.1.last() == Some(&b'\n') {
                    last.1.pop();
                }
            }
            continue;
        }
        let (ch, text) = if raw.is_empty() || raw[0] == b'\t' {
            (b' ', with_nl(raw, nl))
        } else {
            let ch = raw[0];
            raw.remove(0);
            if matches!(ch, b'-' | b'+' | b' ' | b'\\') {
                (ch, with_nl(raw, nl))
            } else {
                return Err(malformed(lines, index.saturating_sub(1)));
            }
        };
        let ch = if ch == b'=' { b' ' } else { ch };
        let is_context = ch == b' ';
        match ch {
            b'-' => {
                if got_old >= ptrn_lines {
                    return Err(malformed(lines, start));
                }
                body.push((b'-', text, true));
                got_old += 1;
            }
            b' ' => {
                if got_old >= ptrn_lines {
                    return Err(malformed(lines, start));
                }
                body.push((b' ', text, true));
                got_old += 1;
                got_new += 1;
            }
            b'+' => {
                if got_new >= repl_lines {
                    return Err(malformed(lines, start));
                }
                body.push((b'+', text, false));
                got_new += 1;
            }
            _ => return Err(malformed(lines, start)),
        }
        if is_context {
            context_run += 1;
        } else if prefix_context < 0 {
            prefix_context = context_run;
            context_run = 0;
        } else {
            context_run = 0;
        }
    }
    if prefix_context < 0 {
        // A hunk of pure context is malformed. An empty pattern (creation)
        // records a prefix of 0 when the only lines are insertions.
        if ptrn_lines == 0 {
            prefix_context = 0;
        } else {
            return Err(malformed(lines, start));
        }
    }
    let suffix_context = context_run;

    let mut hunk_lines = Vec::new();
    hunk_lines.push(HunkLine {
        ch: b'*',
        text: format!(
            "*** {old_first},{end} ****\n",
            end = old_first + ptrn_lines - 1
        )
        .into_bytes(),
    });
    let mut new_lines = Vec::new();
    for (ch, text, on_old) in body {
        if on_old {
            if ch == b' ' {
                new_lines.push(HunkLine {
                    ch,
                    text: text.clone(),
                });
            }
            hunk_lines.push(HunkLine { ch, text });
        } else {
            new_lines.push(HunkLine { ch, text });
        }
    }
    hunk_lines.push(HunkLine {
        ch: b'=',
        text: format!(
            "--- {new_first},{end} ----\n",
            end = new_first + repl_lines - 1
        )
        .into_bytes(),
    });
    hunk_lines.extend(new_lines);
    hunk_lines.push(HunkLine {
        ch: b'^',
        text: Vec::new(),
    });

    Ok((
        Hunk {
            old_first,
            new_first,
            ptrn_lines,
            repl_lines,
            prefix_context,
            suffix_context,
            lines: hunk_lines,
            c_function,
        },
        index,
    ))
}

fn with_nl(mut text: Vec<u8>, nl: bool) -> Vec<u8> {
    if nl {
        text.push(b'\n');
    }
    text
}

fn malformed(lines: &[SrcLine], index: usize) -> String {
    let (num, text) = lines
        .get(index)
        .map(|line| (line.num, line_str(line)))
        .unwrap_or((0, String::new()));
    format!("malformed patch at line {num}: {text}")
}

fn scan_linenum(bytes: &[u8], mut index: usize) -> Option<(i64, usize)> {
    if index >= bytes.len() || !bytes[index].is_ascii_digit() {
        return None;
    }
    let mut value = 0i64;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        value = value
            .saturating_mul(10)
            .saturating_add(i64::from(bytes[index] - b'0'));
        index += 1;
    }
    Some((value, index))
}

fn note_range_nonexistent(
    header: &[u8],
    old_stamp: i64,
    new_stamp: i64,
    old_nonexistent: &mut u8,
    new_nonexistent: &mut u8,
) {
    // header begins at "@@ -"
    let bytes = header;
    let mut s = 4usize;
    if s < bytes.len()
        && bytes[s] == b'0'
        && (s + 1 >= bytes.len() || !bytes[s + 1].is_ascii_digit())
    {
        *old_nonexistent = 1 + u8::from(old_stamp == 0);
    }
    while s < bytes.len() && bytes[s] != b' ' && bytes[s] != b'\n' {
        s += 1;
    }
    while s < bytes.len() && bytes[s] == b' ' {
        s += 1;
    }
    if s + 1 < bytes.len()
        && bytes[s] == b'+'
        && bytes[s + 1] == b'0'
        && (s + 2 >= bytes.len() || !bytes[s + 2].is_ascii_digit())
    {
        *new_nonexistent = 1 + u8::from(new_stamp == 0);
    }
}

fn parse_normal_hunk(lines: &[SrcLine], start: usize) -> Result<(Hunk, usize), String> {
    let header = trim_indent(&line_str(&lines[start]));
    let bytes = header.as_bytes();
    let (mut old_first, mut s) = scan_linenum(bytes, 0).ok_or_else(|| malformed(lines, start))?;
    let ptrn_lines = if s < bytes.len() && bytes[s] == b',' {
        let (last, next) = scan_linenum(bytes, s + 1).ok_or_else(|| malformed(lines, start))?;
        s = next;
        last - old_first + 1
    } else if s < bytes.len() && bytes[s] == b'a' {
        0
    } else {
        1
    };
    let kind = *bytes.get(s).ok_or_else(|| malformed(lines, start))?;
    if kind == b'a' {
        old_first += 1;
    }
    s += 1;
    let (mut new_first, next) = scan_linenum(bytes, s).ok_or_else(|| malformed(lines, start))?;
    s = next;
    let new_last = if s < bytes.len() && bytes[s] == b',' {
        let (last, _) = scan_linenum(bytes, s + 1).ok_or_else(|| malformed(lines, start))?;
        last
    } else {
        new_first
    };
    if kind == b'd' {
        new_first += 1;
    }
    let repl_lines = new_last - new_first + 1;
    let mut index = start + 1;
    let mut old_lines = Vec::new();
    for n in 0..ptrn_lines {
        if index >= lines.len() {
            return Err(format!(
                "unexpected end of file in patch at line {}",
                lines.last().map(|line| line.num).unwrap_or(0)
            ));
        }
        let raw = &lines[index].raw;
        if raw.len() < 2 || raw[0] != b'<' || !raw[1].is_ascii_whitespace() {
            return Err(format!(
                "'<' followed by space or tab expected at line {} of patch",
                lines[index].num
            ));
        }
        let mut text = raw[2..].to_vec();
        if lines[index].nl {
            text.push(b'\n');
        }
        if n + 1 == ptrn_lines {
            drop_incomplete(&mut text, lines, index);
        }
        old_lines.push(text);
        index += 1;
    }
    if kind == b'c' {
        if index >= lines.len() || !lines[index].raw.starts_with(b"-") {
            return Err(format!(
                "'---' expected at line {} of patch",
                lines.get(index).map(|line| line.num).unwrap_or(0)
            ));
        }
        index += 1;
    }
    let mut new_lines = Vec::new();
    for n in 0..repl_lines {
        if index >= lines.len() {
            return Err("unexpected end of file in patch".to_string());
        }
        let raw = &lines[index].raw;
        if raw.len() < 2 || raw[0] != b'>' || !raw[1].is_ascii_whitespace() {
            return Err(format!(
                "'>' followed by space or tab expected at line {} of patch",
                lines[index].num
            ));
        }
        let mut text = raw[2..].to_vec();
        if lines[index].nl {
            text.push(b'\n');
        }
        if n + 1 == repl_lines {
            drop_incomplete(&mut text, lines, index);
        }
        new_lines.push(text);
        index += 1;
    }
    let mut hunk_lines = vec![HunkLine {
        ch: b'*',
        text: format!("*** {old_first},{}\n", old_first + ptrn_lines - 1).into_bytes(),
    }];
    for text in old_lines {
        hunk_lines.push(HunkLine { ch: b'-', text });
    }
    hunk_lines.push(HunkLine {
        ch: b'=',
        text: format!("--- {new_first},{new_last}\n").into_bytes(),
    });
    for text in new_lines {
        hunk_lines.push(HunkLine { ch: b'+', text });
    }
    hunk_lines.push(HunkLine {
        ch: b'^',
        text: Vec::new(),
    });
    Ok((
        Hunk {
            old_first,
            new_first,
            ptrn_lines,
            repl_lines,
            prefix_context: 0,
            suffix_context: 0,
            lines: hunk_lines,
            c_function: Vec::new(),
        },
        index,
    ))
}

fn drop_incomplete(text: &mut Vec<u8>, lines: &[SrcLine], index: usize) {
    let Some(next) = lines.get(index + 1) else {
        return;
    };
    if next.raw.first() == Some(&b'\\') && text.last() == Some(&b'\n') {
        text.pop();
    }
}

fn parse_context_hunks(
    lines: &[SrcLine],
    mut index: usize,
) -> Result<(Vec<Hunk>, usize, bool), String> {
    let mut hunks = Vec::new();
    let mut new_context = false;
    while index < lines.len() {
        let text = trim_indent(&line_str(&lines[index]));
        if text.starts_with("***************") {
            index += 1;
            continue;
        }
        if !looks_like_context_range(text.as_bytes()) {
            break;
        }
        let (hunk, next, is_new) = parse_context_hunk(lines, index)?;
        new_context |= is_new;
        hunks.push(hunk);
        index = next;
    }
    Ok((hunks, index, new_context))
}

fn parse_context_hunk(lines: &[SrcLine], start: usize) -> Result<(Hunk, usize, bool), String> {
    let header = trim_indent(&line_str(&lines[start]));
    let bytes = header.as_bytes();
    // "*** start[,end] ****"
    let mut s = 4usize;
    while s < bytes.len() && bytes[s] == b' ' {
        s += 1;
    }
    let (start_line, next) = scan_linenum(bytes, s).ok_or_else(|| malformed(lines, start))?;
    s = next;
    let (old_first, old_last) = if s < bytes.len() && bytes[s] == b',' {
        let (last, _) = scan_linenum(bytes, s + 1).ok_or_else(|| malformed(lines, start))?;
        (start_line, last)
    } else {
        (start_line, start_line)
    };
    let new_style = header.contains("****");
    let ptrn_lines = if old_last < old_first {
        0
    } else {
        old_last - old_first + 1
    };
    let mut index = start + 1;
    let mut old_lines = Vec::new();
    while old_lines.len() < ptrn_lines as usize {
        if index >= lines.len() {
            return Err("unexpected end of file in patch".to_string());
        }
        let text = line_str(&lines[index]);
        if trim_indent(&text).starts_with("--- ") {
            break;
        }
        old_lines.push(context_body_line(&lines[index]));
        index += 1;
    }
    if index >= lines.len() || !trim_indent(&line_str(&lines[index])).starts_with("--- ") {
        return Err(malformed(lines, index.min(lines.len().saturating_sub(1))));
    }
    let new_header = trim_indent(&line_str(&lines[index]));
    let nb = new_header.as_bytes();
    let mut s = 4usize;
    while s < nb.len() && nb[s] == b' ' {
        s += 1;
    }
    let (nstart, next) = scan_linenum(nb, s).ok_or_else(|| malformed(lines, index))?;
    s = next;
    let (new_first, new_last) = if s < nb.len() && nb[s] == b',' {
        let (last, _) = scan_linenum(nb, s + 1).ok_or_else(|| malformed(lines, index))?;
        (nstart, last)
    } else {
        (nstart, nstart)
    };
    let repl_lines = if new_last < new_first {
        0
    } else {
        new_last - new_first + 1
    };
    index += 1;
    let mut new_lines = Vec::new();
    while new_lines.len() < repl_lines as usize && index < lines.len() {
        let text = trim_indent(&line_str(&lines[index]));
        if text.starts_with("*** ")
            || text.starts_with("***************")
            || text.starts_with("--- ")
        {
            break;
        }
        new_lines.push(context_body_line(&lines[index]));
        index += 1;
    }
    let mut hunk_lines = vec![HunkLine {
        ch: b'*',
        text: format!("*** {old_first},{old_last}\n").into_bytes(),
    }];
    let mut prefix = -1i64;
    let mut run = 0i64;
    for (ch, text) in &old_lines {
        if *ch == b' ' {
            run += 1;
        } else if prefix < 0 {
            prefix = run;
            run = 0;
        } else {
            run = 0;
        }
        hunk_lines.push(HunkLine {
            ch: *ch,
            text: text.clone(),
        });
    }
    if prefix < 0 {
        prefix = 0;
    }
    let suffix = run;
    hunk_lines.push(HunkLine {
        ch: b'=',
        text: format!("--- {new_first},{new_last}\n").into_bytes(),
    });
    for (ch, text) in new_lines {
        hunk_lines.push(HunkLine { ch, text });
    }
    hunk_lines.push(HunkLine {
        ch: b'^',
        text: Vec::new(),
    });
    let ptrn_lines = old_lines.len() as i64;
    let repl_lines = (hunk_lines.len() as i64) - ptrn_lines - 3;
    Ok((
        Hunk {
            old_first,
            new_first,
            ptrn_lines,
            repl_lines: repl_lines.max(0),
            prefix_context: prefix,
            suffix_context: suffix,
            lines: hunk_lines,
            c_function: Vec::new(),
        },
        index,
        new_style,
    ))
}

fn context_body_line(line: &SrcLine) -> (u8, Vec<u8>) {
    let raw = &line.raw;
    let (ch, rest) = if raw.is_empty() {
        (b' ', &raw[..])
    } else if matches!(raw[0], b' ' | b'-' | b'+' | b'!') {
        let rest = if raw.len() > 1 && raw[1] == b' ' {
            &raw[2..]
        } else if raw.len() > 1 {
            &raw[1..]
        } else {
            &raw[..0]
        };
        (raw[0], rest)
    } else {
        (b' ', raw.as_slice())
    };
    let mut text = rest.to_vec();
    if line.nl {
        text.push(b'\n');
    }
    (ch, text)
}

fn looks_like_context_range(bytes: &[u8]) -> bool {
    bytes.starts_with(b"*** ") && bytes.len() > 4 && bytes[4].is_ascii_digit()
}

fn is_minus_header(bytes: &[u8]) -> bool {
    bytes.starts_with(b"--- ") && bytes.get(4).is_none_or(|b| !b.is_ascii_digit())
}

fn is_plus_header(bytes: &[u8]) -> bool {
    bytes.starts_with(b"+++ ")
}

fn is_star_header(bytes: &[u8]) -> bool {
    bytes.starts_with(b"*** ") && bytes.get(4).is_some_and(|b| !b.is_ascii_digit())
}

fn is_normal_command(bytes: &[u8]) -> bool {
    let mut s = 0usize;
    if s >= bytes.len() || !bytes[s].is_ascii_digit() {
        return false;
    }
    while s < bytes.len() && (bytes[s].is_ascii_digit() || bytes[s] == b',') {
        s += 1;
    }
    if s >= bytes.len() || !matches!(bytes[s], b'a' | b'c' | b'd') {
        return false;
    }
    s += 1;
    if s >= bytes.len() || !bytes[s].is_ascii_digit() {
        return false;
    }
    while s < bytes.len() && (bytes[s].is_ascii_digit() || bytes[s] == b',') {
        s += 1;
    }
    s == bytes.len()
}

fn is_ed_command(bytes: &[u8]) -> bool {
    let mut s = 0usize;
    if s < bytes.len() && bytes[s].is_ascii_digit() {
        while s < bytes.len() && bytes[s].is_ascii_digit() {
            s += 1;
        }
        if s < bytes.len() && bytes[s] == b',' {
            s += 1;
            if s >= bytes.len() || !bytes[s].is_ascii_digit() {
                return false;
            }
            while s < bytes.len() && bytes[s].is_ascii_digit() {
                s += 1;
            }
        }
    }
    if s >= bytes.len() || !matches!(bytes[s], b'a' | b'c' | b'd' | b'i') {
        return false;
    }
    s += 1;
    s == bytes.len()
}

fn looks_normal_body(lines: &[SrcLine], command: usize) -> bool {
    lines
        .iter()
        .skip(command + 1)
        .take(6)
        .any(|line| line.raw.starts_with(b"< ") || line.raw.starts_with(b"> "))
}

fn looks_ed_body(lines: &[SrcLine], command: usize) -> bool {
    lines
        .iter()
        .skip(command + 1)
        .take(40)
        .any(|line| line.raw == b".")
}

fn git_pair(bytes: &[u8]) -> Option<(String, String)> {
    let rest = bytes.strip_prefix(b"diff --git ")?;
    let (old, next) = take_name_token(rest)?;
    let rest = trim_ascii_start(next);
    let (new, _) = take_name_token(rest)?;
    Some((old, new))
}

fn take_name_token(bytes: &[u8]) -> Option<(String, &[u8])> {
    let bytes = trim_ascii_start(bytes);
    if bytes.first() == Some(&b'"') {
        let (name, next) = parse_c_string(bytes)?;
        return Some((name, next));
    }
    let mut end = 0usize;
    while end < bytes.len() && !bytes[end].is_ascii_whitespace() {
        end += 1;
    }
    if end == 0 {
        return None;
    }
    Some((
        String::from_utf8_lossy(&bytes[..end]).into_owned(),
        &bytes[end..],
    ))
}

fn trim_ascii_start(bytes: &[u8]) -> &[u8] {
    let mut i = 0usize;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    &bytes[i..]
}

fn parse_index(bytes: &[u8]) -> Option<(String, String, Option<u32>)> {
    let rest = bytes.strip_prefix(b"index ")?;
    let (old, rest) = take_hex(rest)?;
    let rest = rest.strip_prefix(b"..")?;
    let (new, rest) = take_hex(rest)?;
    let rest = trim_ascii_start(rest);
    let mode = if rest.is_empty() {
        None
    } else {
        Some(fetch_mode(rest))
    };
    Some((old, new, mode))
}

fn take_hex(bytes: &[u8]) -> Option<(String, &[u8])> {
    let mut end = 0usize;
    while end < bytes.len() && bytes[end].is_ascii_hexdigit() {
        end += 1;
    }
    if end == 0 {
        return None;
    }
    Some((
        String::from_utf8_lossy(&bytes[..end]).into_owned(),
        &bytes[end..],
    ))
}

fn sha_nonexistent(sha: &str) -> u8 {
    if !sha.is_empty() && sha.bytes().all(|b| b == b'0') {
        return 2;
    }
    const EMPTY: &str = "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391";
    if EMPTY.starts_with(sha) { 1 } else { 0 }
}

fn fetch_mode(bytes: &[u8]) -> u32 {
    let mut mode = 0u32;
    let mut n = 0usize;
    for &byte in bytes {
        if n == 6 || !byte.is_ascii_digit() {
            break;
        }
        if byte > b'7' {
            return 0;
        }
        mode = (mode << 3) + u32::from(byte - b'0');
        n += 1;
    }
    if n == 6 { mode } else { 0 }
}

/// Fetch a file name the way GNU `fetchname` does.
/// `/dev/null` yields no name and an epoch timestamp.
fn fetch_name(bytes: &[u8]) -> (Option<String>, String) {
    let (name, timestr, _stamp) = fetch_name_stamp(bytes);
    (name, timestr)
}

fn fetch_name_stamp(bytes: &[u8]) -> (Option<String>, String, i64) {
    let bytes = trim_ascii_start(bytes);
    if bytes.first() == Some(&b'"') {
        if let Some((name, rest)) = parse_c_string(bytes) {
            if name == "/dev/null" {
                return (None, String::new(), 0);
            }
            let timestr = String::from_utf8_lossy(trim_newline(rest)).into_owned();
            return (Some(name), timestr, -1);
        }
        return (None, String::new(), -1);
    }
    let mut end = 0usize;
    while end < bytes.len() && !bytes[end].is_ascii_whitespace() {
        end += 1;
    }
    // A tab starts the timestamp. Internal spaces are kept when a tab follows.
    if end < bytes.len() && bytes[end] != b'\t' {
        if let Some(tab) = bytes.iter().position(|&b| b == b'\t') {
            end = tab;
        }
    }
    let name = String::from_utf8_lossy(&bytes[..end]).into_owned();
    let timestr = String::from_utf8_lossy(trim_newline(&bytes[end..])).into_owned();
    if name == "/dev/null" {
        return (None, String::new(), 0);
    }
    if name.is_empty() {
        return (None, timestr, -1);
    }
    (Some(name), timestr, -1)
}

fn trim_newline(bytes: &[u8]) -> &[u8] {
    let mut end = bytes.len();
    if end > 0 && bytes[end - 1] == b'\n' {
        end -= 1;
    }
    if end > 0 && bytes[end - 1] == b'\r' {
        end -= 1;
    }
    &bytes[..end]
}

fn parse_c_string(bytes: &[u8]) -> Option<(String, &[u8])> {
    if bytes.first() != Some(&b'"') {
        return None;
    }
    let mut out = Vec::new();
    let mut i = 1usize;
    while i < bytes.len() {
        let c = bytes[i];
        i += 1;
        if c == b'"' {
            return Some((String::from_utf8_lossy(&out).into_owned(), &bytes[i..]));
        }
        if c != b'\\' {
            out.push(c);
            continue;
        }
        if i >= bytes.len() {
            return None;
        }
        let esc = bytes[i];
        i += 1;
        let value = match esc {
            b'a' => b'\x07',
            b'b' => b'\x08',
            b'f' => b'\x0c',
            b'n' => b'\n',
            b'r' => b'\r',
            b't' => b'\t',
            b'v' => b'\x0b',
            b'\\' | b'"' => esc,
            b'0'..=b'3' => {
                if i + 1 >= bytes.len() {
                    return None;
                }
                let b1 = bytes[i];
                let b2 = bytes[i + 1];
                if !(b'0'..=b'7').contains(&b1) || !(b'0'..=b'7').contains(&b2) {
                    return None;
                }
                let acc = ((esc - b'0') << 6) | ((b1 - b'0') << 3) | (b2 - b'0');
                if acc == 0 {
                    return None;
                }
                i += 2;
                acc
            }
            _ => return None,
        };
        out.push(value);
    }
    None
}

/// Strip leading path components the way GNU `strip_leading_slashes` does.
pub fn strip_name(name: &str, strip: i64) -> Option<String> {
    let bytes = name.as_bytes();
    let mut n = 0usize;
    let mut s = strip;
    let mut p = 0usize;
    while p < bytes.len() {
        if bytes[p] == b'/' {
            while p + 1 < bytes.len() && bytes[p + 1] == b'/' {
                p += 1;
            }
            if strip < 0 || {
                s -= 1;
                s >= 0
            } {
                n = p + 1;
            }
        }
        p += 1;
    }
    if (strip < 0 || s <= 0) && n < bytes.len() {
        Some(String::from_utf8_lossy(&bytes[n..]).into_owned())
    } else {
        None
    }
}

pub fn name_is_safe(name: &str) -> bool {
    if name.starts_with('/') {
        return false;
    }
    for component in name.split('/') {
        if component == ".." {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_matches_gnu_components() {
        assert_eq!(strip_name("a/b/c", 1).as_deref(), Some("b/c"));
        assert_eq!(strip_name("a/b/c", -1).as_deref(), Some("c"));
        assert_eq!(strip_name("file", -1).as_deref(), Some("file"));
        assert_eq!(strip_name("file", 1), None);
        assert_eq!(strip_name("a/file", 0).as_deref(), Some("a/file"));
    }

    #[test]
    fn parses_unified_creation_and_edit() {
        let parsed = parse_patch(b"--- a/foo\n+++ b/foo\n@@ -1 +1 @@\n-old\n+new\n").unwrap();
        assert_eq!(parsed.files.len(), 1);
        let file = &parsed.files[0];
        assert_eq!(file.old_name.as_deref(), Some("a/foo"));
        assert_eq!(file.new_name.as_deref(), Some("b/foo"));
        assert_eq!(file.hunks.len(), 1);
        assert_eq!(file.hunks[0].ptrn_lines, 1);
        assert_eq!(file.hunks[0].repl_lines, 1);
        assert_eq!(file.hunks[0].text(1), b"old\n");
    }

    #[test]
    fn c_string_filename_unquotes_tab() {
        let (name, _) = fetch_name(br#""\t \040""#);
        assert_eq!(name.as_deref(), Some("\t  "));
    }

    #[test]
    fn dev_null_is_not_a_name() {
        let (name, _, stamp) = fetch_name_stamp(b"/dev/null\n");
        assert!(name.is_none());
        assert_eq!(stamp, 0);
    }

    #[test]
    fn parses_context_normal_and_ed() {
        let context =
            parse_patch(b"*** foo\n--- foo\n***************\n*** 1 ***\n! old\n--- 1 ---\n! new\n")
                .unwrap();
        assert_eq!(context.files[0].kind, DiffKind::Context);
        assert_eq!(context.files[0].hunks[0].text(1), b"old\n");
        assert_eq!(context.files[0].hunks[0].ch(1), b'!');

        let normal = parse_patch(b"1c1\n< old\n---\n> new\n").unwrap();
        assert_eq!(normal.files[0].kind, DiffKind::Normal);
        assert_eq!(normal.files[0].hunks[0].text(1), b"old\n");
        assert_eq!(normal.files[0].hunks[0].ch(1), b'-');

        let ed = parse_patch(b"1c\nnew\n.\n").unwrap();
        assert_eq!(ed.files[0].kind, DiffKind::Ed);
        assert_eq!(ed.files[0].ed_script, b"1c\nnew\n.\n");
    }

    #[test]
    fn empty_input_is_not_garbage() {
        let parsed = parse_patch(b"").unwrap();
        assert!(parsed.files.is_empty());
        assert!(!parsed.only_garbage);
    }

    #[test]
    fn prose_is_only_garbage() {
        let parsed = parse_patch(b"hello\n").unwrap();
        assert!(parsed.only_garbage);
    }
}
