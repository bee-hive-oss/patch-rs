//! Merge hunks with conflict markers.
//!
//! This follows GNU patch 2.8 `merge.c`, including the Myers searches that
//! `merge.c` pulls in from `bestmatch.h` and `diffseq.h`.

use crate::apply::ApplyError;
use crate::diff::Hunk;

const SNAKE_LIMIT: i64 = 20;

/// `true` when the hunk had a conflict.
#[allow(clippy::too_many_arguments)]
pub fn merge_hunk(
    hunk: &Hunk,
    lines: &[Vec<u8>],
    where_line: i64,
    hunk_no: i64,
    diff3: bool,
    verbose: bool,
    silent: bool,
    ignore_ws: bool,
    in_offset: i64,
    output: &mut Vec<u8>,
    frozen: &mut i64,
    out_offset: &mut i64,
    say: &mut dyn FnMut(&str),
) -> Result<bool, ApplyError> {
    let mut hunk = hunk.clone();
    normalize_unified(&mut hunk);

    let mut out = Out {
        buf: std::mem::take(output),
        after_newline: true,
        frozen: *frozen,
    };
    out.after_newline = out.buf.is_empty() || out.buf.last() == Some(&b'\n');

    let result = merge_into(
        &hunk,
        lines,
        where_line,
        hunk_no,
        diff3,
        verbose,
        silent,
        ignore_ws,
        in_offset,
        &mut out,
        out_offset,
        say,
    );
    *output = out.buf;
    *frozen = out.frozen;
    result
}

struct Out {
    buf: Vec<u8>,
    after_newline: bool,
    frozen: i64,
}

