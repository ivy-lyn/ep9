//! The `path:line` drift check of `cargo xtask adr-lint` (plan v9 section 5.6 task 1; the ADRLINE
//! dry run of the scratchpad tool `path_line_drift.py`, ported with its rules, plan \[I15\]).
//!
//! Every `path:N` or `path:N-M` reference in `docs/adr/*.md` and `docs/plan/*.md` (and a bare
//! `` `:N` `` after a path on the same line, and the form `` `path` lines N-M ``) must name the
//! same line at HEAD as when it was written:
//!
//! 1. A path that is not a file of the repo (at HEAD or in its history), or a token right after
//!    an evidence tag such as `[TSRS `, is external and is skipped.
//! 2. The intro commit is the oldest commit that added the exact token to the doc (`git log -S`).
//! 3. The line maps from the file at the intro commit to HEAD through the `git diff -U0` hunks; a
//!    line inside a changed hunk maps by its exact text (trimmed, 8 characters or more, exactly
//!    one match at HEAD).
//! 4. A history pin (a token in a dated row that records where a past commit put a line) keeps
//!    its number: the line at that commit must match the pin's pattern. A pin (a reference that
//!    was off when written, or whose line changed) finds the line at HEAD by a pattern that must
//!    match exactly one line, plus an offset.
//! 5. A token with no intro commit is new in the work tree: it must lie inside the file at HEAD.
//!
//! Result per token: ok, FIX (the line moved: the doc needs the new number) or UNFIXABLE (no
//! unique answer: a pin is needed). The check passes with no FIX and no UNFIXABLE line. Docs
//! are read from the work tree; code files from HEAD. A history pin that no token uses is
//! UNFIXABLE (its row changed); an unused pin is only listed.
//!
//! The pins live in [`PINS_FILE`] (tab-separated, one per line; `#` starts a comment):
//!
//! ```text
//! pin      <doc file name> <token> <path at HEAD> <start pattern> <offset> [<end pattern> <offset>]
//! history  <doc file name> <token> <text the doc line holds> <commit> <pattern> [<path>]
//! ```
//!
//! A pattern is the regular-expression subset that the pins use: a literal, `^` and `$`
//! anchors, and `\` before a punctuation character. A pattern with any other regex syntax is an
//! error, never a guess.

use super::read;
use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::process::Command;

/// The pin table, relative to the tree root.
pub(crate) const PINS_FILE: &str = "tools/xtask/src/exit/adrline-pins.tsv";

/// The doc dirs (each `*.md` in them, not below).
const DOC_DIRS: [&str; 2] = ["docs/adr", "docs/plan"];

#[derive(Debug, Clone)]
struct Pin {
    path: String,
    start: (String, i64),
    end: Option<(String, i64)>,
}

#[derive(Debug, Clone)]
struct History {
    doc: String,
    token: String,
    text: String,
    commit: String,
    pattern: String,
    path: Option<String>,
}

/// The pin tables of [`PINS_FILE`] (a missing file has none).
#[derive(Debug, Default)]
struct Pins {
    /// (doc file name, token) -> pin, in file order.
    pins: Vec<((String, String), Pin)>,
    history: Vec<History>,
}

fn load_pins(root: &Path) -> Result<Pins, String> {
    let p = root.join(PINS_FILE);
    if !p.is_file() {
        return Ok(Pins::default());
    }
    let mut out = Pins::default();
    for (i, line) in read(&p)?.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        let bad = || format!("{PINS_FILE}:{}: not a pin or history line: {line:?}", i + 1);
        let off = |s: &str| s.parse::<i64>().map_err(|_| bad());
        match f.as_slice() {
            ["pin", doc, token, path, sp, so, rest @ ..] => {
                let end = match rest {
                    [] => None,
                    [ep, eo] => Some((ep.to_string(), off(eo)?)),
                    _ => return Err(bad()),
                };
                for pat in std::iter::once(*sp).chain(end.as_ref().map(|e| e.0.as_str())) {
                    Pattern::parse(pat).map_err(|e| format!("{}: {e}", bad()))?;
                }
                out.pins.push((
                    (doc.to_string(), token.to_string()),
                    Pin {
                        path: path.to_string(),
                        start: (sp.to_string(), off(so)?),
                        end,
                    },
                ));
            }
            ["history", doc, token, text, commit, pattern, rest @ ..] if rest.len() <= 1 => {
                Pattern::parse(pattern).map_err(|e| format!("{}: {e}", bad()))?;
                out.history.push(History {
                    doc: doc.to_string(),
                    token: token.to_string(),
                    text: text.to_string(),
                    commit: commit.to_string(),
                    pattern: pattern.to_string(),
                    path: rest.first().map(|s| s.to_string()),
                });
            }
            _ => return Err(bad()),
        }
    }
    Ok(out)
}

