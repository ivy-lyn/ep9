//! The H20 core golden (docs/adr/H20-builtin-abi.md, "Freeze table"; M3-P3 FREEZE-PREP task
//! 5): `tests/golden/h20-core.txt` lists every public name and signature of the builtin ABI,
//! read from the source:
//!
//! ```text
//! src/abi/*.rs     every pub item: types (fields, variants, derives), traits, the pub items
//!                  of their impl blocks (`Cx`, `Args`, `Local`, ...), trait impls (also the
//!                  impls that a local `macro_rules!` makes: the `upcast!` and `downcast!`
//!                  lists, expanded, so the line has the generated signature with its
//!                  lifetimes), the `pub use` re-exports
//! src/isolate.rs   the `impl Cx` block (the scoped embedder forms)
//! src/**/*.rs      every pub method of every default-build `impl Isolate` block (a block
//!                  in some default build of some target: no `cfg` that is false for every
//!                  target, such as `test` or a feature, on it, on an inline module around it
//!                  or on its module in `lib.rs`; a target `cfg` is read and shown in the
//!                  line, Codex C2-FP1): the embedder entry points (`ENTRY_POINTS`: the calls that run ABI code or
//!                  install a hook that ABI code calls; H20-Q4) are core `embed:` lines; the
//!                  other methods are `embed (not core):` lines
//! src/builtins.rs  the builtin table types and `check_tables`
//! src/lib.rs       the crate-root re-exports of `abi` and `builtins` (the public paths), and
//!                  the path of `Isolate`
//! ```
//!
//! The `embed (not core):` lines are the embedder API of H22, H23 and H19, with their own
//! freeze points. They are here so that a new `Isolate` method (for example a raw `Value` form
//! that comes back, H20-Q4) is a new golden line that a review must bless; test
//! `no_isolate_method_takes_or_gives_a_raw_value` also fails on it.
//!
//! A line is a signature without parameter names (a rename is not an API change), with the
//! attributes that matter to a caller (`derive`, `repr`, `non_exhaustive`, `cfg`,
//! `doc(hidden)`, `deprecated`). Comments, doc text and bodies are not read. Not here: the
//! append-only lists that have their own goldens (the `IntrinsicId` and `ObjectClass` variants,
//! `tests/golden/vm-builtin-ids.txt`, `tests/golden/object-classes.txt`), private and
//! `pub(crate)` items, and `export_facts` (H20 part 2, V8-A13: only its path is here).
//!
//! The core freezes at M0 exit. After that a core line that changes or goes away is a change to
//! a Frozen part (README §3.2); a new line is an addition to the part-2 `Cx` method list (append
//! only). The test tells the two apart. `EP9_BLESS=1 cargo test -p ep9_vm --test h20_core`
//! rewrites the file; review the diff: after the freeze, only added lines are allowed.

use std::collections::BTreeSet;
use std::path::PathBuf;

fn src_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/h20-core.txt")
}

/// The `.rs` files under `src/dir` (paths under `src/`), sorted, with the subdirectories.
fn rs_files(dir: &str, out: &mut Vec<String>) {
    let mut names: Vec<String> = std::fs::read_dir(src_dir().join(dir))
        .unwrap_or_else(|e| panic!("src/{dir}: {e}"))
        .map(|e| e.expect("a dir entry").file_name().into_string().unwrap())
        .collect();
    names.sort();
    for n in names {
        let p = if dir.is_empty() {
            n.clone()
        } else {
            format!("{dir}/{n}")
        };
        if src_dir().join(&p).is_dir() {
            rs_files(&p, out);
        } else if n.ends_with(".rs") {
            out.push(p);
        }
    }
}

/// The predicate of a `#[cfg(..)]` attribute (normalized), else `None`.
fn cfg_pred(attr: &str) -> Option<&str> {
    attr.strip_prefix("#[cfg(")?.strip_suffix(")]")
}

/// The value of the `cfg` predicate `p` in every default build of the crate, for any target:
/// `Some(false)` if it is false there (`test` is off, and so is each `feature = ..`: ep9_vm
/// has no default feature), `Some(true)` if it is true there, `None` if it depends on the
/// target or the profile (`target_pointer_width = "64"`, `unix`, `debug_assertions`, `miri`,
/// any other name). `not`, `all` and `any` combine the values (Codex C2-FP1: a target `cfg`
/// is on in some default build, so its items are in the golden).
fn cfg_value(p: &str) -> Option<bool> {
    let p = p.trim();
    let args = |op: &str| {
        let r = p.strip_prefix(op)?.trim_start().strip_prefix('(')?;
        Some(split_top(r.strip_suffix(')')?))
    };
    if let Some(a) = args("not") {
        return match a.as_slice() {
            [q] => cfg_value(q).map(|v| !v),
            _ => None,
        };
    }
    if let Some(a) = args("all") {
        let vs: Vec<Option<bool>> = a.iter().map(|q| cfg_value(q)).collect();
        return if vs.contains(&Some(false)) {
            Some(false)
        } else if vs.iter().all(|v| *v == Some(true)) {
            Some(true)
        } else {
            None
        };
    }
    if let Some(a) = args("any") {
        let vs: Vec<Option<bool>> = a.iter().map(|q| cfg_value(q)).collect();
        return if vs.contains(&Some(true)) {
            Some(true)
        } else if vs.iter().all(|v| *v == Some(false)) {
            Some(false)
        } else {
            None
        };
    }
    let name = p.split(['=', ' ']).next().unwrap_or("");
    if name == "test" || name == "feature" {
        return Some(false);
    }
    None
}

/// True if an item with these attributes is in no default build: a `cfg` whose predicate is
/// false there ([`cfg_value`]: `cfg(test)`, `cfg(feature = ..)`, `cfg(all(test, unix))`).
/// A target `cfg` (`cfg(target_pointer_width = "64")`, `cfg(any(test, unix))`) and
/// `cfg(not(test))` are not gated: the item is read, and its line shows the `cfg`.
fn gated(attrs: &[String]) -> bool {
    attrs
        .iter()
        .filter_map(|a| cfg_pred(a))
        .any(|p| cfg_value(p) == Some(false))
}

/// The `cfg` attributes of an enclosing block or module, each with a trailing space.
fn cfg_text(attrs: &[String]) -> String {
    attrs
        .iter()
        .filter(|a| cfg_pred(a).is_some())
        .map(|a| format!("{a} "))
        .collect()
}

/// The source files the golden reads, as (path under `src/`, text): `abi/` sorted by name,
/// `isolate.rs`, `builtins.rs`, `lib.rs`, then the other files of the default build (sorted;
/// read only for their `impl Isolate` blocks). A module that `lib.rs` declares under a `cfg`
/// that is in no default build ([`gated`]: `tests`, `native`) is left out.
fn sources() -> Vec<(String, String)> {
    let read = |p: &str| {
        std::fs::read_to_string(src_dir().join(p))
            .unwrap_or_else(|e| panic!("src/{p}: {e}"))
            .replace("\r\n", "\n")
    };
    let gated_mods: Vec<String> = items(&strip(&read("lib.rs")))
        .into_iter()
        .filter(|it| it.body.is_none() && gated(&it.attrs))
        .filter_map(|it| {
            let name = it.head.strip_prefix("pub ").unwrap_or(&it.head);
            name.strip_prefix("mod ").map(String::from)
        })
        .collect();
    let mut abi = Vec::new();
    rs_files("abi", &mut abi);
    abi.extend(["isolate.rs", "builtins.rs", "lib.rs"].map(String::from));
    let mut all = Vec::new();
    rs_files("", &mut all);
    for p in all {
        let top = p.split('/').next().unwrap().trim_end_matches(".rs");
        if !abi.contains(&p) && !gated_mods.iter().any(|m| m == top) {
            abi.push(p);
        }
    }
    abi.into_iter()
        .map(|p| {
            let text = read(&p);
            (p, text)
        })
        .collect()
}