#[allow(clippy::too_many_arguments)]
fn merge_into(
    hunk: &Hunk,
    lines: &[Vec<u8>],
    mut where_line: i64,
    hunk_no: i64,
    diff3: bool,
    verbose: bool,
    silent: bool,
    ignore_ws: bool,
    in_offset: i64,
    out: &mut Out,
    out_offset: &mut i64,
    say: &mut dyn FnMut(&str),
) -> Result<bool, ApplyError> {
    let mut first_result = true;
    let mut last_what: Option<&'static str> = None;
    let mut old = 1i64;
    let pat_lines = hunk.ptrn_lines;
    let mut new = pat_lines + 1;
    while hunk.ch(new) == b'=' || hunk.ch(new) == b'\n' {
        new += 1;
    }

    let (applies_cleanly, matched) = if where_line != 0 {
        (true, pat_lines)
    } else {
        let (located, matched) = locate_merge(hunk, lines, in_offset, out.frozen, ignore_ws);
        where_line = located;
        (false, matched)
    };

    let in_base = pat_lines + 2;
    let mut oldin = vec![b' '; (in_base + matched + 1) as usize];
    oldin[0] = b'*';
    oldin[(in_base - 1) as usize] = b'=';
    oldin[(in_base + matched) as usize] = b'^';
    compute_changes(
        hunk,
        lines,
        ignore_ws,
        1,
        in_base - 1,
        where_line,
        where_line + matched,
        &mut oldin,
        in_base,
    );

    if out.frozen < where_line - 1 && !copy_till(out, lines, where_line - 1) {
        return Err(ApplyError::Misordered);
    }

    let mut in_idx = in_base;
    let mut conflicted = false;
    loop {
        let firstold = old;
        let mut firstnew = new;
        let mut firstin = in_idx;
        let mut hit = false;

        if hunk.ch(old) == b'-' || hunk.ch(new) == b'+' {
            while hunk.ch(old) == b'-' {
                if mark(&oldin, old) == b'-' || mark(&oldin, in_idx) == b'+' {
                    hit = true;
                    break;
                } else if mark(&oldin, old) == b' ' {
                    in_idx += 1;
                }
                old += 1;
            }
            if !hit && (mark(&oldin, old) == b'-' || mark(&oldin, in_idx) == b'+') {
                hit = true;
            }
            if !hit {
                while hunk.ch(new) == b'+' {
                    new += 1;
                }
                let added = new - firstnew;
                if verbose || (!silent && !applies_cleanly) {
                    merge_result(
                        &mut first_result,
                        &mut last_what,
                        hunk_no,
                        Some("merged"),
                        where_line,
                        where_line + added - 1,
                        *out_offset,
                        say,
                    );
                }
                out.frozen += old - firstold;
                where_line += old - firstold;
                *out_offset += added;
                let mut cursor = firstnew;
                while cursor < new {
                    write_line(out, hunk.text(cursor));
                    cursor += 1;
                }
            }
        } else if hunk.ch(old) == b' ' {
            if mark(&oldin, old) == b'-' {
                while hunk.ch(old) == b' ' {
                    if mark(&oldin, old) != b'-' {
                        break;
                    }
                    if hunk.ch(new) == b'+' {
                        hit = true;
                        break;
                    }
                    old += 1;
                    new += 1;
                }
                if !hit && (hunk.ch(old) == b'-' || hunk.ch(new) == b'+') {
                    hit = true;
                }
            } else if mark(&oldin, in_idx) == b'+' {
                while mark(&oldin, in_idx) == b'+' {
                    in_idx += 1;
                }
                where_line += in_idx - firstin;
                if !copy_till(out, lines, where_line - 1) {
                    return Err(ApplyError::Misordered);
                }
            } else if mark(&oldin, old) == b' ' {
                while hunk.ch(old) == b' '
                    && mark(&oldin, old) == b' '
                    && hunk.ch(new) == b' '
                    && mark(&oldin, in_idx) == b' '
                {
                    old += 1;
                    new += 1;
                    in_idx += 1;
                }
                where_line += in_idx - firstin;
                if !copy_till(out, lines, where_line - 1) {
                    return Err(ApplyError::Misordered);
                }
            }
        } else {
            break;
        }

        if !hit {
            continue;
        }

        loop {
            if hunk.ch(old) == b'-' {
                while mark(&oldin, in_idx) == b'+' {
                    in_idx += 1;
                }
                if mark(&oldin, old) == b' ' {
                    in_idx += 1;
                }
                old += 1;
            } else if mark(&oldin, old) == b'-' {
                while hunk.ch(new) == b'+' {
                    new += 1;
                }
                if hunk.ch(old) == b' ' {
                    new += 1;
                }
                old += 1;
            } else if hunk.ch(new) == b'+' {
                while hunk.ch(new) == b'+' {
                    new += 1;
                }
            } else if mark(&oldin, in_idx) == b'+' {
                while mark(&oldin, in_idx) == b'+' {
                    in_idx += 1;
                }
            } else {
                break;
            }
        }

        let mut lastwhere = where_line;
        while firstin < in_idx
            && firstnew < new
            && context_matches(hunk, lines, firstnew, lastwhere, ignore_ws)
        {
            firstin += 1;
            firstnew += 1;
            lastwhere += 1;
        }
        let already = firstin == in_idx && firstnew == new;
        if already {
            merge_result(
                &mut first_result,
                &mut last_what,
                hunk_no,
                Some("already applied"),
                where_line,
                lastwhere - 1,
                *out_offset,
                say,
            );
        }
        if diff3 {
            let common_prefix = lastwhere - where_line;
            firstin -= common_prefix;
            firstnew -= common_prefix;
            lastwhere -= common_prefix;
        }
        if where_line != lastwhere {
            where_line = lastwhere;
            if !copy_till(out, lines, where_line - 1) {
                return Err(ApplyError::Misordered);
            }
        }
        if already {
            continue;
        }

        let mut common_suffix = 0i64;
        if !diff3 {
            lastwhere = where_line + (in_idx - firstin);
            while firstin < in_idx
                && firstnew < new
                && context_matches(hunk, lines, new - 1, lastwhere - 1, ignore_ws)
            {
                in_idx -= 1;
                new -= 1;
                lastwhere -= 1;
                common_suffix += 1;
            }
        }

        let mut span = 3 + (in_idx - firstin) + (new - firstnew);
        if diff3 {
            span += 1 + (old - firstold);
        }
        merge_result(
            &mut first_result,
            &mut last_what,
            hunk_no,
            Some("NOT MERGED"),
            where_line,
            where_line + span - 1,
            *out_offset,
            say,
        );
        *out_offset += span - (in_idx - firstin);

        put_marker(out, "<<<<<<<");
        if firstin < in_idx {
            where_line += in_idx - firstin;
            if !copy_till(out, lines, where_line - 1) {
                return Err(ApplyError::Misordered);
            }
        }
        if diff3 {
            put_marker(out, "|||||||");
            let mut cursor = firstold;
            while cursor < old {
                write_line(out, hunk.text(cursor));
                cursor += 1;
            }
        }
        put_marker(out, "=======");
        let mut cursor = firstnew;
        while cursor < new {
            write_line(out, hunk.text(cursor));
            cursor += 1;
        }
        put_marker(out, ">>>>>>>");
        if common_suffix != 0 {
            where_line += common_suffix;
            if !copy_till(out, lines, where_line - 1) {
                return Err(ApplyError::Misordered);
            }
            in_idx += common_suffix;
            new += common_suffix;
        }
        conflicted = true;
    }
    merge_result(
        &mut first_result,
        &mut last_what,
        0,
        None,
        0,
        0,
        *out_offset,
        say,
    );
    Ok(conflicted)
}