/// A pin pattern: a literal with optional `^` and `$` anchors (`re.search` on one line).
#[derive(Debug, PartialEq)]
pub(crate) struct Pattern {
    lit: String,
    start: bool,
    end: bool,
}

impl Pattern {
    pub(crate) fn parse(p: &str) -> Result<Pattern, String> {
        let (start, body) = match p.strip_prefix('^') {
            Some(b) => (true, b),
            None => (false, p),
        };
        let mut lit = String::new();
        let mut end = false;
        let mut chars = body.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\\' => match chars.next() {
                    Some(e) if !e.is_ascii_alphanumeric() => lit.push(e),
                    other => {
                        return Err(format!(
                            "pattern {p:?}: `\\{}` is regex syntax this check does not read",
                            other.map_or(String::new(), String::from)
                        ));
                    }
                },
                '$' if chars.peek().is_none() => end = true,
                '.' | '*' | '+' | '?' | '[' | ']' | '{' | '}' | '(' | ')' | '|' | '$' | '^' => {
                    return Err(format!(
                        "pattern {p:?}: `{c}` is regex syntax this check does not read (write `\\{c}` for the character)"
                    ));
                }
                c => lit.push(c),
            }
        }
        Ok(Pattern { lit, start, end })
    }

    pub(crate) fn search(&self, line: &str) -> bool {
        match (self.start, self.end) {
            (true, true) => line == self.lit,
            (true, false) => line.starts_with(&self.lit),
            (false, true) => line.ends_with(&self.lit),
            (false, false) => line.contains(&self.lit),
        }
    }
}

/// One reference token of a doc line.
#[derive(Debug, Clone, PartialEq)]
struct Token {
    start: usize,
    end: usize,
    path: Option<String>,
    n: u64,
    n2: Option<u64>,
    /// The token as written (a bare token with its backticks).
    text: String,
    kind: Kind,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Colon,
    Bare,
    Lines,
}

/// A char of `[\w./-]` (a path run).
fn path_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '.' | '/' | '-')
}

/// `(?:[\w.-]+/)*[\w.-]+\.[A-Za-z0-9]+`, the whole string.
fn path_ok(p: &str) -> bool {
    let comps: Vec<&str> = p.split('/').collect();
    if comps.iter().any(|c| c.is_empty()) {
        return false;
    }
    let last = comps[comps.len() - 1];
    match last.rfind('.') {
        Some(dot) => {
            let ext = &last[dot + 1..];
            dot > 0 && !ext.is_empty() && ext.bytes().all(|b| b.is_ascii_alphanumeric())
        }
        None => false,
    }
}

/// `digits` at `s[i..]`: (value, end), or None (no digit, or too large).
fn digits(s: &str, i: usize) -> Option<(u64, usize)> {
    let len = s[i..].bytes().take_while(u8::is_ascii_digit).count();
    if len == 0 {
        return None;
    }
    Some((s[i..i + len].parse().ok()?, i + len))
}

/// `N` or `N-M` at `s[i..]`: (n, n2, end).
fn range(s: &str, i: usize) -> Option<(u64, Option<u64>, usize)> {
    let (n, e) = digits(s, i)?;
    if s[e..].starts_with('-')
        && let Some((n2, e2)) = digits(s, e + 1)
    {
        return Some((n, Some(n2), e2));
    }
    Some((n, None, e))
}

