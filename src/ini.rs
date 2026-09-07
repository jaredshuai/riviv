//! The settings file's ini dialect — a line-oriented port of the upstream
//! parser (c-original/src/ini.c) with its quirks preserved:
//! - UTF-8 with an optional BOM, skipped on read and never written.
//! - Only the one named `[section]` exists for the reader; everything
//!   before it, other sections, and `#`/`;` comment lines are ignored.
//! - A section-header line's remainder is DISCARDED — `[riviv]x=1` reads
//!   nothing: upstream's match loop breaks at the `]` and falls into its
//!   skip-to-next-line scan (ini.c:109-153 + 254-277); a mismatching `[`
//!   line is skipped whole and ends any previous section match.
//! - `key=value` keeps its bytes verbatim — no whitespace trimming; the
//!   value runs to end of line.
//! - Duplicate keys: the LAST occurrence wins (upstream sorts by key and
//!   frees the earlier dupe, ini.c:329-365).
//! - Integer values parse with upstream's `utf8_to_int` (utf8.c:60-136):
//!   `0x`-prefixed hex, else an optional `-` then decimal digits, stopping
//!   at the first foreign byte (partial parse). A key that exists with a
//!   garbage value yields 0 — the caller's default only applies to a
//!   MISSING key.

/// The section's `key` -> `value` pairs. A `BTreeMap`'s insert-overwrite
/// keeps the last duplicate, matching the upstream dedupe's outcome.
pub(crate) type Section = std::collections::BTreeMap<String, String>;

/// Parse one section out of the file's text. See the module docs for the
/// dialect; `section` is matched verbatim (upstream compares byte-by-byte
/// and requires the closing `]`).
pub(crate) fn parse(text: &str, section: &str) -> Section {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut pairs = Section::new();
    let mut in_section = false;
    let mut rest = text;
    while let Some(line_end) = line_end(rest) {
        let (line, after) = split_line(rest, line_end);
        if let Some(after_header) = line.strip_prefix('[') {
            // A section-header attempt always resets the match state; a
            // hit leaves the rest of the line discarded (upstream breaks
            // at the `]` and falls into the skip-to-next-line scan).
            in_section = after_header
                .strip_prefix(section)
                .is_some_and(|t| t.starts_with(']'));
        } else if line.starts_with('#') || line.starts_with(';') {
            // Comment line — ignored wherever it appears (ini.c:156-160).
        } else if in_section && let Some((k, v)) = split_key_value(line) {
            pairs.insert(k, v);
        }
        rest = after;
    }
    // The final line has no terminator; parse it too (a header line here
    // is moot — `in_section` can never be read again, and the header's
    // own line is discarded either way).
    if !rest.starts_with('[')
        && !rest.starts_with('#')
        && !rest.starts_with(';')
        && in_section
        && let Some((k, v)) = split_key_value(rest)
    {
        pairs.insert(k, v);
    }
    pairs
}

/// The byte index just past this line's content: the position of `\n`, or
/// the end of the text. `\r\n` is handled by the caller trimming the `\r`.
fn line_end(text: &str) -> Option<usize> {
    text.find('\n')
}

/// Split at the line terminator, dropping one `\r` before it (the parser
/// only ever sees `\n`- or `\r\n`-terminated files, like upstream).
fn split_line(text: &str, nl: usize) -> (&str, &str) {
    let mut end = nl;
    if end > 0 && text.as_bytes()[end - 1] == b'\r' {
        end -= 1;
    }
    let after = &text[nl + 1..];
    (&text[..end], after)
}

/// `key=value` with the split at the FIRST `=` (ini.c:169-248); bytes are
/// kept verbatim. No `=` on the line: not a pair (skipped by upstream).
fn split_key_value(line: &str) -> Option<(String, String)> {
    let eq = line.find('=')?;
    Some((line[..eq].to_string(), line[eq + 1..].to_string()))
}

/// Serialize one section: the header line, then `key=value` pairs in the
/// caller's order (upstream writes its fixed table order, config.c:293+).
/// CRLF line endings and no BOM, exactly like `_config_write_string`.
pub(crate) fn serialize<K: AsRef<str>, V: AsRef<str>>(section: &str, pairs: &[(K, V)]) -> String {
    let mut out = String::with_capacity(64 + pairs.len() * 24);
    out.push('[');
    out.push_str(section);
    out.push_str("]\r\n");
    for (key, value) in pairs {
        out.push_str(key.as_ref());
        out.push('=');
        out.push_str(value.as_ref());
        out.push_str("\r\n");
    }
    out
}

