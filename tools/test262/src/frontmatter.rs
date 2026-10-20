//! test262 front matter: the YAML between `/*---` and `---*/`.
//!
//! This is a small parser for the YAML subset that test262 uses. It reads the keys the runner
//! needs (`flags`, `negative`, `includes`, `features`, `locale`) and skips the other keys.
//! It never guesses: a form it does not know gives an error, not a wrong value.
//! `ep9-test262 index` plus PyYAML check it against every test file (see README).
//!
//! Known forms:
//!
//! ```text
//! flags: [onlyStrict, async]      flow list on one line (more lines are joined)
//! features:                       block list
//!   - Symbol.iterator
//! negative:                       block map with exactly `phase` and `type`
//!   phase: parse
//!   type: SyntaxError
//! ```
//!
//! Scalars are plain, 'single-quoted' or "double-quoted". A plain scalar that YAML would read
//! as a number, bool or null is an error, because the runner needs strings.

use std::fmt::Write as _;

/// The parsed front matter of one test.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Meta {
    pub flags: Vec<String>,
    pub negative: Option<Negative>,
    pub includes: Vec<String>,
    pub features: Vec<String>,
    pub locale: Vec<String>,
}

/// The `negative` key: the test must throw `error_type` in `phase`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Negative {
    pub phase: String,
    pub error_type: String,
}

impl Meta {
    pub fn has_flag(&self, flag: &str) -> bool {
        self.flags.iter().any(|f| f == flag)
    }

    /// True if the test expects an early error (`negative.phase: parse`).
    pub fn is_parse_negative(&self) -> bool {
        self.negative.as_ref().is_some_and(|n| n.phase == "parse")
    }
}

const OPEN: &str = "/*---";
const CLOSE: &str = "---*/";

/// Parse the front matter of a test262 source file.
pub fn parse(src: &str) -> Result<Meta, String> {
    let start = src
        .find(OPEN)
        .ok_or("no front matter (`/*---` not found)")?
        + OPEN.len();
    let len = src[start..]
        .find(CLOSE)
        .ok_or("front matter has no closing `---*/`")?;
    parse_yaml(&src[start..start + len])
}

struct Line<'a> {
    /// 1-based line number inside the front matter.
    no: usize,
    /// Number of leading spaces.
    indent: usize,
    /// The text after the leading spaces.
    text: &'a str,
}

impl Line<'_> {
    fn is_blank(&self) -> bool {
        self.text.trim().is_empty()
    }

    fn is_comment(&self) -> bool {
        self.text.starts_with('#')
    }
}

/// One top-level `key: value` entry with its continuation lines.
struct Entry<'a> {
    no: usize,
    key: &'a str,
    inline: &'a str,
    more: Vec<&'a Line<'a>>,
}

/// Split on YAML line breaks: CRLF, CR, LF, NEL, LS, PS (as PyYAML does). test262 has a test
/// file with CR-only line ends.
fn yaml_lines(body: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut chars = body.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if matches!(c, '\n' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}') {
            lines.push(&body[start..i]);
            start = i + c.len_utf8();
            if c == '\r' && chars.peek().is_some_and(|&(_, next)| next == '\n') {
                chars.next();
                start += 1;
            }
        }
    }
    lines.push(&body[start..]);
    lines
}

fn parse_yaml(body: &str) -> Result<Meta, String> {
    let lines: Vec<Line<'_>> = yaml_lines(body)
        .into_iter()
        .enumerate()
        .map(|(i, raw)| {
            let text = raw.trim_start_matches(' ');
            Line {
                no: i + 1,
                indent: raw.len() - text.len(),
                text,
            }
        })
        .collect();
    let Some(base) = lines
        .iter()
        .find(|l| !l.is_blank() && !l.is_comment())
        .map(|l| l.indent)
    else {
        return Ok(Meta::default());
    };

    let mut entries: Vec<Entry<'_>> = Vec::new();
    for line in &lines {
        let continues = line.is_blank()
            || line.indent > base
            || (line.indent == base && is_seq_item(line.text));
        if continues {
            if let Some(entry) = entries.last_mut() {
                entry.more.push(line);
            } else if !line.is_blank() {
                return Err(format!("line {}: text before the first key", line.no));
            }
            continue;
        }
        if line.is_comment() {
            continue;
        }
        if line.indent < base {
            return Err(format!(
                "line {}: indented less than the first key",
                line.no
            ));
        }
        let (key, inline) = split_key(line.text)
            .ok_or_else(|| format!("line {}: expected `key: value`", line.no))?;
        entries.push(Entry {
            no: line.no,
            key,
            inline,
            more: Vec::new(),
        });
    }

    let mut meta = Meta::default();
    let mut seen: Vec<&str> = Vec::new();
    for entry in &entries {
        let wanted = matches!(
            entry.key,
            "flags" | "includes" | "features" | "locale" | "negative"
        );
        if wanted && seen.contains(&entry.key) {
            return Err(format!("line {}: duplicate key `{}`", entry.no, entry.key));
        }
        seen.push(entry.key);
        let at = |e: String| format!("line {}: `{}`: {e}", entry.no, entry.key);
        match entry.key {
            "flags" => meta.flags = parse_list(entry).map_err(at)?,
            "includes" => meta.includes = parse_list(entry).map_err(at)?,
            "features" => meta.features = parse_list(entry).map_err(at)?,
            "locale" => meta.locale = parse_list(entry).map_err(at)?,
            "negative" => meta.negative = Some(parse_negative(entry).map_err(at)?),
            _ => check_skipped_value(entry.inline).map_err(at)?,
        }
    }
    Ok(meta)
}