/// The tokens of one line, by start: the `path:N[-M]` form (not after a path character), the
/// bare `` `:N[-M]` `` form and the `` `path` lines N[-M] `` form.
fn tokens(line: &str) -> Vec<Token> {
    let mut out = Vec::new();
    // path:N[-M]
    let mut pos = 0;
    let mut prev: Option<char> = None;
    let mut i = 0;
    while i < line.len() {
        let c = line[i..].chars().next().unwrap_or(' ');
        let run_start = i >= pos && path_char(c) && !prev.is_some_and(path_char);
        if run_start {
            let e = i + line[i..]
                .char_indices()
                .find(|(_, ch)| !path_char(*ch))
                .map_or(line.len() - i, |(k, _)| k);
            if line[e..].starts_with(':')
                && path_ok(&line[i..e])
                && let Some((n, n2, end)) = range(line, e + 1)
            {
                out.push(Token {
                    start: i,
                    end,
                    path: Some(line[i..e].to_string()),
                    n,
                    n2,
                    text: line[i..end].to_string(),
                    kind: Kind::Colon,
                });
                pos = end;
            }
        }
        prev = Some(c);
        i += c.len_utf8();
    }
    // `:N[-M]`
    let mut from = 0;
    while let Some(k) = line[from..].find("`:") {
        let b = from + k;
        if let Some((n, n2, e)) = range(line, b + 2)
            && line[e..].starts_with('`')
        {
            out.push(Token {
                start: b + 1,
                end: e,
                path: None,
                n,
                n2,
                text: line[b..e + 1].to_string(),
                kind: Kind::Bare,
            });
            from = e + 1;
        } else {
            from = b + 1;
        }
    }
    // `path` lines N[-M]
    let mut from = 0;
    while let Some(k) = line[from..].find('`') {
        let b = from + k;
        let e = b
            + 1
            + line[b + 1..]
                .char_indices()
                .find(|(_, ch)| !path_char(*ch))
                .map_or(line.len() - b - 1, |(k, _)| k);
        if line[e..].starts_with("` lines ")
            && path_ok(&line[b + 1..e])
            && let Some((n, n2, end)) = range(line, e + 8)
        {
            out.push(Token {
                start: b,
                end,
                path: Some(line[b + 1..e].to_string()),
                n,
                n2,
                text: line[b..end].to_string(),
                kind: Kind::Lines,
            });
            from = end;
        } else {
            from = b + 1;
        }
    }
    out.sort_by_key(|t| (t.start, t.end));
    out
}

/// The text before a token ends with an evidence tag `[ABC-1 ` (`\[[A-Z][A-Z0-9-]* $`).
fn after_evidence_tag(prefix: &str) -> bool {
    let Some(p) = prefix.strip_suffix(' ') else {
        return false;
    };
    let Some(b) = p.rfind('[') else {
        return false;
    };
    let w = &p[b + 1..];
    w.bytes().next().is_some_and(|c| c.is_ascii_uppercase())
        && w.bytes()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'-')
}

fn render(kind: Kind, path: &str, n: u64, n2: Option<u64>) -> String {
    let multi = n2.is_some_and(|m| m != n);
    let rng = match n2 {
        Some(m) if m != n => format!("{n}-{m}"),
        _ => n.to_string(),
    };
    match kind {
        Kind::Bare => format!(":{rng}"),
        Kind::Lines => format!("`{path}` line{} {rng}", if multi { "s" } else { "" }),
        Kind::Colon => format!("{path}:{rng}"),
    }
}

/// A `git diff -U0` hunk: (old start, old count, new start, new count).
type Hunk = (u64, u64, u64, u64);

/// The git reads of one check, cached.
struct Repo<'a> {
    root: &'a Path,
    files: HashMap<(String, String), Option<Vec<String>>>,
    intro: HashMap<(String, String), Option<String>>,
    hunks: HashMap<(String, String), Vec<Hunk>>,
    logged: HashMap<String, bool>,
}