// ---- source text without comments and literals ---------------------------------------------

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// A char of a string literal as `strip` keeps it.
fn mask(c: char) -> char {
    match c {
        '{' | '}' | '[' | ']' | '(' | ')' | ';' | '/' | '\'' | '\n' => '_',
        c => c,
    }
}

/// `src` with comments removed (each becomes a space), char literals as `' '`, and in string,
/// raw string and byte string literals every bracket, `;` and `/` as `_` (an attribute such as
/// `#[cfg(feature = "x")]` keeps its text), so the brackets and `;` left are code. Lifetimes
/// (`'s`) stay.
fn strip(src: &str) -> String {
    let c: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < c.len() {
        let prev_ident = i > 0 && is_ident(c[i - 1]);
        if c[i] == '/' && c.get(i + 1) == Some(&'/') {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
            out.push(' ');
        } else if c[i] == '/' && c.get(i + 1) == Some(&'*') {
            let mut depth = 0;
            while i < c.len() {
                if c[i] == '/' && c.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if c[i] == '*' && c.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            out.push(' ');
        } else if !prev_ident && (c[i] == 'r' || (c[i] == 'b' && c.get(i + 1) == Some(&'r'))) && {
            let mut j = i + if c[i] == 'b' { 2 } else { 1 };
            while c.get(j) == Some(&'#') {
                j += 1;
            }
            c.get(j) == Some(&'"')
        } {
            // A raw string: r#"..."# with the same number of #.
            let mut j = i + if c[i] == 'b' { 2 } else { 1 };
            let mut hashes = 0;
            while c[j] == '#' {
                hashes += 1;
                j += 1;
            }
            j += 1;
            let start = j;
            loop {
                if c[j] == '"' && (0..hashes).all(|k| c.get(j + 1 + k) == Some(&'#')) {
                    break;
                }
                j += 1;
            }
            out.push('"');
            out.extend(c[start..j].iter().map(|&x| mask(x)));
            out.push('"');
            i = j + 1 + hashes;
        } else if c[i] == '"' || (!prev_ident && c[i] == 'b' && c.get(i + 1) == Some(&'"')) {
            let start = i + if c[i] == 'b' { 2 } else { 1 };
            let mut j = start;
            while c[j] != '"' {
                j += if c[j] == '\\' { 2 } else { 1 };
            }
            out.push('"');
            out.extend(c[start..j].iter().map(|&x| mask(x)));
            out.push('"');
            i = j + 1;
        } else if c[i] == '\'' || (!prev_ident && c[i] == 'b' && c.get(i + 1) == Some(&'\'')) {
            let q = if c[i] == 'b' { i + 1 } else { i };
            if c.get(q + 1) == Some(&'\\') {
                // An escaped char literal: '\n', '\'', '\u{..}'.
                let mut j = q + 3;
                while c[j] != '\'' {
                    j += 1;
                }
                out.push_str("' '");
                i = j + 1;
            } else if c.get(q + 2) == Some(&'\'') {
                out.push_str("' '");
                i = q + 3;
            } else {
                // A lifetime or a label.
                out.push('\'');
                i += 1;
            }
        } else {
            out.push(c[i]);
            i += 1;
        }
    }
    out
}

// ---- items ----------------------------------------------------------------------------------

/// One item of a stripped text: its outer attributes, its head (the text before its body or
/// its `;`) and its `{}` body, if it has one.
#[derive(Debug)]
struct Item {
    attrs: Vec<String>,
    head: String,
    body: Option<String>,
}

/// The index after the bracket that closes the one at `open` (`{`, `[` or `(`).
fn close(c: &[char], open: usize) -> usize {
    let mut depth = 0i32;
    let mut i = open;
    loop {
        match c[i] {
            '{' | '[' | '(' => depth += 1,
            '}' | ']' | ')' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
}

/// The items at the top level of `text` (a stripped file or a stripped body).
fn items(text: &str) -> Vec<Item> {
    let c: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut attrs = Vec::new();
    let mut i = 0;
    while i < c.len() {
        if c[i].is_whitespace() || c[i] == ';' {
            i += 1;
        } else if c[i] == '#' && c.get(i + 1) == Some(&'!') {
            i = close(&c, i + 2);
        } else if c[i] == '#' && c.get(i + 1) == Some(&'[') {
            let end = close(&c, i + 1);
            attrs.push(norm(&c[i..end].iter().collect::<String>()));
            i = end;
        } else {
            let mut j = i;
            let mut depth = 0i32;
            while j < c.len() {
                match c[j] {
                    '(' | '[' => depth += 1,
                    ')' | ']' => depth -= 1,
                    '{' | ';' if depth == 0 => break,
                    _ => {}
                }
                j += 1;
            }
            let head = c[i..j.min(c.len())].iter().collect::<String>();
            let (body, next) = if j < c.len() && c[j] == '{' {
                let end = close(&c, j);
                (Some(c[j + 1..end - 1].iter().collect()), end)
            } else {
                (None, j + 1)
            };
            out.push(Item {
                attrs: std::mem::take(&mut attrs),
                head: norm(&head),
                body,
            });
            i = next;
        }
    }
    out
}

/// The items of a stripped file, with each call of a `macro_rules!` macro of the same file
/// replaced by the items that the call makes ([`expand`]). The definitions are dropped. A call
/// of a macro from elsewhere stays an item (its head ends with `!`).
fn file_items(text: &str) -> Vec<Item> {
    let all = items(text);
    let defs: Vec<(String, String)> = all
        .iter()
        .filter_map(|it| {
            let name = it.head.strip_prefix("macro_rules! ")?;
            Some((name.to_string(), it.body.clone().unwrap_or_default()))
        })
        .collect();
    let mut out = Vec::new();
    for it in all {
        if it.head.starts_with("macro_rules!") {
            continue;
        }
        let def = it
            .head
            .strip_suffix('!')
            .and_then(|n| defs.iter().find(|(d, _)| d == n));
        match def {
            Some((name, def)) => {
                out.extend(items(&expand(name, def, it.body.as_deref().unwrap_or(""))));
            }
            None => out.push(it),
        }
    }
    out
}

/// The text that the call `name! { call }` makes, from the body `def` of `macro_rules! name`.
/// The form read: one rule `($( $a:frag SEP $b:frag ... , )*) => {$( template )*}`; each
/// entry of the call (split at the top-level commas) is split at the fixed tokens of the
/// pattern, and gives one copy of the template with each `$a` replaced. Another form panics:
/// teach this function the form before the golden can read it.
fn expand(name: &str, def: &str, call: &str) -> String {
    let bad = |why: &str| -> ! {
        panic!("macro {name}!: {why}; tests/h20_core.rs `expand` does not read this form")
    };
    // `$( inner )*` -> inner.
    let repeated = |s: &str| -> String {
        let c: Vec<char> = s.trim().chars().collect();
        if c.len() < 4 || c[0] != '$' || c[1] != '(' || c[c.len() - 1] != '*' {
            bad("not a `$( .. )*` repetition");
        }
        if close(&c, 1) != c.len() - 1 {
            bad("text after the repetition");
        }
        c[2..c.len() - 2].iter().collect()
    };
    let c: Vec<char> = def.chars().collect();
    let mut i = 0;
    let skip_ws = |i: &mut usize| {
        while *i < c.len() && c[*i].is_whitespace() {
            *i += 1;
        }
    };
    skip_ws(&mut i);
    if c.get(i) != Some(&'(') {
        bad("no `(` matcher");
    }
    let m_end = close(&c, i);
    let matcher = repeated(&c[i + 1..m_end - 1].iter().collect::<String>());
    i = m_end;
    skip_ws(&mut i);
    if !(c.get(i) == Some(&'=') && c.get(i + 1) == Some(&'>')) {
        bad("no `=>` after the matcher");
    }
    i += 2;
    skip_ws(&mut i);
    if c.get(i) != Some(&'{') {
        bad("no `{` template");
    }
    let t_end = close(&c, i);
    let template = repeated(&c[i + 1..t_end - 1].iter().collect::<String>());
    i = t_end;
    while i < c.len() && (c[i].is_whitespace() || c[i] == ';') {
        i += 1;
    }
    if i != c.len() {
        bad("more than one rule");
    }
    // The pattern: variables and the fixed tokens between them, then `,`.
    let m: Vec<char> = matcher.chars().collect();
    let mut vars = Vec::new();
    let mut fixed = Vec::new();
    let mut lit = String::new();
    let mut k = 0;
    while k < m.len() {
        if m[k] == '$' {
            if !vars.is_empty() {
                fixed.push(lit.trim().to_string());
            } else if !lit.trim().is_empty() {
                bad("a fixed token before the first variable");
            }
            lit.clear();
            k += 1;
            let start = k;
            while k < m.len() && is_ident(m[k]) {
                k += 1;
            }
            vars.push(m[start..k].iter().collect::<String>());
            if m.get(k) != Some(&':') {
                bad("a variable with no fragment");
            }
            k += 1;
            while k < m.len() && is_ident(m[k]) {
                k += 1;
            }
        } else {
            lit.push(m[k]);
            k += 1;
        }
    }
    if vars.is_empty() || lit.trim() != "," || fixed.iter().any(|f| f.is_empty()) {
        bad("a pattern that does not end with `,`, or two variables with no token between");
    }
    let mut out = String::new();
    for entry in split_top(call) {
        let mut values = Vec::new();
        let mut rest = entry.as_str();
        for f in &fixed {
            let Some((v, r)) = rest.split_once(f.as_str()) else {
                bad(&format!("the entry {entry:?} has no {f:?}"));
            };
            values.push(v.trim().to_string());
            rest = r;
        }
        values.push(rest.trim().to_string());
        let t: Vec<char> = template.chars().collect();
        let mut k = 0;
        while k < t.len() {
            if t[k] == '$' {
                let start = k + 1;
                k = start;
                while k < t.len() && is_ident(t[k]) {
                    k += 1;
                }
                let v: String = t[start..k].iter().collect();
                match vars.iter().position(|x| *x == v) {
                    Some(n) => out.push_str(&values[n]),
                    None => bad(&format!(
                        "the template names ${v}, which the pattern has not"
                    )),
                }
            } else {
                out.push(t[k]);
                k += 1;
            }
        }
        out.push('\n');
    }
    out
}

/// Whitespace runs to one space; no space inside `()` and `[]`, after the `<` of generics
/// or before `,`; no trailing comma; `{}` for an empty brace pair.
fn norm(s: &str) -> String {
    let mut out = String::new();
    for w in s.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
    }
    for (from, to) in [
        ("( ", "("),
        (" )", ")"),
        ("[ ", "["),
        (" ]", "]"),
        (" ,", ","),
        (",)", ")"),
        (",]", "]"),
        (",>", ">"),
        (",}", "}"),
        (", )", ")"),
        (", ]", "]"),
        (", >", ">"),
        (", }", " }"),
        ("{ }", "{}"),
    ] {
        while out.contains(from) {
            out = out.replace(from, to);
        }
    }
    // `Foo< A` (a generic list broken over lines) -> `Foo<A`; `1 << 3` stays.
    let c: Vec<char> = out.chars().collect();
    let mut s = String::with_capacity(out.len());
    for i in 0..c.len() {
        if c[i] == ' ' && i >= 2 && c[i - 1] == '<' && is_ident(c[i - 2]) {
            continue;
        }
        s.push(c[i]);
    }
    s
}

/// True if the head declares a `fn` (`pub const fn` too), not a `const` whose type or value
/// has a `fn` or `=`.
fn is_fn_head(head: &str) -> bool {
    let end = head.find(['(', '<', ':', '=']).unwrap_or(head.len());
    head[..end].split(' ').any(|w| w == "fn")
}

/// Splits `s` at the commas outside brackets (`()`, `[]`, `<>`, `{}`; the `>` of `->` and
/// `=>` closes nothing). Empty parts are dropped.
fn split_top(s: &str) -> Vec<String> {
    let c: Vec<char> = s.chars().collect();
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for i in 0..c.len() {
        match c[i] {
            '(' | '[' | '<' | '{' => depth += 1,
            '>' if i > 0 && (c[i - 1] == '-' || c[i - 1] == '=') => {}
            ')' | ']' | '>' | '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(c[start..i].iter().collect::<String>());
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(c[start..].iter().collect());
    parts
        .into_iter()
        .map(|p| norm(&p))
        .filter(|p| !p.is_empty())
        .collect()
}

/// The attributes a caller depends on, joined with a trailing space ("" if none).
fn kept_attrs(attrs: &[String]) -> String {
    let mut s = String::new();
    for a in attrs {
        let name = a.trim_start_matches("#[");
        if [
            "derive",
            "repr",
            "non_exhaustive",
            "cfg",
            "doc(hidden)",
            "deprecated",
        ]
        .iter()
        .any(|k| name.starts_with(k))
        {
            s.push_str(a);
            s.push(' ');
        }
    }
    s
}

/// Leading outer attributes of a field, variant or parameter text, split off.
fn split_attrs(s: &str) -> (Vec<String>, String) {
    let c: Vec<char> = s.chars().collect();
    let mut attrs = Vec::new();
    let mut i = 0;
    loop {
        while i < c.len() && c[i].is_whitespace() {
            i += 1;
        }
        if c.get(i) == Some(&'#') && c.get(i + 1) == Some(&'[') {
            let end = close(&c, i + 1);
            attrs.push(norm(&c[i..end].iter().collect::<String>()));
            i = end;
        } else {
            break;
        }
    }
    (attrs, norm(&c[i..].iter().collect::<String>()))
}

/// The index of the first `:` at bracket depth 0 that is not part of `::`.
fn type_colon(p: &str) -> Option<usize> {
    let b = p.as_bytes();
    let mut depth = 0i32;
    for i in 0..b.len() {
        match b[i] {
            b'(' | b'[' | b'<' => depth += 1,
            b'>' if i > 0 && b[i - 1] == b'-' => {}
            b')' | b']' | b'>' => depth -= 1,
            b':' if depth == 0 && b.get(i + 1) != Some(&b':') && (i == 0 || b[i - 1] != b':') => {
                return Some(i);
            }
            _ => {}
        }
    }
    None
}

/// The index after the `>` that closes the `<` at `open` (the `>` of `->` closes nothing).
fn close_angle(c: &[char], open: usize) -> usize {
    let mut depth = 0i32;
    let mut i = open;
    loop {
        match c[i] {
            '<' => depth += 1,
            '>' if c[i - 1] == '-' => {}
            '>' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
}

/// A `fn` head without parameter names: `pub fn get(&mut self, o: Local<'s, JsObject>)` ->
/// `pub fn get(&mut self, Local<'s, JsObject>)`.
fn fn_sig(head: &str) -> String {
    let c: Vec<char> = head.chars().collect();
    // `fn`, the name, the generics, then the parameter list.
    let Some(mut k) =
        (0..c.len()).find(|&i| c[i..].starts_with(&['f', 'n', ' ']) && (i == 0 || c[i - 1] == ' '))
    else {
        return head.to_string();
    };
    k += 3;
    while k < c.len() && (is_ident(c[k]) || c[k] == ' ') {
        k += 1;
    }
    if c.get(k) == Some(&'<') {
        k = close_angle(&c, k);
    }
    while c.get(k) == Some(&' ') {
        k += 1;
    }
    if c.get(k) != Some(&'(') {
        return head.to_string();
    }
    let open = k;
    let end = close(&c, open);
    let params: String = c[open + 1..end - 1].iter().collect();
    let mut kept = Vec::new();
    for p in split_top(&params) {
        let (attrs, p) = split_attrs(&p);
        let selfish = p == "self"
            || p.ends_with("&self")
            || p.ends_with("mut self")
            || p.starts_with("self:")
            || p.starts_with("mut self:")
            || (p.starts_with('&') && p.ends_with(" self"));
        let ty = if selfish {
            p
        } else {
            match type_colon(&p) {
                Some(k) => norm(&p[k + 1..]),
                None => p,
            }
        };
        kept.push(format!("{}{ty}", kept_attrs(&attrs)));
    }
    let before: String = c[..open].iter().collect();
    let after: String = c[end..].iter().collect();
    // A `where` clause ends with a comma before the body.
    let sig = norm(&format!("{before}({}){after}", kept.join(", ")));
    sig.trim_end_matches(',').to_string()
}

/// The name of an impl's self type: `impl<'s> abi::Cx<'s>` -> `Cx`; `impl X for &str` ->
/// `&str`. And the trait name for a trait impl.
fn impl_target(head: &str) -> (Option<String>, String) {
    // Drop `impl` and its generics.
    let rest = head.trim_start_matches("impl");
    let rest = if rest.starts_with('<') {
        let c: Vec<char> = rest.chars().collect();
        let mut depth = 0;
        let mut k = 0;
        for (i, &ch) in c.iter().enumerate() {
            match ch {
                '<' => depth += 1,
                '>' if i > 0 && c[i - 1] == '-' => {}
                '>' => {
                    depth -= 1;
                    if depth == 0 {
                        k = i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        c[k..].iter().collect::<String>()
    } else {
        rest.to_string()
    };
    let rest = norm(&rest);
    let last_name = |s: &str| {
        let s = s.split('<').next().unwrap_or("").trim();
        s.rsplit("::").next().unwrap_or("").to_string()
    };
    match rest.split_once(" for ") {
        Some((tr, ty)) => (Some(last_name(tr)), last_name(ty)),
        None => (None, last_name(&rest)),
    }
}

/// `abi::Cx<'s>` -> `Cx<'s>`: module paths of the crate's own items are not part of a line.
fn drop_paths(s: &str) -> String {
    s.replace("crate::abi::", "")
        .replace("abi::", "")
        .replace("crate::", "")
}

fn is_pub(head: &str) -> bool {
    head.starts_with("pub ")
}

/// The name an item declares: `pub struct Args<'s>` -> `Args`.
fn item_name(head: &str) -> Option<String> {
    let words: Vec<&str> = head.split(' ').collect();
    for (k, w) in words.iter().enumerate() {
        if ["struct", "enum", "trait", "type", "fn", "const", "static"].contains(w) {
            let n = words.get(k + 1)?;
            let n: String = n.chars().take_while(|&ch| is_ident(ch)).collect();
            return Some(n);
        }
    }
    None
}

/// The `Isolate` methods in the core (H20-Q4, M3-P3 FREEZE-PREP step 2; fix round 5, Codex
/// W4-3): the embedder's ways into ABI code. A `Cx` scope (`with_cx`, `with_cx_checkpoint`,
/// `create_realm`), the calls that run JS or host functions with no scope of the embedder
/// (`perform_microtask_checkpoint` runs the jobs; `take_unhandled_rejections` runs ToString of
/// an object reason), and the hooks that ABI code calls (`new` takes them in `IsolateConfig`:
/// the builtin tables and the `PromiseRejectionTracker`; `register_host_function` takes a
/// `HostFn`; `with_module_host` takes the hook types of `Cx::run_module`). The other
/// `Isolate` methods (the handles, `collect_garbage`, `gc_stats`, `string_view`, ...) are the
/// embedder API of H22, H23 and H19, with their own freeze points: they are
/// `embed (not core):` lines.
const ENTRY_POINTS: [&str; 8] = [
    "create_realm",
    "new",
    "perform_microtask_checkpoint",
    "register_host_function",
    "take_unhandled_rejections",
    "with_cx",
    "with_cx_checkpoint",
    "with_module_host",
];

/// The append-only lists that have their own goldens (H20 part 2): their variants and their
/// `ALL` values are not core lines.
const PART2_ENUMS: [&str; 2] = ["IntrinsicId", "ObjectClass"];

/// The lines of one public item `it` (not an impl) of group `g`.
fn item_lines(g: &str, it: &Item, out: &mut Vec<String>) {
    let attrs = kept_attrs(&it.attrs);
    let head = drop_paths(&it.head);
    let name = item_name(&it.head).unwrap_or_default();
    if head.contains(" struct ") || head.starts_with("pub struct") {
        let mut pub_fields = Vec::new();
        let mut private = false;
        let mut head = head.clone();
        if let Some(b) = head.find('(') {
            // A tuple struct: its fields are in the head.
            let c: Vec<char> = head.chars().collect();
            let open = head[..b].chars().count();
            let end = close(&c, open);
            let inner: String = c[open + 1..end - 1].iter().collect();
            let mut shown = Vec::new();
            for f in split_top(&inner) {
                if f.starts_with("pub ") {
                    shown.push(f);
                } else {
                    shown.push("_".to_string());
                }
            }
            head = format!(
                "{}({})",
                c[..open].iter().collect::<String>(),
                shown.join(", ")
            );
        }
        if let Some(body) = &it.body {
            for f in split_top(body) {
                let (fa, f) = split_attrs(&f);
                if f.starts_with("pub ") {
                    pub_fields.push(format!("{}{f}", kept_attrs(&fa)));
                } else {
                    private = true;
                }
            }
            if private {
                pub_fields.push("..".to_string());
            }
            out.push(format!(
                "{g}: {attrs}{head} {{ {} }}",
                pub_fields.join(", ")
            ));
        } else {
            out.push(format!("{g}: {attrs}{head}"));
        }
    } else if head.starts_with("pub enum") {
        let body = it.body.clone().unwrap_or_default();
        let variants = if PART2_ENUMS.contains(&name.as_str()) {
            vec![format!(".. (part 2: the {name} golden)")]
        } else {
            split_top(&body)
                .into_iter()
                .map(|v| {
                    let (va, v) = split_attrs(&v);
                    format!("{}{v}", kept_attrs(&va))
                })
                .collect()
        };
        out.push(norm(&format!(
            "{g}: {attrs}{head} {{ {} }}",
            variants.join(", ")
        )));
    } else if head.starts_with("pub trait") {
        let mut fns = Vec::new();
        for m in items(it.body.as_deref().unwrap_or("")) {
            fns.push(format!(
                "{}{}",
                kept_attrs(&m.attrs),
                drop_paths(&fn_sig(&m.head))
            ));
        }
        out.push(format!("{g}: {attrs}{head} {{ {} }}", fns.join("; ")));
    } else if head.starts_with("pub type") {
        out.push(format!(
            "{g}: {attrs}{}",
            drop_paths(&fn_sig_in_type(&head))
        ));
    } else if is_fn_head(&head) {
        out.push(format!("{g}: {attrs}{}", drop_paths(&fn_sig(&head))));
    } else if head.starts_with("pub const") || head.starts_with("pub static") {
        out.push(format!(
            "{g}: {attrs}{}",
            const_line(&head, it.body.as_deref(), "")
        ));
    }
}

/// A type alias as it is (a fn pointer type names no parameters).
fn fn_sig_in_type(head: &str) -> String {
    head.to_string()
}

/// `pub const NAME: T = value` with its value; for the `ALL` list of a part-2 enum, the
/// count and the entries are not core.
fn const_line(head: &str, body: Option<&str>, owner: &str) -> String {
    let head = drop_paths(head);
    if PART2_ENUMS.contains(&owner) && item_name(&head).as_deref() == Some("ALL") {
        let ty = head.split_once(':').map(|(_, t)| t).unwrap_or("");
        let ty = ty.split(" = ").next().unwrap_or("").trim();
        let ty: String = match ty.rfind(';') {
            Some(k) if ty.starts_with('[') => format!("{}; N]", &ty[..k]),
            _ => ty.to_string(),
        };
        let name = head.split(':').next().unwrap_or("");
        return format!("{name}: {ty} = .. (part 2: the {owner} golden)");
    }
    match body {
        Some(b) => format!("{head} {{ {} }}", split_top(b).join(", ")),
        None => head,
    }
}

/// The lines of the default-build `impl Isolate` blocks of `its` and of the inline modules in
/// it: an entry point ([`ENTRY_POINTS`]) is a core `embed:` line, any other pub method or
/// const an `embed (not core):` line. A block or module that is in no default build
/// ([`gated`]) is skipped. `outer` is the `cfg` text of the enclosing modules; each line
/// shows it and the block's own `cfg` before the method's attributes.
fn isolate_lines(its: &[Item], outer: &str, out: &mut Vec<String>) {
    for it in its {
        if gated(&it.attrs) {
            continue;
        }
        let outer = format!("{outer}{}", cfg_text(&it.attrs));
        let words: Vec<&str> = it.head.split(' ').collect();
        let is_mod = words
            .iter()
            .position(|w| *w == "mod")
            .is_some_and(|k| words[..k].iter().all(|w| w.starts_with("pub")));
        if is_mod {
            if let Some(b) = &it.body {
                isolate_lines(&items(b), &outer, out);
            }
            continue;
        }
        if !it.head.starts_with("impl") || impl_target(&it.head) != (None, "Isolate".to_string()) {
            continue;
        }
        for m in items(it.body.as_deref().unwrap_or("")) {
            if !is_pub(&m.head) {
                continue;
            }
            let name = item_name(&m.head).unwrap_or_default();
            let (g, line) = if is_fn_head(&m.head) {
                let g = if ENTRY_POINTS.contains(&name.as_str()) {
                    "embed"
                } else {
                    "embed (not core)"
                };
                (g, drop_paths(&fn_sig(&m.head)))
            } else {
                (
                    "embed (not core)",
                    const_line(&m.head, m.body.as_deref(), "Isolate"),
                )
            };
            out.push(format!(
                "{g}: impl Isolate :: {outer}{}{line}",
                kept_attrs(&m.attrs)
            ));
        }
    }
}

/// True for the files whose items (not only their `impl Isolate` blocks) the golden reads.
fn is_core_file(p: &str) -> bool {
    p.starts_with("abi/") || ["isolate.rs", "builtins.rs", "lib.rs"].contains(&p)
}

/// Every line of the golden from `files` ((path under `src/`, text)).
fn extract(files: &[(String, String)]) -> Vec<String> {
    let stripped: Vec<(String, Vec<Item>)> = files
        .iter()
        .map(|(p, t)| {
            let t = strip(t);
            // The local macros of the ABI files are expanded (the marker lists).
            let its = if is_core_file(p) {
                file_items(&t)
            } else {
                items(&t)
            };
            (p.clone(), its)
        })
        .collect();
    let group = |p: &str| if p == "builtins.rs" { "tables" } else { "abi" };
    // The public types and traits of the ABI files: their impls are part of the ABI.
    let mut pub_types = BTreeSet::new();
    let mut pub_traits = BTreeSet::new();
    for (p, its) in &stripped {
        if !(p.starts_with("abi/") || p == "builtins.rs") {
            continue;
        }
        for it in its {
            if !is_pub(&it.head) {
                continue;
            }
            let Some(n) = item_name(&it.head) else {
                continue;
            };
            if it.head.contains("trait ") {
                pub_traits.insert(n);
            } else if ["struct ", "enum ", "type "]
                .iter()
                .any(|k| it.head.contains(k))
            {
                pub_types.insert(n);
            }
        }
    }
    // The `cfg` text of each module that `lib.rs` declares (a target `cfg` is read: `gated`).
    let mod_cfgs: Vec<(String, String)> = stripped
        .iter()
        .filter(|(p, _)| p == "lib.rs")
        .flat_map(|(_, its)| its)
        .filter(|it| it.body.is_none())
        .filter_map(|it| {
            let name = it.head.strip_prefix("pub ").unwrap_or(&it.head);
            let name = name.strip_prefix("mod ")?;
            Some((name.to_string(), cfg_text(&it.attrs)))
        })
        .collect();
    let mut out = Vec::new();
    let mut abi_mod_uses: BTreeSet<String> = BTreeSet::new();
    for (p, its) in &stripped {
        let top = p.split('/').next().unwrap_or("").trim_end_matches(".rs");
        let outer = mod_cfgs
            .iter()
            .find(|(m, _)| m == top)
            .map_or("", |(_, c)| c.as_str());
        isolate_lines(its, outer, &mut out);
        if !is_core_file(p) {
            continue;
        }
        let g = group(p);
        let is_abi = p.starts_with("abi/") || p == "builtins.rs";
        for it in its {
            if p == "lib.rs" {
                lib_paths(it, &mut out);
                continue;
            }
            if p == "abi/mod.rs" && it.head.starts_with("pub use ") {
                for n in use_names(&use_text(it)) {
                    let name = n.rsplit("::").next().unwrap().to_string();
                    let note = if name == "export_facts" {
                        " (part 2, not core: V8-A13)"
                    } else {
                        ""
                    };
                    out.push(format!("path: ep9_vm::abi::{name}{note}"));
                    abi_mod_uses.insert(name);
                }
                continue;
            }
            if it.head.starts_with("impl") && impl_target(&it.head).1 == "Isolate" {
                // `isolate_lines` read it.
                continue;
            }
            if it.head.starts_with("impl") {
                let (tr, ty) = impl_target(&it.head);
                let ours =
                    pub_types.contains(&ty) || tr.as_ref().is_some_and(|t| pub_traits.contains(t));
                if !ours || (!is_abi && ty != "Cx") {
                    continue;
                }
                let ih = drop_paths(&it.head);
                if tr.is_some() {
                    out.push(format!("{g}: {ih}"));
                    continue;
                }
                for m in items(it.body.as_deref().unwrap_or("")) {
                    if !is_pub(&m.head) {
                        continue;
                    }
                    let a = kept_attrs(&m.attrs);
                    let line = if is_fn_head(&m.head) {
                        drop_paths(&fn_sig(&m.head))
                    } else {
                        const_line(&m.head, m.body.as_deref(), &ty)
                    };
                    out.push(format!("{g}: {ih} :: {a}{line}"));
                }
                continue;
            }
            if !is_abi {
                continue;
            }
            if it.head.ends_with('!') {
                // A top-level call of a macro from outside the file (`file_items` expanded
                // the local ones): its input, as the golden cannot see what it makes.
                let name = it.head.trim_end_matches('!');
                let entries = split_top(it.body.as_deref().unwrap_or(""));
                out.push(format!("{g}: {name}! {{ {} }}", entries.join(", ")));
                continue;
            }
            if is_pub(&it.head) && !it.head.starts_with("pub use") {
                item_lines(g, it, &mut out);
                if p == "abi/mod.rs"
                    && let Some(n) = item_name(&it.head)
                {
                    out.push(format!("path: ep9_vm::abi::{n}"));
                    abi_mod_uses.insert(n);
                }
            }
        }
    }
    // A pub type of an `abi` submodule that `abi/mod.rs` does not re-export has no path.
    for (p, its) in &stripped {
        if !p.starts_with("abi/") || p == "abi/mod.rs" {
            continue;
        }
        for it in its {
            if !is_pub(&it.head) || it.head.starts_with("pub use") {
                continue;
            }
            let Some(n) = item_name(&it.head) else {
                continue;
            };
            if !abi_mod_uses.contains(&n) {
                out.push(format!(
                    "path: (none) {n}: pub in the private module abi::{}, not re-exported",
                    p.trim_start_matches("abi/").trim_end_matches(".rs")
                ));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The names of a `pub use a::{B, C}` or `pub use a::B` head.
fn use_names(head: &str) -> Vec<String> {
    let rest = head.trim_start_matches("pub use ").trim();
    if let (Some(a), Some(b)) = (rest.find('{'), rest.rfind('}')) {
        let prefix = &rest[..a];
        split_top(&rest[a + 1..b])
            .into_iter()
            .flat_map(|n| {
                if n.contains('{') {
                    use_names(&format!("pub use {prefix}{n}"))
                } else {
                    vec![format!("{prefix}{n}")]
                }
            })
            .collect()
    } else {
        vec![rest.to_string()]
    }
}

/// The text of a `use` item: `items` stops a head at `{`, so `pub use x::{..};` comes as a
/// head and a body.
fn use_text(it: &Item) -> String {
    match &it.body {
        Some(b) => format!("{}{{{b}}}", it.head),
        None => it.head.clone(),
    }
}

/// The crate-root paths of `abi` and `builtins` names from a `lib.rs` item.
fn lib_paths(it: &Item, out: &mut Vec<String>) {
    if !it.head.starts_with("pub use ") {
        return;
    }
    for n in use_names(&use_text(it)) {
        let n = norm(&n);
        let module = n.split("::").next().unwrap_or("");
        if module == "abi" || module == "builtins" || n == "isolate::Isolate" {
            let name = n.rsplit("::").next().unwrap();
            out.push(format!("path: ep9_vm::{name} = {n}"));
        }
    }
}

// ---- the golden ------------------------------------------------------------------------------

const HEADER: &str = "\
# H20 core: the public names and signatures of the builtin ABI (docs/adr/H20-builtin-abi.md,
# Freeze table). Frozen at M0 exit: after it, a changed or removed core line is a change to a
# Frozen part; a new line is an addition (part 2, append only). The `embed (not core):` lines
# are the other `Isolate` methods (the embedder API of H22, H23, H19): not core, but a new one
# needs a review (H20-Q4). Made by
# `EP9_BLESS=1 cargo test -p ep9_vm --test h20_core`; see crates/ep9_vm/tests/h20_core.rs.
";

/// The prefix of a line that is not core: an `Isolate` method that is not an entry point.
const NOT_CORE: &str = "embed (not core): ";

fn text(lines: &[String]) -> String {
    let mut s = HEADER.to_string();
    for l in lines {
        s.push_str(l);
        s.push('\n');
    }
    s
}

fn golden_lines() -> Vec<String> {
    std::fs::read_to_string(golden_path())
        .unwrap_or_else(|e| panic!("{}: {e}; run with EP9_BLESS=1", golden_path().display()))
        .replace("\r\n", "\n")
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(String::from)
        .collect()
}

/// (lines of the golden that the source no longer has, lines of the source that are new).
fn diff(golden: &[String], now: &[String]) -> (Vec<String>, Vec<String>) {
    let g: BTreeSet<&String> = golden.iter().collect();
    let n: BTreeSet<&String> = now.iter().collect();
    (
        g.difference(&n).map(|s| s.to_string()).collect(),
        n.difference(&g).map(|s| s.to_string()).collect(),
    )
}

#[test]
fn h20_core_matches_the_golden() {
    let now = extract(&sources());
    if std::env::var_os("EP9_BLESS").is_some() {
        std::fs::write(golden_path(), text(&now)).unwrap();
        return;
    }
    let (gone, new) = diff(&golden_lines(), &now);
    let (other_gone, core_gone): (Vec<String>, Vec<String>) =
        gone.into_iter().partition(|l| l.starts_with(NOT_CORE));
    assert!(
        core_gone.is_empty() && other_gone.is_empty() && new.is_empty(),
        "the H20 core golden changed.\n\
         changed or removed core lines (after M0 exit a Frozen-part change, README §3.2):\n  {}\n\
         changed or removed `embed (not core):` lines (the embedder API of H22, H23, H19; \
         not an H20 core change):\n  {}\n\
         new (a part-2 addition, or a new `Isolate` method: review it, H20-Q4, then run with \
         EP9_BLESS=1):\n  {}",
        core_gone.join("\n  "),
        other_gone.join("\n  "),
        new.join("\n  ")
    );
}

/// The core has its parts: `HostFn`, `Args`, `Local`, the `Cx` methods (also the scoped
/// embedder forms of isolate.rs), the table types, the public paths.
#[test]
fn the_golden_covers_the_core() {
    let lines = golden_lines();
    let has = |s: &str| lines.iter().any(|l| l.contains(s));
    for needle in [
        "pub type HostFn = for<'s> fn(&mut Cx<'s>, Local<'s>, Args<'s>, Local<'s>) -> JsResult<Local<'s>>",
        "impl<'s> Args<'s> :: pub fn len(&self) -> usize",
        "pub struct Local<'s, T = JsValue> { .. }",
        "impl<'s> Cx<'s> :: pub fn run_script(&mut self, VerifiedUnit, &str) -> JsResult<Local<'s>>",
        "impl<'s> Cx<'s> :: pub fn get_index(",
        "tables: #[derive(Clone, Copy)] pub struct BuiltinDef { pub name: &'static str,",
        "path: ep9_vm::Cx = abi::Cx",
        "path: ep9_vm::abi::export_facts (part 2, not core: V8-A13)",
        "path: ep9_vm::CollectionCursor = abi::CollectionCursor",
        "path: ep9_vm::Isolate = isolate::Isolate",
        "abi: impl<'s> From<Local<'s, JsObject>> for Local<'s, JsValue>",
        "abi: impl<'s> From<Local<'s, JsSymbol>> for Local<'s, JsKey>",
        "abi: impl Downcast for JsValue",
        "embed: impl Isolate :: pub fn new(IsolateConfig)",
        "embed: impl Isolate :: pub fn take_unhandled_rejections(&mut self)",
        "embed (not core): impl Isolate :: pub fn gc_stats(&self) -> GcStats",
        "embed (not core): impl Isolate :: pub fn string_view(&mut self, Value)",
    ] {
        assert!(has(needle), "no line has {needle:?}");
    }
    let cx = lines
        .iter()
        .filter(|l| l.contains("impl<'s> Cx<'s> :: pub fn"))
        .count();
    assert!(cx > 100, "{cx} Cx methods");
    assert!(
        !lines.iter().any(|l| l.starts_with("path: (none)")),
        "an ABI type with no public path"
    );
}

/// Each embedder entry point (H20-Q4) has one golden line, and no other `Isolate` method has
/// one: a rename or a removal of an entry point fails here and in the golden.
#[test]
fn the_golden_has_the_embedder_entry_points() {
    let lines = golden_lines();
    let embed: Vec<&String> = lines.iter().filter(|l| l.starts_with("embed: ")).collect();
    for name in ENTRY_POINTS {
        let n = embed
            .iter()
            .filter(|l| {
                l.contains(&format!(":: pub fn {name}<"))
                    || l.contains(&format!(":: pub fn {name}("))
            })
            .count();
        assert_eq!(n, 1, "{name}: {embed:?}");
    }
    assert_eq!(embed.len(), ENTRY_POINTS.len(), "{embed:?}");
}

/// The negative control: a changed return type is a changed core line (the old line is gone,
/// a new one is there).
#[test]
fn changing_a_signature_fails_the_golden() {
    let mut files = sources();
    let (_, t) = files.iter_mut().find(|(p, _)| p == "abi/mod.rs").unwrap();
    let from = "pub fn len(&self) -> usize";
    assert!(t.contains(from));
    *t = t.replacen(from, "pub fn len(&self) -> u32", 1);
    let (gone, new) = diff(&golden_lines(), &extract(&files));
    assert_eq!(
        gone,
        ["abi: impl<'s> Args<'s> :: pub fn len(&self) -> usize"]
    );
    assert_eq!(new, ["abi: impl<'s> Args<'s> :: pub fn len(&self) -> u32"]);
}

/// The negative control for the embedder entry points: a changed `Isolate` entry point is a
/// changed core line, and a test-only `impl Isolate` block adds no line.
#[test]
fn changing_an_entry_point_fails_the_golden() {
    let mut files = sources();
    let (_, t) = files.iter_mut().find(|(p, _)| p == "isolate.rs").unwrap();
    let from = "pub fn register_host_function(&mut self, name: &str, length: u32, f: HostFn)";
    assert!(t.contains(from));
    *t = t.replacen(
        from,
        "pub fn register_host_function(&mut self, name: &str, length: u64, f: HostFn)",
        1,
    );
    t.push_str("\n#[cfg(test)]\nimpl Isolate {\n    pub fn with_cx(&self) {}\n}\n");
    let (gone, new) = diff(&golden_lines(), &extract(&files));
    assert_eq!(
        gone,
        ["embed: impl Isolate :: pub fn register_host_function(&mut self, &str, u32, HostFn)"]
    );
    assert_eq!(
        new,
        ["embed: impl Isolate :: pub fn register_host_function(&mut self, &str, u64, HostFn)"]
    );
}

/// The negative control for the impls that a marker macro makes (Codex C-FP1): an upcast
/// that gives the result a free lifetime (`Local<'s, A>` to `Local<'t, B>`, so `.into()`
/// could take a `Local` out of `with_cx`) changes every `From` line of the `upcast!` list.
/// The `compile_fail` doctest on `Local` (src/abi/local.rs) shows that the escape does not
/// compile with the frozen impl.
#[test]
fn changing_a_generated_impl_fails_the_golden() {
    let mut files = sources();
    let (_, t) = files.iter_mut().find(|(p, _)| p == "abi/local.rs").unwrap();
    let from = "impl<'s> From<Local<'s, $from>> for Local<'s, $to>";
    assert!(t.contains(from));
    *t = t.replacen(
        from,
        "impl<'s, 't> From<Local<'s, $from>> for Local<'t, $to>",
        1,
    );
    let (gone, new) = diff(&golden_lines(), &extract(&files));
    let upcasts = golden_lines()
        .iter()
        .filter(|l| l.starts_with("abi: impl<'s> From<Local<'s, "))
        .count();
    assert!(upcasts >= 11, "{upcasts} upcast lines");
    assert_eq!(gone.len(), upcasts, "{gone:?}");
    assert_eq!(new.len(), upcasts, "{new:?}");
    assert!(
        gone.contains(&"abi: impl<'s> From<Local<'s, JsObject>> for Local<'s, JsValue>".into())
    );
    assert!(
        new.contains(&"abi: impl<'s, 't> From<Local<'s, JsObject>> for Local<'t, JsValue>".into())
    );
    // A new entry of the list is an addition only.
    let mut files = sources();
    let (_, t) = files.iter_mut().find(|(p, _)| p == "abi/local.rs").unwrap();
    let from = "upcast! {\n";
    assert!(t.contains(from));
    *t = t.replacen(from, "upcast! {\n    JsKey => JsObject,\n", 1);
    let (gone, new) = diff(&golden_lines(), &extract(&files));
    assert!(gone.is_empty(), "{gone:?}");
    assert_eq!(
        new,
        ["abi: impl<'s> From<Local<'s, JsKey>> for Local<'s, JsObject>"]
    );
}

/// The types that give a raw `Value` or a raw cell to safe code: `Value` itself, and the
/// `ep9_heap` types whose safe methods make one (`Heap::alloc`, `Heap::find_cell` and
/// `Heap::last_conservative_roots` give `CellPtr`s; `CellPtr::to_value`,
/// `ScratchRoots::get` and `Slot::get` give `Value`s). An `Isolate` method that gives one of
/// them gives a raw `Value` (Codex W4-1: `heap(&self) -> &Heap` did).
const RAW_TYPES: [&str; 5] = ["Value", "Heap", "CellPtr", "ScratchRoots", "Slot"];

/// The `embed` lines that name a raw type ([`RAW_TYPES`]; the word, so not `JsValue`,
/// `IntrinsicValue` or `HeapConfig`).
fn raw_value_lines(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter(|l| l.starts_with("embed"))
        .filter(|l| {
            l.split(|c: char| !is_ident(c))
                .any(|w| RAW_TYPES.contains(&w))
        })
        .cloned()
        .collect()
}

/// H20-Q4: no default-build `Isolate` method takes or gives a raw `Value` or a type that
/// gives one ([`RAW_TYPES`]), except `string_view` (it stays: the H20 Decision, "Embedders
/// get `Local` too"; it checks its `Value`).
#[test]
fn no_isolate_method_takes_or_gives_a_raw_value() {
    let raw = raw_value_lines(&extract(&sources()));
    assert_eq!(
        raw,
        [
            "embed (not core): impl Isolate :: pub fn string_view(&mut self, Value) -> Option<JsStr<'_>>"
        ]
    );
}

/// The negative control for the other `Isolate` methods (FP2-GOLDEN-RAW): a new pub method in
/// a default-build `impl Isolate` block, in `isolate.rs` or in another file, is a new line,
/// and a raw `Value` form fails `no_isolate_method_takes_or_gives_a_raw_value`. So is a block
/// under a target `cfg` (Codex C2-FP1: `target_pointer_width = "64"` is on for this host),
/// in a block, an inline module or a module of `lib.rs`, and its line shows the `cfg`. A
/// block under a `cfg` that is in no default build (`test`, a feature, `all(test, ..)`), and
/// a file of a module that `lib.rs` gates so (`tests`, `native`), add none.
#[test]
fn a_new_isolate_method_is_a_new_line() {
    let mut files = sources();
    let (_, t) = files.iter_mut().find(|(p, _)| p == "isolate.rs").unwrap();
    t.push_str("\nimpl Isolate {\n    pub fn global(&self, name: &str) -> Option<Value> {\n        None\n    }\n}\n");
    t.push_str("\n#[cfg(feature = \"spike-native\")]\nimpl Isolate {\n    pub fn spike(&self) -> Value {}\n}\n");
    t.push_str(
        "\n#[cfg(all(test, unix))]\nimpl Isolate {\n    pub fn in_test(&self) -> Value {}\n}\n",
    );
    t.push_str("\n#[cfg(target_pointer_width = \"64\")]\nimpl Isolate {\n    pub fn wide(&mut self) -> Value {}\n}\n");
    t.push_str(
        "\n#[cfg(any(test, unix))]\nimpl Isolate {\n    pub fn any_os(&self) -> Value {}\n}\n",
    );
    // Codex W4-1: the raw heap in a default-build block is a raw line; in the block of
    // `isolate.rs` that is gated to tests and spike-native, it is no line.
    t.push_str("\nimpl Isolate {\n    pub fn heap_again(&self) -> &ep9_heap::Heap {}\n}\n");
    t.push_str("\nimpl Isolate {\n    pub fn cell(&self) -> Option<ep9_heap::CellPtr> {}\n}\n");
    assert!(t.contains("#[cfg(any(test, feature = \"spike-native\"))]\nimpl Isolate {\n    pub fn heap(&self) -> &Heap {"));
    files.push((
        "probe.rs".to_string(),
        "mod inner {\n    impl crate::Isolate {\n        pub fn run_raw(&mut self) -> Value {}\n    }\n}\n\
         #[cfg(target_arch = \"aarch64\")]\nmod arm {\n    impl crate::Isolate {\n        pub fn arm_raw(&self) -> Value {}\n    }\n}\n"
            .to_string(),
    ));
    let (_, lib) = files.iter_mut().find(|(p, _)| p == "lib.rs").unwrap();
    lib.push_str("\n#[cfg(debug_assertions)]\nmod probe_debug;\n");
    files.push((
        "probe_debug.rs".to_string(),
        "impl crate::Isolate {\n    pub fn debug_raw(&self) -> Value {}\n}\n".to_string(),
    ));
    assert!(
        sources()
            .iter()
            .all(|(p, _)| !p.starts_with("tests/") && p != "native.rs")
    );
    let (gone, new) = diff(&golden_lines(), &extract(&files));
    assert!(gone.is_empty(), "{gone:?}");
    let mut want = [
        "embed (not core): impl Isolate :: pub fn global(&self, &str) -> Option<Value>",
        "embed (not core): impl Isolate :: pub fn run_raw(&mut self) -> Value",
        "embed (not core): impl Isolate :: #[cfg(target_pointer_width = \"64\")] pub fn wide(&mut self) -> Value",
        "embed (not core): impl Isolate :: #[cfg(any(test, unix))] pub fn any_os(&self) -> Value",
        "embed (not core): impl Isolate :: #[cfg(target_arch = \"aarch64\")] pub fn arm_raw(&self) -> Value",
        "embed (not core): impl Isolate :: #[cfg(debug_assertions)] pub fn debug_raw(&self) -> Value",
        "embed (not core): impl Isolate :: pub fn heap_again(&self) -> &ep9_heap::Heap",
        "embed (not core): impl Isolate :: pub fn cell(&self) -> Option<ep9_heap::CellPtr>",
    ];
    want.sort_unstable();
    assert_eq!(new, want);
    assert_eq!(raw_value_lines(&new), want);
}

/// The `cfg` reader on fixed input (Codex C2-FP1): `false` only for a predicate that is false
/// in every default build of every target; a target or profile name is unknown (`None`),
/// so its items stay in the golden.
#[test]
fn the_cfg_reader_knows_the_default_build() {
    for (p, v) in [
        ("test", Some(false)),
        ("feature = \"spike-native\"", Some(false)),
        ("not(test)", Some(true)),
        ("not(feature = \"ic-stats\")", Some(true)),
        ("any(test, feature = \"spike-native\")", Some(false)),
        ("all(debug_assertions, test)", Some(false)),
        ("target_pointer_width = \"64\"", None),
        ("target_arch = \"x86_64\"", None),
        ("unix", None),
        ("debug_assertions", None),
        ("not(miri)", None),
        ("any(test, unix)", None),
        ("all(target_arch = \"x86_64\", panic = \"unwind\")", None),
        ("all(not(test), not(feature = \"become\"))", Some(true)),
        ("testing", None),
        ("allocator", None),
    ] {
        assert_eq!(cfg_value(p), v, "cfg({p})");
    }
    let attrs = |a: &str| vec![a.to_string()];
    assert!(gated(&attrs("#[cfg(test)]")));
    assert!(!gated(&attrs("#[cfg(target_pointer_width = \"64\")]")));
    assert!(!gated(&attrs("#[cfg(not(test))]")));
    assert!(!gated(&attrs("#[inline]")));
}

/// A new `Cx` method is an addition only: no golden line goes away.
#[test]
fn a_new_cx_method_is_an_addition() {
    let mut files = sources();
    let (_, t) = files.iter_mut().find(|(p, _)| p == "abi/time.rs").unwrap();
    let from = "impl<'s> Cx<'s> {";
    assert!(t.contains(from));
    *t = t.replacen(
        from,
        "impl<'s> Cx<'s> {\n    /// A probe.\n    pub fn probe_method(\n        &mut self,\n        v: Local<'s>,\n    ) -> bool {\n        let _ = v;\n        true\n    }\n",
        1,
    );
    let (gone, new) = diff(&golden_lines(), &extract(&files));
    assert!(gone.is_empty(), "{gone:?}");
    assert_eq!(
        new,
        ["abi: impl<'s> Cx<'s> :: pub fn probe_method(&mut self, Local<'s>) -> bool"]
    );
}

/// A parameter rename, a moved method, a doc change and a body change keep the golden.
#[test]
fn renames_and_bodies_keep_the_golden() {
    let mut files = sources();
    let (_, t) = files.iter_mut().find(|(p, _)| p == "abi/mod.rs").unwrap();
    let from = "pub fn get(&self, i: usize) -> Local<'s> {";
    assert!(t.contains(from));
    *t = t.replacen(
        from,
        "/// Changed text { [ ( \" '{'\n    pub fn get(&self, index_renamed: usize) -> Local<'s> {\n        let _ = \"}}}\";",
        1,
    );
    let (gone, new) = diff(&golden_lines(), &extract(&files));
    assert!(gone.is_empty() && new.is_empty(), "{gone:?} {new:?}");
}

/// The extractor on a small source: literals and comments with brackets, a lifetime, a
/// multi-line signature with a `where` clause, private and `pub(crate)` items, a private
/// module, a trait impl, a tuple struct, an enum, and a local `macro_rules!` with its call
/// (expanded: the line is the generated impl).
#[test]
fn the_extractor_reads_rust_items() {
    let src = r##"
//! Docs { [ (
use x::y;
/* a { block /* nested } */ comment */
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct T<'s>(pub u64, u32);
pub struct S { pub a: u8, b: u16, #[cfg(test)] pub c: u8 }
#[non_exhaustive]
pub enum E { A, B(u8), C { x: u8 } = 3 }
pub struct Cx<'s> { inner: &'s u8 }
struct Hidden;
mod private { pub fn no() {} }
impl<'s> Cx<'s> {
    /// Text with '{' and "}" and r#"}"#.
    #[inline]
    pub fn one<F: Fn(u8) -> u8>(
        &mut self,
        mut a: Local<'s, JsObject>,
        (b, c): (u8, u8),
        f: F,
    ) -> JsResult<()>
    where
        F: Copy,
    {
        let s = "{"; let ch = '}'; let l: &'static str = r"}"; let e = '\'';
        Ok(())
    }
    pub(crate) fn two(&self) {}
    fn three(&self) {}
    pub const K: u32 = 1 << 3;
}
impl<T> Clone for T<'_> { fn clone(&self) -> Self { *self } }
impl Hidden { pub fn not_shown(&self) {} }
pub fn free(x: &str) -> bool { x.is_empty() }
macro_rules! mk {
    ($($a:ident => $b:ty,)*) => {$(
        impl<'s> From<Local<'s, $a>> for $b { fn from(_: Local<'s, $a>) -> Self { todo!() } }
    )*};
}
mk! { S => E, T<'s> => S, }
"##;
    let stripped = strip(src);
    assert!(!stripped.contains("Docs") && !stripped.contains("nested"));
    let files = vec![("abi/probe.rs".to_string(), src.to_string())];
    let lines = extract(&files);
    let want = [
        "abi: #[derive(Clone, Copy)] #[repr(transparent)] pub struct T<'s>(pub u64, _)",
        "abi: #[non_exhaustive] pub enum E { A, B(u8), C { x: u8 } = 3 }",
        "abi: impl<'s> Cx<'s> :: pub const K: u32 = 1 << 3",
        "abi: impl<'s> Cx<'s> :: pub fn one<F: Fn(u8) -> u8>(&mut self, Local<'s, JsObject>, (u8, u8), F) -> JsResult<()> where F: Copy",
        "abi: impl<T> Clone for T<'_>",
        "abi: impl<'s> From<Local<'s, S>> for E",
        "abi: impl<'s> From<Local<'s, T<'s>>> for S",
        "abi: pub fn free(&str) -> bool",
        "abi: pub struct S { pub a: u8, #[cfg(test)] pub c: u8, .. }",
    ];
    let mut want: Vec<String> = want.iter().map(|s| s.to_string()).collect();
    // The probe file is not `abi/mod.rs`, so its pub items have no path.
    for n in ["Cx", "E", "S", "T", "free"] {
        want.push(format!(
            "path: (none) {n}: pub in the private module abi::probe, not re-exported"
        ));
    }
    want.push("abi: pub struct Cx<'s> { .. }".to_string());
    want.sort();
    assert_eq!(lines, want);
}

/// The public path rule (H20 "Public paths"; M3-P3 FREEZE-PREP task 3): every name of
/// `ep9_vm::abi` is also a crate-root name, except `export_facts` (part 2, V8-A13).
#[test]
fn every_abi_name_has_a_crate_root_path() {
    let lines = extract(&sources());
    let abi: BTreeSet<String> = lines
        .iter()
        .filter_map(|l| l.strip_prefix("path: ep9_vm::abi::"))
        .filter(|n| !n.starts_with("export_facts"))
        .map(String::from)
        .collect();
    let root: BTreeSet<String> = lines
        .iter()
        .filter_map(|l| l.strip_prefix("path: ep9_vm::"))
        .filter_map(|l| l.split_once(" = abi::"))
        .map(|(n, _)| n.to_string())
        .collect();
    let missing: Vec<&String> = abi.difference(&root).collect();
    assert!(
        missing.is_empty(),
        "no crate-root re-export in lib.rs for {missing:?}"
    );
}