fn is_seq_item(text: &str) -> bool {
    text == "-" || text.starts_with("- ")
}

/// Split `key: value` (or `key:`). Keys are `[A-Za-z0-9_-]+`, as in every test262 file.
fn split_key(text: &str) -> Option<(&str, &str)> {
    let colon = text.find(':')?;
    let key = &text[..colon];
    let rest = &text[colon + 1..];
    let key_ok = !key.is_empty()
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    (key_ok && (rest.is_empty() || rest.starts_with(' '))).then_some((key, rest))
}

/// A skipped key may hold any block value, but a quoted or flow value must end on its own line.
/// Otherwise a later line of it could look like a key.
fn check_skipped_value(inline: &str) -> Result<(), String> {
    let value = strip_comment(inline).trim();
    let closed = match value.as_bytes().first() {
        Some(b'"' | b'\'') => scalar(value).is_ok(),
        Some(b'[') => value.ends_with(']'),
        Some(b'{') => value.ends_with('}'),
        _ => true,
    };
    if closed {
        Ok(())
    } else {
        Err(format!(
            "multi-line quoted or flow value is not supported: {value}"
        ))
    }
}

fn parse_list(entry: &Entry<'_>) -> Result<Vec<String>, String> {
    let inline = strip_comment(entry.inline).trim();
    if inline.starts_with('[') {
        // Flow list; lines after the key line continue it.
        let mut text = inline.to_string();
        for line in &entry.more {
            text.push(' ');
            text.push_str(strip_comment(line.text).trim());
        }
        let inner = text
            .trim()
            .strip_prefix('[')
            .and_then(|t| t.strip_suffix(']'))
            .ok_or_else(|| format!("flow list does not end with `]`: {text}"))?;
        return flow_items(inner);
    }
    if !inline.is_empty() {
        return Err(format!("expected a list, found `{inline}`"));
    }
    // Block list.
    let mut items = Vec::new();
    let mut indent = None;
    for line in entry
        .more
        .iter()
        .filter(|l| !l.is_blank() && !l.is_comment())
    {
        if *indent.get_or_insert(line.indent) != line.indent {
            return Err(format!(
                "line {}: list items have different indentation",
                line.no
            ));
        }
        let rest = if line.text == "-" {
            ""
        } else {
            line.text
                .strip_prefix("- ")
                .ok_or_else(|| format!("line {}: expected `- item`", line.no))?
        };
        items.push(
            scalar(strip_comment(rest).trim()).map_err(|e| format!("line {}: {e}", line.no))?,
        );
    }
    if items.is_empty() {
        return Err("empty value (YAML null), expected a list".into());
    }
    Ok(items)
}

fn flow_items(inner: &str) -> Result<Vec<String>, String> {
    if inner.trim().is_empty() {
        return Ok(Vec::new());
    }
    if inner.contains(['[', ']', '{', '}']) {
        return Err(format!(
            "nested flow collections are not supported: [{inner}]"
        ));
    }
    let parts = split_outside_quotes(inner, ',');
    let last = parts.len() - 1;
    let mut items = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        let part = part.trim();
        if part.is_empty() && i == last {
            break; // YAML allows one trailing comma.
        }
        items.push(scalar(part)?);
    }
    Ok(items)
}