/// Upstream's `utf8_to_int` (utf8.c:60-136): `0x`/`0X` hex (unsigned, no
/// sign), else optional `-` then decimal; parsing stops at the first
/// foreign byte, so `"12ab"` is 12 and garbage is 0. Wrapping arithmetic
/// mirrors the C build (no panic on absurd values).
pub(crate) fn parse_int(value: &str) -> i32 {
    let bytes = value.as_bytes();
    if bytes.first() == Some(&b'0') && matches!(bytes.get(1), Some(&b'x') | Some(&b'X')) {
        let mut acc: i32 = 0;
        for &b in &bytes[2..] {
            let digit = match b {
                b'0'..=b'9' => i32::from(b - b'0'),
                b'A'..=b'F' => i32::from(b - b'A') + 10,
                b'a'..=b'f' => i32::from(b - b'a') + 10,
                _ => break,
            };
            acc = acc.wrapping_mul(16).wrapping_add(digit);
        }
        return acc;
    }
    let (sign, digits) = match bytes.first() {
        Some(&b'-') => (-1, &value[1..]),
        _ => (1, value),
    };
    let mut acc: i32 = 0;
    for b in digits.bytes() {
        if !b.is_ascii_digit() {
            break;
        }
        acc = acc.wrapping_mul(10).wrapping_add(i32::from(b - b'0'));
    }
    acc.wrapping_mul(sign)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get<'a>(pairs: &'a Section, key: &str) -> Option<&'a str> {
        pairs.get(key).map(String::as_str)
    }

    #[test]
    fn reads_only_the_named_section() {
        let text = "before=0\r\n[other]\nx=1\r\n[riviv]\nx=2\r\n[riviv2]\nx=3\r\ny=4\r\n";
        let s = parse(text, "riviv");
        assert_eq!(get(&s, "x"), Some("2"));
        assert_eq!(get(&s, "y"), None, "keys after a new header don't leak in");
        assert_eq!(get(&s, "before"), None);
    }

    #[test]
    fn last_duplicate_wins() {
        let s = parse("[riviv]\nx=1\r\nx=2\r\n", "riviv");
        assert_eq!(get(&s, "x"), Some("2"));
    }

    #[test]
    fn comment_lines_are_ignored() {
        let s = parse("[riviv]\n;x=1\n#x=2\nx=3\n", "riviv");
        assert_eq!(get(&s, "x"), Some("3"));
    }

    #[test]
    fn key_value_bytes_are_verbatim() {
        // No trimming — upstream stores the raw bytes (ini.c:226-231).
        let s = parse("[riviv]\n x = 1 \n", "riviv");
        assert_eq!(get(&s, " x "), Some(" 1 "));
        // Split is at the FIRST '='; '=' can appear in the value.
        let s = parse("[riviv]\nk=a=b\n", "riviv");
        assert_eq!(get(&s, "k"), Some("a=b"));
    }

    #[test]
    fn lines_without_equals_are_skipped() {
        let s = parse("[riviv]\nnotaquery\nk=1\n", "riviv");
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn section_header_rest_of_line_is_discarded() {
        // The upstream match loop breaks at the `]` and skips the rest of
        // the line (ini.c:123-128 + 254-277) — a key sharing the header
        // line is NOT read; a near-miss header is skipped whole.
        let s = parse("[riviv]x=1\ny=2\n", "riviv");
        assert_eq!(get(&s, "x"), None, "header-line remainder discarded");
        assert_eq!(get(&s, "y"), Some("2"), "the section still opens");
        let s = parse("[riv]x=9\n[riviv]\ny=2\n", "riviv");
        assert_eq!(get(&s, "x"), None);
    }

    #[test]
    fn missing_close_bracket_is_not_a_section() {
        let s = parse("[riviv\nx=1\n", "riviv");
        assert!(s.is_empty());
    }

    #[test]
    fn bom_is_skipped() {
        let s = parse("\u{feff}[riviv]\nx=1\n", "riviv");
        assert_eq!(get(&s, "x"), Some("1"));
    }

    #[test]
    fn final_line_without_terminator_is_read() {
        let s = parse("[riviv]\nx=1", "riviv");
        assert_eq!(get(&s, "x"), Some("1"));
    }

    #[test]
    fn lf_and_crlf_both_terminate() {
        let s = parse("[riviv]\r\nx=1\ny=2\r\n", "riviv");
        assert_eq!(get(&s, "x"), Some("1"));
        assert_eq!(get(&s, "y"), Some("2"));
    }

    #[test]
    fn serialize_round_trips_through_parse() {
        let text = serialize("riviv", &[("x", "10"), ("title", "a b")]);
        assert!(text.starts_with("[riviv]\r\n"));
        assert!(text.ends_with("title=a b\r\n"));
        let s = parse(&text, "riviv");
        assert_eq!(get(&s, "x"), Some("10"));
        assert_eq!(get(&s, "title"), Some("a b"));
    }

    #[test]
    fn parse_int_matches_utf8_to_int() {
        assert_eq!(parse_int("42"), 42);
        assert_eq!(parse_int("-42"), -42);
        assert_eq!(parse_int("0"), 0);
        assert_eq!(parse_int("0x10"), 16);
        assert_eq!(parse_int("0X1f"), 31);
        assert_eq!(parse_int("12ab"), 12, "partial parse stops at 'a'");
        assert_eq!(parse_int("garbage"), 0);
        assert_eq!(parse_int(""), 0);
        assert_eq!(
            parse_int("+5"),
            0,
            "upstream handles no '+': it is a foreign byte at digit position"
        );
        assert_eq!(parse_int("-"), 0);
    }
}