impl Repo<'_> {
    fn git(&self, args: &[&str]) -> Option<String> {
        let out = Command::new("git")
            .arg("-C")
            .arg(self.root)
            .args(args)
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// The lines of `path` at `commit` (split on `\n`, so a final newline gives a last empty
    /// line), or None.
    fn file_at(&mut self, commit: &str, path: &str) -> Option<Vec<String>> {
        let key = (commit.to_string(), path.to_string());
        if !self.files.contains_key(&key) {
            let v = self
                .git(&["show", &format!("{commit}:{path}")])
                .map(|s| s.split('\n').map(str::to_string).collect());
            self.files.insert(key.clone(), v);
        }
        self.files[&key].clone()
    }

    fn intro_commit(&mut self, doc: &str, needle: &str) -> Option<String> {
        let key = (doc.to_string(), needle.to_string());
        if !self.intro.contains_key(&key) {
            let v = self
                .git(&["log", "--format=%H", "-S", needle, "--", doc])
                .and_then(|s| s.split_whitespace().last().map(str::to_string));
            self.intro.insert(key.clone(), v);
        }
        self.intro[&key].clone()
    }

    fn hunks(&mut self, commit: &str, path: &str) -> Vec<Hunk> {
        let key = (commit.to_string(), path.to_string());
        if !self.hunks.contains_key(&key) {
            let out = self
                .git(&["diff", "-U0", commit, "HEAD", "--", path])
                .unwrap_or_default();
            let hs = out.lines().filter_map(hunk_header).collect();
            self.hunks.insert(key.clone(), hs);
        }
        self.hunks[&key].clone()
    }

    fn in_repo(&mut self, path: &str) -> bool {
        if self.file_at("HEAD", path).is_some() {
            return true;
        }
        if !self.logged.contains_key(path) {
            let v = self
                .git(&["log", "-1", "--format=%H", "--", path])
                .is_some_and(|s| !s.trim().is_empty());
            self.logged.insert(path.to_string(), v);
        }
        self.logged[path]
    }

    /// The line at HEAD of line `n` of `path` at `commit`: (line, how) or (None, why).
    fn map_line(&mut self, commit: &str, path: &str, n: u64) -> (Option<u64>, String) {
        let Some(old) = self.file_at(commit, path) else {
            return (None, "file absent at intro commit".into());
        };
        let Some(new) = self.file_at("HEAD", path) else {
            return (None, "file absent at HEAD".into());
        };
        if n < 1 || n > old.len() as u64 {
            return (
                None,
                format!("line past end at intro ({} lines)", old.len()),
            );
        }
        let mut offset: i64 = 0;
        for (os, ol, _ns, nl) in self.hunks(commit, path) {
            if ol == 0 {
                // a pure insertion after old line os
                if os < n {
                    offset += nl as i64;
                }
                continue;
            }
            if n < os {
                break;
            }
            if n < os + ol {
                // the line itself changed: match its text
                let text = old[(n - 1) as usize].trim();
                if text.chars().count() >= 8 {
                    let mut hits = new
                        .iter()
                        .enumerate()
                        .filter(|(_, t)| t.trim() == text)
                        .map(|(i, _)| i as u64 + 1);
                    if let (Some(one), None) = (hits.next(), hits.next()) {
                        return (Some(one), "content match".into());
                    }
                }
                return (
                    None,
                    format!(
                        "line changed: {:?}",
                        text.chars().take(60).collect::<String>()
                    ),
                );
            }
            offset += nl as i64 - ol as i64;
        }
        ((n as i64 + offset).try_into().ok(), "diff map".into())
    }

    /// The HEAD line of a pin pattern plus its offset: exactly one line must match.
    fn pin_line(&mut self, path: &str, (pat, off): &(String, i64)) -> Result<u64, String> {
        let p = Pattern::parse(pat)?;
        let head = self.file_at("HEAD", path).unwrap_or_default();
        let hits: Vec<usize> = head
            .iter()
            .enumerate()
            .filter(|(_, t)| p.search(t))
            .map(|(i, _)| i + 1)
            .collect();
        match hits.as_slice() {
            [one] => u64::try_from(*one as i64 + off)
                .map_err(|_| format!("PIN ERROR {path} /{pat}/: offset {off} leaves the file")),
            _ => Err(format!("PIN ERROR {path} /{pat}/: {} matches", hits.len())),
        }
    }
}