fn parse_negative(entry: &Entry<'_>) -> Result<Negative, String> {
    if !strip_comment(entry.inline).trim().is_empty() {
        return Err("expected a block map with `phase` and `type`".into());
    }
    let (mut phase, mut error_type) = (None, None);
    let mut indent = None;
    for line in entry
        .more
        .iter()
        .filter(|l| !l.is_blank() && !l.is_comment())
    {
        if *indent.get_or_insert(line.indent) != line.indent {
            return Err(format!(
                "line {}: map keys have different indentation",
                line.no
            ));
        }
        let (key, value) = split_key(line.text)
            .ok_or_else(|| format!("line {}: expected `key: value`", line.no))?;
        let slot = match key {
            "phase" => &mut phase,
            "type" => &mut error_type,
            _ => return Err(format!("line {}: unknown key `{key}`", line.no)),
        };
        if slot.is_some() {
            return Err(format!("line {}: duplicate key `{key}`", line.no));
        }
        *slot = Some(
            scalar(strip_comment(value).trim()).map_err(|e| format!("line {}: {e}", line.no))?,
        );
    }
    match (phase, error_type) {
        (Some(phase), Some(error_type)) => Ok(Negative { phase, error_type }),
        _ => Err("needs both `phase` and `type`".into()),
    }
}

/// One scalar: plain, 'single' or "double" quoted. Returns its string value.
fn scalar(s: &str) -> Result<String, String> {
    if let Some(rest) = s.strip_prefix('\'') {
        let inner = rest
            .strip_suffix('\'')
            .ok_or_else(|| format!("unterminated quote: {s}"))?;
        if inner.replace("''", "").contains('\'') {
            return Err(format!("bad single-quoted scalar: {s}"));
        }
        return Ok(inner.replace("''", "'"));
    }
    if let Some(rest) = s.strip_prefix('"') {
        let inner = rest
            .strip_suffix('"')
            .ok_or_else(|| format!("unterminated quote: {s}"))?;
        let mut out = String::new();
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            match c {
                '"' => return Err(format!("bad double-quoted scalar: {s}")),
                '\\' => match chars.next() {
                    Some(c @ ('"' | '\\' | '/')) => out.push(c),
                    other => {
                        return Err(format!(
                            "unsupported escape \\{} in {s}",
                            other.unwrap_or(' ')
                        ));
                    }
                },
                c => out.push(c),
            }
        }
        return Ok(out);
    }
    if s.is_empty() {
        return Err("empty item".into());
    }
    if s.starts_with([
        '[', ']', '{', '}', ',', '#', '&', '*', '!', '|', '>', '%', '@', '`', '-', '?', ':',
    ]) || s.contains(": ")
        || s.contains(" #")
    {
        return Err(format!("unsupported plain scalar: {s}"));
    }
    if yaml_non_string(s) {
        return Err(format!(
            "`{s}` is not a string in YAML (number, bool or null)"
        ));
    }
    Ok(s.to_string())
}

/// True for plain scalars that YAML 1.1 (PyYAML) reads as null, bool or a number.
/// Over-approximates numbers: any item that starts like one is rejected.
fn yaml_non_string(s: &str) -> bool {
    const WORDS: [&str; 9] = ["~", "null", "true", "false", "yes", "no", "on", "off", "y"];
    let lower = s.to_ascii_lowercase();
    if WORDS.contains(&lower.as_str()) || lower == "n" || lower == ".inf" || lower == ".nan" {
        return true;
    }
    let digits = s.trim_start_matches(['+', '-']).trim_start_matches('.');
    digits.starts_with(|c: char| c.is_ascii_digit())
}

/// Cut a `# comment`: a `#` at the start or after a space or tab, outside quotes.
fn strip_comment(s: &str) -> &str {
    let mut quote = None;
    let mut prev = ' ';
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match (quote, c) {
            (None, '#') if prev == ' ' || prev == '\t' => return &s[..i],
            (None, '\'' | '"') => quote = Some(c),
            (Some('"'), '\\') => {
                chars.next();
            }
            (Some(q), c) if c == q => quote = None,
            _ => {}
        }
        prev = c;
    }
    s
}

