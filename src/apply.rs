//! Locate and apply hunks, including offset search and fuzz.
//!
//! The search follows GNU patch's `locate_hunk` / `patch_match` /
//! `apply_hunk`. Input lines are 1-based and keep their trailing newline
//! when the file had one.

use crate::diff::Hunk;

#[derive(Clone, Debug)]
pub struct Applied {
    pub output: Vec<u8>,
    pub where_line: i64,
    pub fuzz: i64,
    pub offset: i64,
}

/// Lines of the input file. Each line includes its `\n` when present.
pub fn split_file(data: &[u8]) -> Vec<Vec<u8>> {
    if data.is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut start = 0usize;
    for (i, &byte) in data.iter().enumerate() {
        if byte == b'\n' {
            lines.push(data[start..=i].to_vec());
            start = i + 1;
        }
    }
    if start < data.len() {
        lines.push(data[start..].to_vec());
    }
    lines
}

pub fn join_lines(lines: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for line in lines {
        out.extend_from_slice(line);
    }
    out
}

fn ifetch(lines: &[Vec<u8>], number: i64) -> &[u8] {
    if number <= 0 {
        return b"";
    }
    lines
        .get((number as usize) - 1)
        .map(Vec::as_slice)
        .unwrap_or(b"")
}

fn patch_match(
    hunk: &Hunk,
    lines: &[Vec<u8>],
    base: i64,
    offset: i64,
    prefix_fuzz: i64,
    suffix_fuzz: i64,
    ignore_ws: bool,
) -> bool {
    let pat_lines = hunk.ptrn_lines - suffix_fuzz;
    let mut pline = 1 + prefix_fuzz;
    while pline <= pat_lines {
        let file = ifetch(lines, pline - 1 + base + offset);
        let pattern = hunk.text(pline);
        if ignore_ws {
            if !similar(file, pattern) {
                return false;
            }
        } else if file != pattern {
            return false;
        }
        pline += 1;
    }
    true
}

fn similar(a: &[u8], b: &[u8]) -> bool {
    let mut a = if a.last() == Some(&b'\n') {
        &a[..a.len() - 1]
    } else {
        a
    };
    let mut b = if b.last() == Some(&b'\n') {
        &b[..b.len() - 1]
    } else {
        b
    };
    loop {
        if b.is_empty() || b[0] == b' ' || b[0] == b'\t' {
            while !b.is_empty() && (b[0] == b' ' || b[0] == b'\t') {
                b = &b[1..];
            }
            if !a.is_empty() {
                if a[0] != b' ' && a[0] != b'\t' {
                    return false;
                }
                while !a.is_empty() && (a[0] == b' ' || a[0] == b'\t') {
                    a = &a[1..];
                }
            }
            if a.is_empty() || b.is_empty() {
                return a.is_empty() && b.is_empty();
            }
        } else if a.is_empty() || a[0] != b[0] {
            return false;
        } else {
            a = &a[1..];
            b = &b[1..];
        }
    }
}

/// Return the 1-based input line where `hunk` matches, and the fuzz used.
///
/// `in_offset` is updated the way GNU patch updates it: later hunks keep
/// the accumulated shift.
pub fn locate_hunk(
    hunk: &Hunk,
    lines: &[Vec<u8>],
    fuzz: i64,
    in_offset: &mut i64,
    last_frozen: i64,
    ignore_ws: bool,
) -> Option<i64> {
    let input_lines = lines.len() as i64;
    let first_guess = hunk.old_first + *in_offset;
    let pat_lines = hunk.ptrn_lines;
    let prefix_context = hunk.prefix_context;
    let suffix_context = hunk.suffix_context;
    let context = prefix_context.max(suffix_context);
    let mut prefix_fuzz = fuzz + prefix_context - context;
    let suffix_fuzz = fuzz + suffix_context - context;
    let max_where = input_lines - (pat_lines - suffix_fuzz) + 1;
    let min_where = last_frozen + 1;
    let max_pos_offset = max_where - first_guess;
    let mut max_neg_offset = first_guess - min_where;
    let max_offset = max_pos_offset.max(max_neg_offset);

    if pat_lines == 0 {
        return Some(first_guess);
    }
    if first_guess <= max_neg_offset {
        max_neg_offset = first_guess - 1;
    }

    if prefix_fuzz < 0 && hunk.old_first <= 1 {
        if suffix_fuzz < 0 && (pat_lines != input_lines || prefix_context < last_frozen) {
            return None;
        }
        let offset = 1 - first_guess;
        if last_frozen <= prefix_context
            && offset <= max_pos_offset
            && patch_match(
                hunk,
                lines,
                first_guess,
                offset,
                0,
                suffix_fuzz.max(0),
                ignore_ws,
            )
        {
            *in_offset += offset;
            return Some(first_guess + offset);
        }
        return None;
    } else if prefix_fuzz < 0 {
        prefix_fuzz = 0;
    }

    if suffix_fuzz < 0 {
        let offset = first_guess - (input_lines - pat_lines + 1);
        if offset <= max_neg_offset
            && patch_match(hunk, lines, first_guess, -offset, prefix_fuzz, 0, ignore_ws)
        {
            *in_offset -= offset;
            return Some(first_guess - offset);
        }
        return None;
    }

    let min_offset = if max_pos_offset < 0 {
        first_guess - max_where
    } else if max_neg_offset < 0 {
        first_guess - min_where
    } else {
        0
    };
    let mut offset = min_offset;
    while offset <= max_offset {
        if offset <= max_pos_offset
            && patch_match(
                hunk,
                lines,
                first_guess,
                offset,
                prefix_fuzz,
                suffix_fuzz,
                ignore_ws,
            )
        {
            *in_offset += offset;
            return Some(first_guess + offset);
        }
        if offset <= max_neg_offset
            && patch_match(
                hunk,
                lines,
                first_guess,
                -offset,
                prefix_fuzz,
                suffix_fuzz,
                ignore_ws,
            )
        {
            *in_offset -= offset;
            return Some(first_guess - offset);
        }
        offset += 1;
    }
    None
}