fn mark(oldin: &[u8], index: i64) -> u8 {
    oldin[index as usize]
}

fn put_marker(out: &mut Out, text: &str) {
    if !out.after_newline {
        out.buf.push(b'\n');
    }
    out.buf.extend_from_slice(text.as_bytes());
    out.buf.push(b'\n');
    out.after_newline = true;
}

fn write_line(out: &mut Out, text: &[u8]) {
    if text.is_empty() {
        return;
    }
    out.buf.extend_from_slice(text);
    out.after_newline = text.last() == Some(&b'\n');
}

fn copy_till(out: &mut Out, lines: &[Vec<u8>], last: i64) -> bool {
    if out.frozen > last {
        let mut stdout = std::io::stdout().lock();
        let _ = std::io::Write::write_all(&mut stdout, b"misordered hunks! output would be garbled\n");
        let _ = std::io::Write::flush(&mut stdout);
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

#[allow(clippy::too_many_arguments)]
fn merge_result(
    first: &mut bool,
    last_what: &mut Option<&'static str>,
    hunk_no: i64,
    what: Option<&'static str>,
    from: i64,
    to: i64,
    out_offset: i64,
    say: &mut dyn FnMut(&str),
) {
    match what {
        Some(text) if *first => {
            say(&format!("Hunk #{hunk_no} {text} at "));
            *last_what = Some(text);
        }
        None => {
            if !*first {
                say(".\n");
                *last_what = None;
            }
            return;
        }
        Some(text) if *last_what == Some(text) => {
            say(",");
        }
        Some(text) => {
            say(&format!(", {text} at "));
        }
    }
    let from = from + out_offset;
    let to = to + out_offset;
    if to <= from {
        say(&format!("{from}"));
    } else {
        say(&format!("{from}-{to}"));
    }
    *first = false;
}

fn normalize_unified(hunk: &mut Hunk) {
    let last = hunk.ptrn_lines;
    let end = hunk.end();
    let mut new = last + 1;
    while hunk.ch(new) == b'=' || hunk.ch(new) == b'\n' {
        new += 1;
    }
    let mut old = 1i64;
    while old <= last {
        if hunk.ch(old) == b'!' {
            hunk.lines[old as usize].ch = b'-';
        }
        old += 1;
    }
    while new <= end {
        if hunk.ch(new) == b'!' {
            hunk.lines[new as usize].ch = b'+';
        }
        new += 1;
    }
}

fn locate_merge(
    hunk: &Hunk,
    lines: &[Vec<u8>],
    in_offset: i64,
    last_frozen: i64,
    ignore_ws: bool,
) -> (i64, i64) {
    let input_lines = lines.len() as i64;
    let first_guess = hunk.old_first + in_offset;
    let pat_lines = hunk.ptrn_lines;
    let context_lines = count_context_lines(hunk);
    let max_where = input_lines - pat_lines + context_lines + 1;
    let min_where = last_frozen + 1;
    let mut max_pos_offset = max_where - first_guess;
    let mut max_neg_offset = first_guess - min_where;
    let max_offset = max_pos_offset.max(max_neg_offset);
    let mut where_line = first_guess;
    let mut max_matched = 0i64;

    if context_lines == 0 {
        if where_line < min_where {
            where_line = min_where;
        }
        return (where_line, max_matched);
    }

    let mut max = 2 * context_lines;
    let mut min = pat_lines - context_lines;
    let anchor = hunk.suffix_context - hunk.prefix_context;
    if anchor > 0 && hunk.old_first <= 1 {
        max_pos_offset = 0;
    }
    let match_until_eof = anchor < 0;
    if first_guess <= max_neg_offset {
        max_neg_offset = first_guess - 1;
    }

    let mut offset = 0i64;
    while offset <= max_offset {
        if offset <= max_pos_offset {
            let guess = first_guess + offset;
            let need = if match_until_eof {
                input_lines - guess + 1
            } else {
                min
            };
            let (changes, last) = bestmatch(
                hunk,
                lines,
                ignore_ws,
                1,
                pat_lines + 1,
                guess,
                input_lines + 1,
                need,
                max,
            );
            if changes <= max && max_matched < last - guess {
                max_matched = last - guess;
                where_line = guess;
                if changes == 0 {
                    break;
                }
                min = last - guess;
                max = changes - 1;
            }
        }
        if offset > 0 && offset <= max_neg_offset {
            let guess = first_guess - offset;
            let need = if match_until_eof {
                input_lines - guess + 1
            } else {
                min
            };
            let (changes, last) = bestmatch(
                hunk,
                lines,
                ignore_ws,
                1,
                pat_lines + 1,
                guess,
                input_lines + 1,
                need,
                max,
            );
            if changes <= max && max_matched < last - guess {
                max_matched = last - guess;
                where_line = guess;
                if changes == 0 {
                    break;
                }
                min = last - guess;
                max = changes - 1;
            }
        }
        offset += 1;
    }
    if where_line < min_where {
        where_line = min_where;
    }
    (where_line, max_matched)
}

fn count_context_lines(hunk: &Hunk) -> i64 {
    let mut context = 0i64;
    let mut old = 1i64;
    while old <= hunk.ptrn_lines {
        if hunk.ch(old) == b' ' {
            context += 1;
        }
        old += 1;
    }
    context
}

fn context_matches(hunk: &Hunk, lines: &[Vec<u8>], old: i64, where_line: i64, ignore_ws: bool) -> bool {
    let line = ifetch(lines, where_line);
    if line.is_empty() {
        return false;
    }
    let pat = hunk.text(old);
    if ignore_ws {
        similar(pat, line)
    } else {
        line == pat
    }
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

/// GNU `similar`: blanks are folded on the second string first.
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

#[allow(clippy::too_many_arguments)]
fn bestmatch(
    hunk: &Hunk,
    lines: &[Vec<u8>],
    ignore_ws: bool,
    mut xoff: i64,
    xlim: i64,
    mut yoff: i64,
    ylim: i64,
    mut min: i64,
    max: i64,
) -> (i64, i64) {
    let dmin = xoff - ylim;
    let dmax = xlim - yoff;
    let fmid = xoff - yoff;
    let mut fmin = fmid;
    let mut fmax = fmid;
    let span = (2 * max + 3) as usize;
    let mut v = vec![0i64; span];
    let index = |d: i64| (d - fmid + max + 1) as usize;
    let mut ymax = -1i64;
    let mut c;

    let fmid_plus_2_min = if min != 0 {
        let sum = fmid + 2 * min;
        min += yoff;
        if min > ylim {
            return (max + 1, ymax);
        }
        sum
    } else {
        0
    };

    while xoff < xlim && yoff < ylim && context_matches(hunk, lines, xoff, yoff, ignore_ws) {
        xoff += 1;
        yoff += 1;
    }
    if xoff == xlim && yoff >= min && xoff + yoff >= fmid_plus_2_min {
        return (0, yoff);
    }

    v[index(fmid)] = xoff;
    c = 1;
    while c <= max {
        if fmin > dmin {
            fmin -= 1;
            v[index(fmin - 1)] = -1;
        } else {
            fmin += 1;
        }
        if fmax < dmax {
            fmax += 1;
            v[index(fmax + 1)] = -1;
        } else {
            fmax -= 1;
        }
        let mut d = fmax;
        while d >= fmin {
            let x = if v[index(d - 1)] < v[index(d + 1)] {
                v[index(d + 1)]
            } else {
                v[index(d - 1)] + 1
            };
            let mut x = x;
            let mut y = x - d;
            while x < xlim && y < ylim && context_matches(hunk, lines, x, y, ignore_ws) {
                x += 1;
                y += 1;
            }
            v[index(d)] = x;
            if x == xlim && y >= min && x + y - c >= fmid_plus_2_min {
                if ymax < y {
                    ymax = y;
                }
                if y == ylim {
                    return (c, ymax);
                }
            }
            d -= 2;
        }
        if ymax != -1 {
            return (c, ymax);
        }
        c += 1;
    }
    (c, ymax)
}

struct Part {
    xmid: i64,
    ymid: i64,
    lo_minimal: bool,
    hi_minimal: bool,
}

#[allow(clippy::too_many_arguments)]
fn compute_changes(
    hunk: &Hunk,
    lines: &[Vec<u8>],
    ignore_ws: bool,
    xmin: i64,
    xmax: i64,
    ymin: i64,
    ymax: i64,
    oldin: &mut [u8],
    in_base: i64,
) {
    compareseq(
        hunk,
        lines,
        ignore_ws,
        xmin,
        xmax,
        ymin,
        ymax,
        false,
        oldin,
        in_base,
        ymin,
    );
}

#[allow(clippy::too_many_arguments)]
fn compareseq(
    hunk: &Hunk,
    lines: &[Vec<u8>],
    ignore_ws: bool,
    mut xoff: i64,
    mut xlim: i64,
    mut yoff: i64,
    mut ylim: i64,
    mut find_minimal: bool,
    oldin: &mut [u8],
    in_base: i64,
    yorigin: i64,
) {
    loop {
        while xoff < xlim && yoff < ylim && context_matches(hunk, lines, xoff, yoff, ignore_ws) {
            xoff += 1;
            yoff += 1;
        }
        while xoff < xlim
            && yoff < ylim
            && context_matches(hunk, lines, xlim - 1, ylim - 1, ignore_ws)
        {
            xlim -= 1;
            ylim -= 1;
        }
        if xoff == xlim {
            while yoff < ylim {
                note_insert(oldin, in_base, yorigin, yoff);
                yoff += 1;
            }
            break;
        }
        if yoff == ylim {
            while xoff < xlim {
                oldin[xoff as usize] = b'-';
                xoff += 1;
            }
            break;
        }
        let part = diag(
            hunk,
            lines,
            ignore_ws,
            xoff,
            xlim,
            yoff,
            ylim,
            find_minimal,
        );
        let (x1, x1lim, y1, y1lim, min1, x2, x2lim, y2, y2lim, min2) = if (xlim + ylim)
            - (part.xmid + part.ymid)
            < (part.xmid + part.ymid) - (xoff + yoff)
        {
            (
                part.xmid,
                xlim,
                part.ymid,
                ylim,
                part.hi_minimal,
                xoff,
                part.xmid,
                yoff,
                part.ymid,
                part.lo_minimal,
            )
        } else {
            (
                xoff,
                part.xmid,
                yoff,
                part.ymid,
                part.lo_minimal,
                part.xmid,
                xlim,
                part.ymid,
                ylim,
                part.hi_minimal,
            )
        };
        compareseq(
            hunk,
            lines,
            ignore_ws,
            x1,
            x1lim,
            y1,
            y1lim,
            min1,
            oldin,
            in_base,
            yorigin,
        );
        xoff = x2;
        xlim = x2lim;
        yoff = y2;
        ylim = y2lim;
        find_minimal = min2;
    }
}

fn note_insert(oldin: &mut [u8], in_base: i64, yorigin: i64, yoff: i64) {
    oldin[(in_base + (yoff - yorigin)) as usize] = b'+';
}

#[allow(clippy::too_many_arguments)]
fn diag(
    hunk: &Hunk,
    lines: &[Vec<u8>],
    ignore_ws: bool,
    xoff: i64,
    xlim: i64,
    yoff: i64,
    ylim: i64,
    find_minimal: bool,
) -> Part {
    let dmin = xoff - ylim;
    let dmax = xlim - yoff;
    let fmid = xoff - yoff;
    let bmid = xlim - ylim;
    let mut fmin = fmid;
    let mut fmax = fmid;
    let mut bmin = bmid;
    let mut bmax = bmid;
    let odd = (fmid - bmid) & 1 != 0;
    let mut fd: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
    let mut bd: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
    fd.insert(fmid, xoff);
    bd.insert(bmid, xlim);
    let too_expensive = 100_000i64;
    let mut c = 1i64;
    loop {
        let mut big_snake = false;
        if fmin > dmin {
            fmin -= 1;
            fd.insert(fmin - 1, -1);
        } else {
            fmin += 1;
        }
        if fmax < dmax {
            fmax += 1;
            fd.insert(fmax + 1, -1);
        } else {
            fmax -= 1;
        }
        let mut d = fmax;
        while d >= fmin {
            let tlo = fd[&(d - 1)];
            let thi = fd[&(d + 1)];
            let x0 = if tlo < thi { thi } else { tlo + 1 };
            let mut x = x0;
            let mut y = x0 - d;
            while x < xlim && y < ylim && context_matches(hunk, lines, x, y, ignore_ws) {
                x += 1;
                y += 1;
            }
            if x - x0 > SNAKE_LIMIT {
                big_snake = true;
            }
            fd.insert(d, x);
            if odd && bmin <= d && d <= bmax && bd[&d] <= x {
                return Part {
                    xmid: x,
                    ymid: y,
                    lo_minimal: true,
                    hi_minimal: true,
                };
            }
            d -= 2;
        }

        if bmin > dmin {
            bmin -= 1;
            bd.insert(bmin - 1, i64::MAX);
        } else {
            bmin += 1;
        }
        if bmax < dmax {
            bmax += 1;
            bd.insert(bmax + 1, i64::MAX);
        } else {
            bmax -= 1;
        }
        d = bmax;
        while d >= bmin {
            let tlo = bd[&(d - 1)];
            let thi = bd[&(d + 1)];
            let x0 = if tlo < thi { tlo } else { thi - 1 };
            let mut x = x0;
            let mut y = x0 - d;
            while xoff < x && yoff < y && context_matches(hunk, lines, x - 1, y - 1, ignore_ws) {
                x -= 1;
                y -= 1;
            }
            if x0 - x > SNAKE_LIMIT {
                big_snake = true;
            }
            bd.insert(d, x);
            if !odd && fmin <= d && d <= fmax && x <= fd[&d] {
                return Part {
                    xmid: x,
                    ymid: y,
                    lo_minimal: true,
                    hi_minimal: true,
                };
            }
            d -= 2;
        }

        if !find_minimal && 200 < c && big_snake {
            let mut best = 0i64;
            let mut found = false;
            let mut part = Part {
                xmid: 0,
                ymid: 0,
                lo_minimal: true,
                hi_minimal: false,
            };
            d = fmax;
            while d >= fmin {
                let dd = d - fmid;
                let x = fd[&d];
                let y = x - d;
                let v = (x - xoff) * 2 - dd;
                let add = if dd < 0 { -dd } else { dd };
                if v > 12 * (c + add)
                    && v > best
                    && xoff + SNAKE_LIMIT <= x
                    && x < xlim
                    && yoff + SNAKE_LIMIT <= y
                    && y < ylim
                {
                    let mut k = 1i64;
                    while context_matches(hunk, lines, x - k, y - k, ignore_ws) {
                        if k == SNAKE_LIMIT {
                            best = v;
                            part.xmid = x;
                            part.ymid = y;
                            found = true;
                            break;
                        }
                        k += 1;
                    }
                }
                d -= 2;
            }
            if found && best > 0 {
                return part;
            }

            best = 0;
            found = false;
            d = bmax;
            while d >= bmin {
                let dd = d - bmid;
                let x = bd[&d];
                let y = x - d;
                let v = (xlim - x) * 2 + dd;
                let add = if dd < 0 { -dd } else { dd };
                if v > 12 * (c + add)
                    && v > best
                    && xoff < x
                    && x <= xlim - SNAKE_LIMIT
                    && yoff < y
                    && y <= ylim - SNAKE_LIMIT
                {
                    let mut k = 0i64;
                    while context_matches(hunk, lines, x + k, y + k, ignore_ws) {
                        if k == SNAKE_LIMIT - 1 {
                            best = v;
                            part = Part {
                                xmid: x,
                                ymid: y,
                                lo_minimal: false,
                                hi_minimal: true,
                            };
                            found = true;
                            break;
                        }
                        k += 1;
                    }
                }
                d -= 2;
            }
            if found && best > 0 {
                return part;
            }
        }

        if c >= too_expensive {
            let mut fxybest = -1i64;
            let mut fxbest = 0i64;
            d = fmax;
            while d >= fmin {
                let mut x = fd[&d].min(xlim);
                let mut y = x - d;
                if ylim < y {
                    x = ylim + d;
                    y = ylim;
                }
                if fxybest < x + y {
                    fxybest = x + y;
                    fxbest = x;
                }
                d -= 2;
            }
            let mut bxybest = i64::MAX;
            let mut bxbest = 0i64;
            d = bmax;
            while d >= bmin {
                let mut x = xoff.max(bd[&d]);
                let mut y = x - d;
                if y < yoff {
                    x = yoff + d;
                    y = yoff;
                }
                if x + y < bxybest {
                    bxybest = x + y;
                    bxbest = x;
                }
                d -= 2;
            }
            if (xlim + ylim) - bxybest < fxybest - (xoff + yoff) {
                return Part {
                    xmid: fxbest,
                    ymid: fxybest - fxbest,
                    lo_minimal: true,
                    hi_minimal: false,
                };
            }
            return Part {
                xmid: bxbest,
                ymid: bxybest - bxbest,
                lo_minimal: false,
                hi_minimal: true,
            };
        }
        c += 1;
    }
}