/// `@@ -a[,b] +c[,d] @@`: (a, b, c, d), a missing count being 1.
fn hunk_header(line: &str) -> Option<Hunk> {
    let rest = line.strip_prefix("@@ -")?;
    let (old, rest) = rest.split_once(" +")?;
    let (new, _) = rest.split_once(" @@")?;
    let pair = |s: &str| -> Option<(u64, u64)> {
        match s.split_once(',') {
            Some((a, b)) => Some((a.parse().ok()?, b.parse().ok()?)),
            None => Some((s.parse().ok()?, 1)),
        }
    };
    let (a, b) = pair(old)?;
    let (c, d) = pair(new)?;
    Some((a, b, c, d))
}

/// The result of the drift check.
#[derive(Debug, Default)]
pub(crate) struct Drift {
    pub(crate) checked: usize,
    pub(crate) ok: usize,
    pub(crate) new: usize,
    pub(crate) fixed: usize,
    pub(crate) unfixable: usize,
    pub(crate) pinned: usize,
    pub(crate) history: usize,
    pub(crate) external: usize,
    /// The FIX, UNFIXABLE and UNUSED PIN lines.
    pub(crate) lines: Vec<String>,
}

impl Drift {
    /// No FIX and no UNFIXABLE line.
    pub(crate) fn pass(&self) -> bool {
        self.fixed == 0 && self.unfixable == 0
    }

    pub(crate) fn summary(&self) -> String {
        format!(
            "checked={} ok={} (new={}) fixed={} unfixable={} pinned={} history={} external={}",
            self.checked,
            self.ok,
            self.new,
            self.fixed,
            self.unfixable,
            self.pinned,
            self.history,
            self.external
        )
    }
}