fn split_outside_quotes(s: &str, sep: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut quote = None;
    let mut start = 0;
    let mut chars = s.char_indices();
    while let Some((i, c)) = chars.next() {
        match (quote, c) {
            (None, '\'' | '"') => quote = Some(c),
            (Some('"'), '\\') => {
                chars.next();
            }
            (Some(q), c) if c == q => quote = None,
            (None, c) if c == sep => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

/// A JSON object for the index: path, flags, negative, includes, features, locale.
pub fn to_json(path: &str, meta: &Meta) -> String {
    use crate::json;
    let mut out = String::from("{\"path\":");
    json::string(&mut out, path);
    for (key, list) in [
        ("flags", &meta.flags),
        ("includes", &meta.includes),
        ("features", &meta.features),
    ] {
        let _ = write!(out, ",\"{key}\":");
        json::string_array(&mut out, list);
    }
    out.push_str(",\"locale\":");
    json::string_array(&mut out, &meta.locale);
    out.push_str(",\"negative\":");
    match &meta.negative {
        Some(n) => {
            out.push_str("{\"phase\":");
            json::string(&mut out, &n.phase);
            out.push_str(",\"type\":");
            json::string(&mut out, &n.error_type);
            out.push('}');
        }
        None => out.push_str("null"),
    }
    out.push('}');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fm(body: &str) -> Result<Meta, String> {
        parse(&format!("// header\n/*---\n{body}\n---*/\ncode();\n"))
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn typical_test() {
        let meta = fm("esid: sec-x\ndescription: >\n  text with flags: [no] inside\nflags: [onlyStrict, async]\nfeatures: [Symbol.iterator, 'class-fields-private']\nincludes: [propertyHelper.js]\nnegative:\n  phase: parse\n  type: SyntaxError\ninfo: |\n  includes: [nope.js]\n  # not a comment, block text\n").unwrap();
        assert_eq!(meta.flags, strings(&["onlyStrict", "async"]));
        assert_eq!(
            meta.features,
            strings(&["Symbol.iterator", "class-fields-private"])
        );
        assert_eq!(meta.includes, strings(&["propertyHelper.js"]));
        assert!(meta.locale.is_empty());
        assert_eq!(
            meta.negative,
            Some(Negative {
                phase: "parse".into(),
                error_type: "SyntaxError".into()
            })
        );
        assert!(meta.is_parse_negative());
        assert!(meta.has_flag("async"));
    }

    #[test]
    fn block_lists_comments_and_crlf() {
        let meta = fm("flags:\n  - module # trailing\n  # whole line\n  - \"raw\"\r\nfeatures:\n- a\n- b\nlocale: [en-US, \"de\" ] # c").unwrap();
        assert_eq!(meta.flags, strings(&["module", "raw"]));
        assert_eq!(meta.features, strings(&["a", "b"]));
        assert_eq!(meta.locale, strings(&["en-US", "de"]));
    }

    #[test]
    fn every_yaml_line_break() {
        assert_eq!(
            yaml_lines("a\r\nb\rc\nd\u{85}e\u{2028}f\u{2029}g"),
            ["a", "b", "c", "d", "e", "f", "g"]
        );
        assert_eq!(yaml_lines("a\r\r\n"), ["a", "", ""]);
        let meta = parse("/*---\rflags: [raw]\rfeatures:\r  - x\r---*/").unwrap();
        assert_eq!(meta.flags, strings(&["raw"]));
        assert_eq!(meta.features, strings(&["x"]));
    }

    #[test]
    fn multi_line_flow_list_and_trailing_comma() {
        let meta = fm("features: [a,\n  b,\n  c,]\nflags: []").unwrap();
        assert_eq!(meta.features, strings(&["a", "b", "c"]));
        assert!(meta.flags.is_empty());
    }

    #[test]
    fn indented_front_matter() {
        let meta = fm(" description: >\n    text\n flags: [noStrict]").unwrap();
        assert_eq!(meta.flags, strings(&["noStrict"]));
    }

    #[test]
    fn errors_instead_of_guesses() {
        for body in [
            "flags: onlyStrict",
            "flags:",
            "flags: [a, [b]]",
            "features: [2017]",
            "features: [true]",
            "flags: [a]\nflags: [b]",
            "negative:\n  phase: parse",
            "negative:\n  phase: parse\n  type: SyntaxError\n  extra: 1",
            "negative: {phase: parse, type: SyntaxError}",
            "description: \"multi\n  line\"",
            "  flags: [a]\nfeatures: [b]",
            "flags: [a]\n  - b",
        ] {
            assert!(fm(body).is_err(), "should fail: {body:?} -> {:?}", fm(body));
        }
        assert!(parse("no front matter").is_err());
        assert!(parse("/*--- flags: [a]").is_err());
    }

    #[test]
    fn empty_front_matter() {
        assert_eq!(fm("").unwrap(), Meta::default());
        assert_eq!(fm("# only a comment").unwrap(), Meta::default());
    }

    #[test]
    fn quoted_scalars() {
        assert_eq!(scalar("'it''s'").unwrap(), "it's");
        assert_eq!(scalar("\"a\\\"b\"").unwrap(), "a\"b");
        assert!(scalar("'open").is_err());
        assert!(scalar("\"bad\\n\"").is_err());
        assert_eq!(strip_comment("a # b"), "a ");
        assert_eq!(strip_comment("'a # b' # c"), "'a # b' ");
        assert_eq!(strip_comment("a#b"), "a#b");
    }

    #[test]
    fn json_shape() {
        let meta = fm("flags: [raw]\nnegative:\n  phase: parse\n  type: SyntaxError").unwrap();
        assert_eq!(
            to_json("test/a\"b.js", &meta),
            r#"{"path":"test/a\"b.js","flags":["raw"],"includes":[],"features":[],"locale":[],"negative":{"phase":"parse","type":"SyntaxError"}}"#
        );
    }
}