struct OutState {
    buf: Vec<u8>,
    after_newline: bool,
    frozen: i64,
}

fn copy_till(out: &mut OutState, lines: &[Vec<u8>], last: i64) -> bool {
    if out.frozen > last {
        return false;
    }
    while out.frozen < last {
        out.frozen += 1;
        let line = ifetch(lines, out.frozen);
        if !line.is_empty() {
            if !out.after_newline {
                out.buf.push(b'\n');
            }
            out.buf.extend_from_slice(line);
            out.after_newline = line.last() == Some(&b'\n');
        }
    }
    true
}

fn write_hunk_line(out: &mut OutState, text: &[u8]) {
    if !text.is_empty() {
        if !out.after_newline {
            out.buf.push(b'\n');
        }
        out.buf.extend_from_slice(text);
        out.after_newline = text.last() == Some(&b'\n');
    }
}

/// Apply `hunk` at 1-based `where`, appending to `output` which already holds
/// the lines copied before this hunk. `frozen` is the last input line copied.
pub fn apply_hunk(
    hunk: &Hunk,
    lines: &[Vec<u8>],
    where_line: i64,
    output: &mut Vec<u8>,
    frozen: &mut i64,
) -> bool {
    let mut out = OutState {
        buf: std::mem::take(output),
        after_newline: true,
        frozen: *frozen,
    };
    // `output` was moved into `out.buf`, so the newline state comes from those bytes.
    out.after_newline = out.buf.is_empty() || out.buf.last() == Some(&b'\n');

    let mut old = 1i64;
    let lastline = hunk.ptrn_lines;
    let mut new = lastline + 1;
    let pat_end = hunk.end();
    let mut where_line = where_line;
    where_line -= 1;
    while hunk.ch(new) == b'=' || hunk.ch(new) == b'\n' {
        new += 1;
    }
    while old <= lastline {
        if hunk.ch(old) == b'-' {
            if !copy_till(&mut out, lines, where_line + old - 1) {
                *output = out.buf;
                *frozen = out.frozen;
                return false;
            }
            out.frozen += 1;
            old += 1;
        } else if new > pat_end {
            break;
        } else if hunk.ch(new) == b'+' {
            if !copy_till(&mut out, lines, where_line + old - 1) {
                *output = out.buf;
                *frozen = out.frozen;
                return false;
            }
            write_hunk_line(&mut out, hunk.text(new));
            new += 1;
        } else if hunk.ch(new) != hunk.ch(old) {
            *output = out.buf;
            *frozen = out.frozen;
            return false;
        } else if hunk.ch(new) == b'!' {
            if !copy_till(&mut out, lines, where_line + old - 1) {
                *output = out.buf;
                *frozen = out.frozen;
                return false;
            }
            while hunk.ch(old) == b'!' {
                out.frozen += 1;
                old += 1;
            }
            while hunk.ch(new) == b'!' {
                write_hunk_line(&mut out, hunk.text(new));
                new += 1;
            }
        } else {
            old += 1;
            new += 1;
        }
    }
    if new <= pat_end && hunk.ch(new) == b'+' {
        if !copy_till(&mut out, lines, where_line + old - 1) {
            *output = out.buf;
            *frozen = out.frozen;
            return false;
        }
        while new <= pat_end && hunk.ch(new) == b'+' {
            write_hunk_line(&mut out, hunk.text(new));
            new += 1;
        }
    }
    *output = out.buf;
    *frozen = out.frozen;
    true
}

/// Copy any input lines not yet consumed.
pub fn spew_rest(lines: &[Vec<u8>], output: &mut Vec<u8>, frozen: &mut i64) -> bool {
    apply_tail(lines, output, frozen, lines.len() as i64)
}