/// The drift check of the docs in `root` (a git work tree).
pub(crate) fn drift(root: &Path) -> Result<Drift, String> {
    let pins = load_pins(root)?;
    let mut repo = Repo {
        root,
        files: HashMap::new(),
        intro: HashMap::new(),
        hunks: HashMap::new(),
        logged: HashMap::new(),
    };
    let mut docs: Vec<String> = Vec::new();
    for d in DOC_DIRS {
        let Ok(rd) = std::fs::read_dir(root.join(d)) else {
            continue;
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.ends_with(".md") && e.path().is_file() {
                docs.push(format!("{d}/{name}"));
            }
        }
    }
    docs.sort();
    let mut r = Drift::default();
    let mut used_pins = BTreeSet::new();
    let mut used_history = BTreeSet::new();
    for rel in &docs {
        let name = rel.rsplit('/').next().unwrap_or(rel).to_string();
        let text = read(&root.join(rel))?;
        for (li, line) in text.split('\n').enumerate() {
            let mut last_path: Option<String> = None;
            for t in tokens(line) {
                let mut path = match (&t.kind, &t.path) {
                    (Kind::Bare, _) => match &last_path {
                        None => continue,
                        Some(p) => p.clone(),
                    },
                    (_, Some(p)) => {
                        if after_evidence_tag(&line[..t.start]) || !repo.in_repo(p) {
                            r.external += 1;
                            last_path = None;
                            continue;
                        }
                        p.clone()
                    }
                    (_, None) => continue,
                };
                last_path = Some(path.clone());
                r.checked += 1;
                let at = format!("{rel}:{}", li + 1);
                // History pins come first: the token keeps its number.
                if let Some((hi, h)) = pins.history.iter().enumerate().find(|(_, h)| {
                    h.doc == name && h.token == t.text && line.contains(h.text.as_str())
                }) {
                    used_history.insert(hi);
                    if let Some(p) = &h.path {
                        path = p.clone();
                    }
                    let old = repo.file_at(&h.commit, &path).unwrap_or_default();
                    let inside = |m: u64| (1..=old.len() as u64).contains(&m);
                    let bad = !inside(t.n) || t.n2.is_some_and(|m| m != 0 && !inside(m));
                    let pat = Pattern::parse(&h.pattern)?;
                    if bad || !pat.search(&old[(t.n - 1) as usize]) {
                        r.unfixable += 1;
                        r.lines.push(format!(
                            "UNFIXABLE {at} {} (history {}): line {} of {path} at {} does not match /{}/",
                            t.text, h.commit, t.n, h.commit, h.pattern
                        ));
                    } else {
                        r.ok += 1;
                        r.history += 1;
                    }
                    continue;
                }
                let (n_new, n2_new, how, tag);
                if let Some((pi, (_, pin))) = pins
                    .pins
                    .iter()
                    .enumerate()
                    .find(|(_, (k, _))| k.0 == name && k.1 == t.text)
                {
                    used_pins.insert(pi);
                    if t.kind == Kind::Bare && pin.path != path {
                        r.unfixable += 1;
                        r.lines.push(format!(
                            "UNFIXABLE {at} {}: the pin names {}, the line names {path}",
                            t.text, pin.path
                        ));
                        continue;
                    }
                    let start = repo.pin_line(&pin.path, &pin.start);
                    let end = pin.end.as_ref().map(|e| repo.pin_line(&pin.path, e));
                    match (start, end.transpose()) {
                        (Ok(s), Ok(e)) => {
                            n_new = s;
                            n2_new = e;
                        }
                        (Err(e), _) | (_, Err(e)) => {
                            r.unfixable += 1;
                            r.lines.push(format!("UNFIXABLE {at} {}: {e}", t.text));
                            continue;
                        }
                    }
                    r.pinned += 1;
                    path = pin.path.clone();
                    how = "pin".to_string();
                    tag = "pinned".to_string();
                } else {
                    let Some(c) = repo.intro_commit(rel, &t.text) else {
                        // New in the work tree: it must lie inside the file at HEAD.
                        let head = repo.file_at("HEAD", &path).unwrap_or_default();
                        let len = head.len() as u64;
                        if (1..=len).contains(&t.n) && t.n2.is_none_or(|m| m <= len) {
                            r.ok += 1;
                            r.new += 1;
                        } else {
                            r.unfixable += 1;
                            r.lines.push(format!(
                                "UNFIXABLE {at} {}: new token past the end of {path}",
                                t.text
                            ));
                        }
                        continue;
                    };
                    let (a, how_a) = repo.map_line(&c, &path, t.n);
                    let (b, how_b) = match t.n2 {
                        Some(m) if m != 0 => {
                            let (b, h) = repo.map_line(&c, &path, m);
                            (Some(b), h)
                        }
                        _ => (None, String::new()),
                    };
                    match (a, b) {
                        (Some(a), None) => {
                            n_new = a;
                            n2_new = None;
                        }
                        (Some(a), Some(Some(b))) => {
                            n_new = a;
                            n2_new = Some(b);
                        }
                        (a, _) => {
                            r.unfixable += 1;
                            r.lines.push(format!(
                                "UNFIXABLE {at} {} (intro {}): {}",
                                t.text,
                                &c[..c.len().min(9)],
                                if a.is_none() { how_a } else { how_b }
                            ));
                            continue;
                        }
                    }
                    how = how_a;
                    tag = format!("intro {}", &c[..c.len().min(9)]);
                }
                let repl = render(t.kind, &path, n_new, n2_new);
                let old = if t.kind == Kind::Bare {
                    t.text.trim_matches('`').to_string()
                } else {
                    t.text.clone()
                };
                if repl == old {
                    r.ok += 1;
                } else {
                    r.fixed += 1;
                    let head = repo.file_at("HEAD", &path).unwrap_or_default();
                    let shown = head
                        .get((n_new as usize).wrapping_sub(1))
                        .map_or("?".to_string(), |l| l.trim().chars().take(90).collect());
                    r.lines.push(format!(
                        "FIX       {at} {old} -> {repl} ({tag}, {how})\n          | {shown}"
                    ));
                }
            }
        }
    }
    for (i, (k, _)) in pins.pins.iter().enumerate() {
        if !used_pins.contains(&i) {
            r.lines.push(format!("UNUSED PIN ({:?}, {:?})", k.0, k.1));
        }
    }
    for (i, h) in pins.history.iter().enumerate() {
        if !used_history.contains(&i) {
            // a history row whose token or text changed: the past line is no longer checked
            r.unfixable += 1;
            r.lines.push(format!(
                "UNFIXABLE UNUSED HISTORY PIN ({:?}, {:?}, {:?})",
                h.doc, h.token, h.text
            ));
        }
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    type Tk = (String, Option<String>, u64, Option<u64>, Kind);

    fn tk(line: &str) -> Vec<Tk> {
        tokens(line)
            .into_iter()
            .map(|t| (t.text, t.path, t.n, t.n2, t.kind))
            .collect()
    }

    #[test]
    fn token_forms() {
        let s = |x: &str| x.to_string();
        assert_eq!(
            tk("see crates/ep9_vm/src/ic.rs:953, :962 and `:970-986`; also `ci.yml` lines 38-42."),
            vec![
                (
                    s("crates/ep9_vm/src/ic.rs:953"),
                    Some(s("crates/ep9_vm/src/ic.rs")),
                    953,
                    None,
                    Kind::Colon
                ),
                (s("`:970-986`"), None, 970, Some(986), Kind::Bare),
                (
                    s("`ci.yml` lines 38-42"),
                    Some(s("ci.yml")),
                    38,
                    Some(42),
                    Kind::Lines
                ),
            ]
        );
        // Not after a path character; a range needs digits; no extension, no token.
        assert!(tk("x/a.rs:12").len() == 1);
        assert!(tk("/a.rs:12").is_empty());
        assert!(tk("http://x.org:80").is_empty());
        assert!(tk("README:12 a.:3 .rs:4 a/.rs:5").is_empty());
        assert_eq!(tk("a.rs:12-x")[0].3, None);
        assert_eq!(tk("a.rs:12-14")[0].3, Some(14));
        assert_eq!(tk("a.rs:12-14")[0].0, "a.rs:12-14");
        // Two tokens on one line, and a token right after another one's end.
        assert_eq!(tk("a.rs:1 b.md:2").len(), 2);
        assert!(after_evidence_tag("[TSRS "));
        assert!(after_evidence_tag("text [R16-D4 "));
        assert!(!after_evidence_tag("[TSRS"));
        assert!(!after_evidence_tag("[tsrs "));
        assert!(!after_evidence_tag("(x) "));
    }

    #[test]
    fn patterns() {
        let p = |s: &str| Pattern::parse(s).unwrap();
        assert!(p(r"^pub\(crate\) fn f\(").search("pub(crate) fn f(x)"));
        assert!(!p(r"^pub\(crate\) fn f\(").search(" pub(crate) fn f(x)"));
        assert!(p(r"^mod strf \{$").search("mod strf {"));
        assert!(!p(r"^mod strf \{$").search("mod strf { }"));
        assert!(p(r"x = 1;$").search("let x = 1;"));
        assert!(p(r"a\|b").search("|a|b|"));
        for bad in [r"a.b", r"a+", r"(x)", r"a\d", r"a|b", r"x$y"] {
            assert!(Pattern::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn render_forms() {
        assert_eq!(render(Kind::Colon, "a.rs", 3, None), "a.rs:3");
        assert_eq!(render(Kind::Colon, "a.rs", 3, Some(3)), "a.rs:3");
        assert_eq!(render(Kind::Bare, "a.rs", 3, Some(5)), ":3-5");
        assert_eq!(render(Kind::Lines, "a.rs", 3, Some(5)), "`a.rs` lines 3-5");
        assert_eq!(render(Kind::Lines, "a.rs", 3, None), "`a.rs` line 3");
    }

    #[test]
    fn hunk_headers() {
        assert_eq!(hunk_header("@@ -3,0 +4,2 @@ fn x"), Some((3, 0, 4, 2)));
        assert_eq!(hunk_header("@@ -7 +7 @@"), Some((7, 1, 7, 1)));
        assert_eq!(hunk_header("+@@ x"), None);
    }
}
