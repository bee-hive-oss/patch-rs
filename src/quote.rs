//! Filename quoting compatible with GNU patch's gnulib `quotearg`.
//!
//! The default style is shell quoting: a string is left alone when every
//! character is shell-safe, and wrapped in single quotes otherwise.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuotingStyle {
    Literal,
    Shell,
    ShellAlways,
    ShellEscape,
    ShellEscapeAlways,
    C,
    CMaybe,
    Escape,
    Locale,
    Clocale,
}

impl QuotingStyle {
    pub fn parse(name: &str) -> Result<Self, ArgMatch> {
        argmatch(
            name,
            &[
                ("literal", Self::Literal),
                ("shell", Self::Shell),
                ("shell-always", Self::ShellAlways),
                ("shell-escape", Self::ShellEscape),
                ("shell-escape-always", Self::ShellEscapeAlways),
                ("c", Self::C),
                ("c-maybe", Self::CMaybe),
                ("escape", Self::Escape),
                ("locale", Self::Locale),
                ("clocale", Self::Clocale),
            ],
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArgMatch {
    Invalid,
    Ambiguous,
}

pub fn argmatch<T: Copy>(name: &str, table: &[(&str, T)]) -> Result<T, ArgMatch> {
    if let Some((_, value)) = table.iter().find(|(key, _)| *key == name) {
        return Ok(*value);
    }
    let mut found: Option<T> = None;
    for (key, value) in table {
        if key.starts_with(name) {
            if let Some(prev) = found {
                if std::mem::discriminant(&prev) != std::mem::discriminant(value) {
                    return Err(ArgMatch::Ambiguous);
                }
            } else {
                found = Some(*value);
            }
        }
    }
    found.ok_or(ArgMatch::Invalid)
}

/// Quote `text` the way GNU patch quotes a file name in a message.
pub fn quote(style: QuotingStyle, text: &str) -> String {
    match style {
        QuotingStyle::Literal => text.to_string(),
        QuotingStyle::Shell | QuotingStyle::ShellEscape => shell_quote(text, false),
        QuotingStyle::ShellAlways | QuotingStyle::ShellEscapeAlways => shell_quote(text, true),
        QuotingStyle::C | QuotingStyle::CMaybe | QuotingStyle::Clocale => c_quote(text),
        QuotingStyle::Escape => escape_quote(text),
        QuotingStyle::Locale => shell_quote(text, true),
    }
}

fn shell_safe(text: &str, index: usize, ch: char) -> bool {
    if !ch.is_ascii() {
        return !ch.is_control();
    }
    match ch {
        'a'..='z'
        | 'A'..='Z'
        | '0'..='9'
        | '%'
        | '+'
        | ','
        | '-'
        | '.'
        | '/'
        | ':'
        | '@'
        | ']'
        | '_' => true,
        '#' | '~' => index != 0,
        '{' | '}' => text.chars().nth(1).is_some(),
        _ => false,
    }
}

fn shell_quote(text: &str, always: bool) -> String {
    let safe = !text.is_empty()
        && text
            .chars()
            .enumerate()
            .all(|(i, ch)| shell_safe(text, i, ch));
    if safe && !always {
        return text.to_string();
    }
    let mut out = String::from("'");
    for ch in text.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

fn c_quote(text: &str) -> String {
    let mut out = String::from("\"");
    for ch in text.chars() {
        match ch {
            '"' | '\\' => {
                out.push('\\');
                out.push(ch);
            }
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{7}' => out.push_str("\\a"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\u{b}' => out.push_str("\\v"),
            c if (c as u32) < 32 || c == '\u{7f}' => {
                out.push_str(&format!("\\{:03o}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn escape_quote(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ' ' => out.push_str("\\ "),
            c if (c as u32) < 32 => out.push_str(&format!("\\{:03o}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_leaves_plain_names() {
        assert_eq!(quote(QuotingStyle::Shell, "file.c"), "file.c");
        assert_eq!(quote(QuotingStyle::Shell, "a/b"), "a/b");
        assert_eq!(quote(QuotingStyle::Shell, "-1"), "-1");
    }

    #[test]
    fn shell_quotes_spaces_and_empty() {
        assert_eq!(quote(QuotingStyle::Shell, "c d"), "'c d'");
        assert_eq!(quote(QuotingStyle::Shell, ""), "''");
        assert_eq!(quote(QuotingStyle::Shell, "it's"), "'it'\\''s'");
    }

    #[test]
    fn argmatch_prefixes() {
        assert_eq!(QuotingStyle::parse("shell"), Ok(QuotingStyle::Shell));
        assert_eq!(QuotingStyle::parse("sh"), Err(ArgMatch::Ambiguous));
        assert_eq!(QuotingStyle::parse("nope"), Err(ArgMatch::Invalid));
        assert_eq!(QuotingStyle::parse(""), Err(ArgMatch::Ambiguous));
    }
}