fn apply_tail(lines: &[Vec<u8>], output: &mut Vec<u8>, frozen: &mut i64, last: i64) -> bool {
    let mut out = OutState {
        buf: std::mem::take(output),
        after_newline: true,
        frozen: *frozen,
    };
    out.after_newline = out.buf.is_empty() || out.buf.last() == Some(&b'\n');
    let ok = copy_till(&mut out, lines, last);
    *output = out.buf;
    *frozen = out.frozen;
    ok
}

pub fn different_line_endings(hunk: &Hunk, lines: &[Vec<u8>], where_line: i64) -> bool {
    let patch = hunk.text(1);
    if patch.len() < 2 {
        return false;
    }
    let patch_crlf =
        patch.len() >= 2 && patch[patch.len() - 2] == b'\r' && patch[patch.len() - 1] == b'\n';
    if lines.is_empty() {
        return false;
    }
    let mut where_line = where_line;
    if where_line > lines.len() as i64 {
        where_line = lines.len() as i64;
    }
    if where_line < 1 {
        where_line = 1;
    }
    let line = ifetch(lines, where_line);
    if line.is_empty() {
        return false;
    }
    let input_crlf =
        line.len() >= 2 && line[line.len() - 2] == b'\r' && line[line.len() - 1] == b'\n';
    patch_crlf != input_crlf
}

pub fn unified_reject_hunk(
    hunk: &Hunk,
    out_offset: i64,
    old_name: &str,
    new_name: &str,
    old_time: &str,
    new_time: &str,
    header: bool,
) -> Vec<u8> {
    let mut out = Vec::new();
    if header {
        push_header(&mut out, b"--- ", old_name, old_time);
        push_header(&mut out, b"+++ ", new_name, new_time);
    }
    out.extend_from_slice(b"@@ -");
    push_range(&mut out, hunk.old_first + out_offset, hunk.ptrn_lines);
    out.extend_from_slice(b" +");
    push_range(&mut out, hunk.new_first + out_offset, hunk.repl_lines);
    out.extend_from_slice(b" @@");
    out.extend_from_slice(&hunk.c_function);
    out.push(b'\n');
    let mut old = 1i64;
    let lastline = hunk.ptrn_lines;
    let mut new = lastline + 1;
    while hunk.ch(new) == b'=' || hunk.ch(new) == b'\n' {
        new += 1;
    }
    loop {
        while hunk.ch(old) == b'-' {
            out.push(b'-');
            out.extend_from_slice(hunk.text(old));
            old += 1;
        }
        while hunk.ch(new) == b'+' {
            out.push(b'+');
            out.extend_from_slice(hunk.text(new));
            new += 1;
        }
        if old > lastline {
            break;
        }
        out.push(b' ');
        out.extend_from_slice(hunk.text(old));
        old += 1;
        new += 1;
    }
    out
}

fn push_header(out: &mut Vec<u8>, tag: &[u8], name: &str, timestr: &str) {
    out.extend_from_slice(tag);
    if name.is_empty() {
        out.extend_from_slice(b"/dev/null");
    } else {
        out.extend_from_slice(name.as_bytes());
    }
    out.extend_from_slice(timestr.as_bytes());
    out.push(b'\n');
}

fn push_range(out: &mut Vec<u8>, start: i64, count: i64) {
    match count {
        0 => {
            out.extend_from_slice((start - 1).to_string().as_bytes());
            out.extend_from_slice(b",0");
        }
        1 => out.extend_from_slice(start.to_string().as_bytes()),
        _ => {
            out.extend_from_slice(start.to_string().as_bytes());
            out.push(b',');
            out.extend_from_slice(count.to_string().as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::parse_patch;

    #[test]
    fn exact_replace() {
        let parsed = parse_patch(b"--- foo\n+++ foo\n@@ -1 +1 @@\n-old\n+new\n").unwrap();
        let hunk = &parsed.files[0].hunks[0];
        let lines = split_file(b"old\n");
        let mut offset = 0i64;
        let found = locate_hunk(hunk, &lines, 0, &mut offset, 0, false);
        assert_eq!(found, Some(1));
        assert_eq!(offset, 0);
        let mut output = Vec::new();
        let mut frozen = 0i64;
        assert!(apply_hunk(hunk, &lines, 1, &mut output, &mut frozen));
        spew_rest(&lines, &mut output, &mut frozen);
        assert_eq!(output, b"new\n");
    }

    #[test]
    fn offset_and_fuzz_like_asymmetric_hunk() {
        let parsed = parse_patch(b"--- a\n+++ a\n@@ -1,3 +1,4 @@\n 2\n+2a\n 3\n 4\n").unwrap();
        let hunk = &parsed.files[0].hunks[0];
        let lines = split_file(b"1\n2\n3\n4\n5\n");
        let mut offset = 0i64;
        assert!(locate_hunk(hunk, &lines, 0, &mut offset, 0, false).is_none());
        offset = 0;
        let found = locate_hunk(hunk, &lines, 1, &mut offset, 0, false);
        assert_eq!(found, Some(2));
        assert_eq!(offset, 1);
    }
}
