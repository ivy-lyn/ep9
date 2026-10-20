//! Minimal JSON writing: strings and string arrays. Numbers are written with `write!`.

use std::fmt::Write as _;

/// Append `s` as a JSON string literal.
pub fn string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Append a JSON array of strings.
pub fn string_array<S: AsRef<str>>(out: &mut String, items: &[S]) {
    out.push('[');
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        string(out, item.as_ref());
    }
    out.push(']');
}

#[cfg(test)]
mod tests {
    #[test]
    fn escapes() {
        let mut out = String::new();
        super::string(&mut out, "a\"b\\c\nd\u{1}é");
        assert_eq!(out, "\"a\\\"b\\\\c\\nd\\u0001é\"");
        let mut out = String::new();
        super::string_array(&mut out, &["x", "y"]);
        assert_eq!(out, "[\"x\",\"y\"]");
    }
}
