//! The H20 core golden (docs/adr/H20-builtin-abi.md, "Freeze table"; M3-P3 FREEZE-PREP task
//! 5): `tests/golden/h20-core.txt` holds the public names and signatures of the builtin ABI
//! and of the embedder API that this extractor reads from the source text.
//!
//! It is a tripwire, not a complete list (M3-P3 fix round 7, decision
//! M3-P3-H20-GOLDEN-TRIPWIRE): a reader of source text cannot list a Rust API. It reads the
//! forms below; a change in another form gives no line. A complete check needs compiler data
//! (rustdoc JSON or a public-API tool) and is M1 work (H20-Q6).
//!
//! ```text
//! src/abi/*.rs     every pub item: types (fields, variants, derives), traits, the pub items
//!                  of their impl blocks (`Cx`, `Args`, `Local`, ...), the trait impls whose
//!                  self type is a pub abi type or whose trait is a pub abi trait (also the
//!                  impls that a local `macro_rules!` makes: the `upcast!` and `downcast!`
//!                  lists, expanded, so the line has the generated signature with its
//!                  lifetimes), the `pub use` re-exports
//! src/isolate.rs   the `impl Cx` block (the scoped embedder forms)
//! src/**/*.rs      in the default build (below):
//!                  - every pub method of every `impl Isolate` block (also `impl
//!                    crate::Isolate` and `impl Isolate where ..`): the embedder entry
//!                    points (`ENTRY_POINTS`: the calls that run ABI code or take a hook that
//!                    is called while ABI code runs; H20-Q4) are core `embed:` lines; the other
//!                    methods are `embed (not core):` lines
//!                  - every other impl whose head names `Isolate` (a trait impl such as
//!                    `impl Drop for Isolate`, or `impl From<&Isolate> for X`): one
//!                    `embed (not core):` line with its members
//!                  - every free pub fn, const or static outside `abi/` whose line names
//!                    `Isolate` or a raw type (`RAW_TYPES`), in any module
//!                  - the types that those lines name, to the end (the type closure): the type
//!                    line (public fields, variants), and the pub methods and trait impls of
//!                    the impls whose self type is that type;
//!                    core `embed:` lines if a core line names the type (`Exception`,
//!                    `IsolateConfig`, `CompileResult`, ...), else `embed (not core):` lines
//! src/builtins.rs  the builtin table types and `check_tables`
//! src/lib.rs       every `pub use` and `pub mod` (the public paths); each re-exported item of
//!                  a module of the crate and each pub item of a pub module (`versions`,
//!                  `internals`) is read as the type closure reads a type
//! ```
//!
//! The default build: an item is read unless a `cfg` on it, on an inline module around it or
//! on its module in `lib.rs` is false in every default build of every target: `test` and the
//! features in `OFF_FEATURES` (`spike-native`, `ic-stats`: dep-guard 11 keeps them out of the
//! ep9 graph; test `only_the_off_features_are_off` reads the Cargo.toml files). A target `cfg`
//! and every other feature (`jit` is on in the default ep9_cli build) are read, and the line
//! shows the `cfg` (Codex C2-FP1; M3-P3 fix round 6, RV5-F1 M1).
//!
//! Some forms that it cannot read are `unread:` lines, and test `the_extractor_reads_every_form`
//! fails on them: a type alias of `Isolate` (or one that renames a raw type), a `use` that
//! renames `Isolate` or a raw type, an impl that names `Isolate` inside a fn, block or
//! `macro_rules!` body, a glob re-export, a module file of a pub module. With these rules the
//! escape classes of the fix-round-5 review (RV5-F1 M1-M5, RV5-S1) give a line.
//!
//! Known escape classes (no line, and every test here passes; H20 Decision, "The golden is a
//! tripwire", classes (a)-(f); review checks them, and H20-Q6 has them as its test list):
//! (a) an item that a macro makes outside the core files (decision M3-P3-H20-GOLDEN-LIMIT; a
//! re-exported one is a line that says so); (b) a `#[path]` module, and a module file outside
//! `src/`, not named `.rs` or under a gated module directory (`tests`, `native`) (RV6-G1);
//! (c) a trait impl with a foreign self type (`impl From<Local<'s>> for Value`), and in `abi/`
//! a trait impl whose trait and self type are not pub abi items (RV6-G2, RV6-A1); (d) an
//! alias, a renaming `use` or a body impl of a core closure type such as `Exception` (RV6-G3);
//! (e) an impl of an abi type (`Local`, `Cx`) outside the `abi/` files and `builtins.rs`
//! (`impl Cx` of `isolate.rs` is read) (RV6-G4); (f) the types of other crates (`ep9_heap`,
//! `ep9_value`, ...).
//!
//! The `embed (not core):` lines are the embedder API of H22, H23 and H19, with their own
//! freeze points. They are here so that a new way for an embedder to reach the heap in a form
//! that this file reads (for example a raw `Value` form of `Isolate` that comes back, H20-Q4)
//! is a new golden line that a review must bless; test `no_embed_line_names_a_raw_type` also
//! fails on a line that names a raw type.
//!
//! A line is a signature without parameter names (a rename is not an API change), with the
//! attributes that matter to a caller (`derive`, `repr`, `non_exhaustive`, `cfg`,
//! `doc(hidden)`, `deprecated`). Comments, doc text and bodies are not read. In the `embed`
//! lines a const or static shows its name and type, not its value. Not here: the
//! append-only lists that have their own goldens (the `IntrinsicId` and `ObjectClass` variants,
//! `tests/golden/vm-builtin-ids.txt`, `tests/golden/object-classes.txt`), private and
//! `pub(crate)` items. `export_facts` (H20 part 2, V8-A13) is not core: it has its path and an
//! `embed (not core):` line (it takes `&Isolate`).
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

/// The ep9_vm features that are off in every shipped build: spike and report features that
/// dep-guard 11 keeps out of the ep9 graph and that no workspace crate turns on by default
/// (test `only_the_off_features_are_off`). Every other feature is read as a target `cfg`: on
/// in some default build (`jit`: the default build of ep9_cli turns on `ep9_vm/jit`, so the
/// shipped ep9 has it; M3-P3 fix round 6, RV5-F1 M1), so its items are in the golden.
const OFF_FEATURES: [&str; 2] = ["ic-stats", "spike-native"];

/// The value of the `cfg` predicate `p` in every default build of the crate, for any target:
/// `Some(false)` if it is false there (`test` is off, and so is `feature = ..` of each of
/// [`OFF_FEATURES`]), `Some(true)` if it is true there, `None` if it depends on the target, the
/// profile or the build (`target_pointer_width = "64"`, `unix`, `debug_assertions`, `miri`, any
/// other feature, any other name). `not`, `all` and `any` combine the values (Codex C2-FP1: a
/// target `cfg` is on in some default build, so its items are in the golden).
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
    if name == "test" {
        return Some(false);
    }
    if name == "feature" {
        let f = p.split_once('=').map(|(_, v)| v.trim().trim_matches('"'));
        if f.is_some_and(|f| OFF_FEATURES.contains(&f)) {
            return Some(false);
        }
    }
    None
}

/// True if an item with these attributes is in no default build: a `cfg` whose predicate is
/// false there ([`cfg_value`]: `cfg(test)`, `cfg(feature = "spike-native")`,
/// `cfg(all(test, unix))`). A target `cfg` (`cfg(target_pointer_width = "64")`,
/// `cfg(any(test, unix))`), another feature (`cfg(feature = "jit")`) and `cfg(not(test))` are
/// not gated: the item is read, and its line shows the `cfg`.
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
#[derive(Clone, Debug)]
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

/// True if the head is an impl head (`impl`, `impl<..>`, `unsafe impl`).
fn is_impl_head(head: &str) -> bool {
    let h = head.strip_prefix("unsafe ").unwrap_or(head);
    h == "impl" || h.starts_with("impl ") || h.starts_with("impl<")
}

/// True if `s` has the word `w` (an identifier, so `Isolate` is not in `IsolateConfig`).
fn has_word(s: &str, w: &str) -> bool {
    s.split(|c: char| !is_ident(c)).any(|x| x == w)
}

/// The identifiers of `s`, as the type closure reads them ([`type_closure`]).
fn words(s: &str) -> impl Iterator<Item = &str> {
    s.split(|c: char| !is_ident(c))
        .filter(|w| w.starts_with(|c: char| c.is_alphabetic() || c == '_'))
}

/// The name of an impl's self type: `impl<'s> abi::Cx<'s>` -> `Cx`; `impl X for &str` ->
/// `&str`; `impl Isolate where Isolate: Sized` -> `Isolate` (the head is read before its
/// `where`, M3-P3 fix round 6, RV5-F1 M2). And the trait name for a trait impl.
fn impl_target(head: &str) -> (Option<String>, String) {
    let head = head.strip_prefix("unsafe ").unwrap_or(head);
    let head = match head.find(" where ") {
        Some(k) => &head[..k],
        None => head.strip_suffix(" where").unwrap_or(head),
    };
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
/// W4-3; fix round 6, RV5-F3): the embedder's ways into ABI code. One rule: an entry point is
/// a method of `Isolate` that runs ABI code (a `Cx` scope, JS or a `HostFn`), or that takes a
/// hook, a function of the embedder that is called while ABI code runs. Only `impl Isolate`
/// members are marked, so `InterruptHandle::request` (a method of `InterruptHandle`, H23-Q6)
/// is not one, and `Isolate::interrupt_handle` takes no hook (fix round 7, RV6-A2). So: a
/// `Cx` scope (`with_cx`, `with_cx_checkpoint`, `create_realm`), the calls that run JS or host
/// functions with no scope of the embedder (`perform_microtask_checkpoint` runs the jobs;
/// `take_unhandled_rejections` runs ToString of an object reason), and the calls that take
/// hooks (`new` takes them in `IsolateConfig`: the builtin tables with their `HostFn`s, the
/// `PromiseRejectionTracker` and the `CompilerHooks`, which `Cx::evaluate_script`, `eval` and
/// `Function()` call; `register_host_function` takes a `HostFn`; `with_module_host` takes the
/// `resolve` and `load` hooks that `import()` calls).
/// The other `Isolate` methods (the handles, `collect_garbage`, `gc_stats`, `string_view`,
/// `compiler`, which gives the hooks back and takes none, ...) are the embedder API of H22,
/// H23 and H19, with their own freeze points: they are `embed (not core):` lines.
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

/// The types that a core line names but that another ADR owns before its own freeze point:
/// their lines are `embed (not core):` lines. `Dispatch` (the `IsolateConfig::dispatch`
/// field) is the list of dispatch expansions of H15, whose H15-Q2 is open.
const NOT_CORE_TYPES: [&str; 1] = ["Dispatch"];

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

/// True for a `mod` head (`mod x`, `pub mod x`, `pub(crate) mod x`).
fn is_mod_head(head: &str) -> bool {
    let words: Vec<&str> = head.split(' ').collect();
    words
        .iter()
        .position(|w| *w == "mod")
        .is_some_and(|k| words[..k].iter().all(|w| w.starts_with("pub")))
}

/// True for a `use` head (`use x`, `pub use x`, `pub(crate) use x`).
fn is_use_head(head: &str) -> bool {
    let words: Vec<&str> = head.split(' ').collect();
    words
        .iter()
        .position(|w| *w == "use")
        .is_some_and(|k| words[..k].iter().all(|w| w.starts_with("pub")))
}

/// True for a type alias head (`type X = ..`, `pub type X<T> = ..`).
fn is_alias_head(head: &str) -> bool {
    let words: Vec<&str> = head.split(' ').collect();
    head.contains('=')
        && words
            .iter()
            .position(|w| *w == "type")
            .is_some_and(|k| words[..k].iter().all(|w| w.starts_with("pub")))
}

/// The module-level items of a stripped file: its top-level items and those of its inline
/// modules (recursively), each with the `cfg` text of the modules around it (`outer` first).
/// An item or a module that is in no default build ([`gated`]) is left out.
fn module_items(its: &[Item], outer: &str, out: &mut Vec<(String, Item)>) {
    for it in its {
        if gated(&it.attrs) {
            continue;
        }
        if is_mod_head(&it.head) {
            if let Some(b) = &it.body {
                let outer = format!("{outer}{}", cfg_text(&it.attrs));
                module_items(&items(b), &outer, out);
            }
            continue;
        }
        out.push((outer.to_string(), it.clone()));
    }
}

/// The signature of a member of an impl or a module, for an `embed` line: a fn without
/// parameter names ([`fn_sig`]), a type as written, a const or static with its name and type
/// (not its value), a macro call as `name! { .. }`. `None` for a `use` or a `mod`.
fn member_sig(m: &Item) -> Option<String> {
    let a = kept_attrs(&m.attrs);
    let h = drop_paths(&m.head);
    if is_use_head(&m.head) || is_mod_head(&m.head) {
        return None;
    }
    let sig = if is_fn_head(&m.head) {
        drop_paths(&fn_sig(&m.head))
    } else if is_alias_head(&m.head) {
        h
    } else if m.head.split(' ').any(|w| w == "const" || w == "static") {
        h.split(" = ").next().unwrap_or("").to_string()
    } else if m.head.ends_with('!') {
        format!("{h} {{ .. }}")
    } else {
        h
    };
    Some(format!("{a}{sig}"))
}

/// The line of an impl that is read as one line: its head and the signature of each member
/// ([`member_sig`]): `impl std::ops::Deref for Isolate { type Target = Heap; fn deref(&self)
/// -> &Heap }`. So a raw type in an associated type or a method fails
/// `no_embed_line_names_a_raw_type`.
fn impl_line(it: &Item) -> String {
    let members: Vec<String> = items(it.body.as_deref().unwrap_or(""))
        .iter()
        .filter(|m| !gated(&m.attrs))
        .filter_map(member_sig)
        .collect();
    let head = drop_paths(&it.head);
    if members.is_empty() {
        head
    } else {
        format!("{head} {{ {} }}", members.join("; "))
    }
}

/// The `Isolate` lines of the module-level items `its` of one file ([`module_items`]):
///
/// - each pub method and const of an `impl Isolate` block (also `impl crate::Isolate` and
///   `impl Isolate where ..`, RV5-F1 M2): an entry point ([`ENTRY_POINTS`]) is a core `embed:`
///   line, any other an `embed (not core):` line;
/// - one `embed (not core):` line ([`impl_line`]) for each other impl whose head names
///   `Isolate`: a trait impl (`impl Drop for Isolate`, `impl AsRef<Heap> for Isolate`,
///   `impl Deref for Isolate`; RV5-F1 M4) or an impl that takes it (`impl From<&Isolate> for
///   &Heap`).
///
/// Each line shows the `cfg` text of the modules and of the block before the member.
fn isolate_lines(its: &[(String, Item)], out: &mut Vec<String>) {
    for (outer, it) in its {
        if !is_impl_head(&it.head) {
            continue;
        }
        let outer = format!("{outer}{}", cfg_text(&it.attrs));
        if impl_target(&it.head) != (None, "Isolate".to_string()) {
            if has_word(&it.head, "Isolate") {
                out.push(format!("{NOT_CORE}{outer}{}", impl_line(it)));
            }
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

/// The `embed` line of a module-level pub item that is not an impl, a `use` or a `mod`: a type
/// (struct, enum, type alias, trait: [`item_lines`]) or a fn, const or static ([`member_sig`]).
/// `None` for an item that is not `pub` (a `pub(crate)` or private item cannot be named from
/// outside the crate).
fn embed_item_line(g: &str, outer: &str, it: &Item) -> Option<String> {
    let h = &it.head;
    if !is_pub(h) || is_impl_head(h) || is_use_head(h) || is_mod_head(h) {
        return None;
    }
    let ws: Vec<&str> = h.split(' ').collect();
    let is_type = ["struct", "enum", "trait", "union"]
        .iter()
        .any(|k| ws.contains(k))
        || is_alias_head(h);
    if is_type && !ws.contains(&"union") {
        let mut v = Vec::new();
        item_lines(g, it, &mut v);
        return v
            .pop()
            .map(|l| drop_paths(&l.replacen(&format!("{g}: "), &format!("{g}: {outer}"), 1)));
    }
    Some(format!("{g}: {outer}{}", member_sig(it)?))
}

/// True for a name that an `embed` line must not hide (RV5-F1 M5): `Isolate` and the raw
/// types ([`RAW_TYPES`]).
fn names_embed_type(s: &str) -> bool {
    has_word(s, "Isolate") || RAW_TYPES.iter().any(|t| has_word(s, t))
}

/// The forms the extractor cannot read safely, each as an `unread:` line (the golden test and
/// `the_extractor_reads_every_form` fail on it; RV5-F1 M3): a type alias of `Isolate` (an
/// `impl IsoAlias` block is an `impl Isolate` block under another name), a type alias that
/// gives a raw type ([`RAW_TYPES`]) another name, a `use` that renames `Isolate` or a raw type
/// (`use crate::Isolate as Iso`), and an impl that names `Isolate` inside a fn or block body
/// (or a `macro_rules!` body), which the module-level reader does not see. `ctx`: 0 at module
/// level, 1 in an impl or trait body (an associated type such as `type Target = Heap` is read
/// by [`impl_line`]), 2 in any other body.
fn unread_lines(p: &str, its: &[Item], ctx: u8, out: &mut Vec<String>) {
    for it in its {
        if gated(&it.attrs) {
            continue;
        }
        let h = &it.head;
        if ctx != 1 && is_alias_head(h) {
            let name = item_name(h).unwrap_or_default();
            let rhs = h.split_once('=').map_or("", |(_, r)| r);
            if has_word(rhs, "Isolate")
                || RAW_TYPES
                    .iter()
                    .any(|t| has_word(rhs, t) && !RAW_TYPES.contains(&name.as_str()))
            {
                out.push(format!(
                    "unread: src/{p}: `{h}`: a type alias of Isolate or a raw type"
                ));
            }
        }
        if is_use_head(h) {
            let t = use_text(it);
            let after = t.split_once("use ").map_or("", |(_, a)| a);
            for n in use_names(&format!("pub use {after}")) {
                let Some((path, alias)) = n.split_once(" as ") else {
                    continue;
                };
                let orig = path.rsplit("::").next().unwrap_or("").trim();
                let alias = alias.trim();
                if (orig == "Isolate" || RAW_TYPES.contains(&orig)) && alias != orig && alias != "_"
                {
                    out.push(format!(
                        "unread: src/{p}: `use {n}`: a use that renames Isolate or a raw type"
                    ));
                }
            }
            continue;
        }
        if ctx == 2 && is_impl_head(h) && has_word(h, "Isolate") {
            out.push(format!(
                "unread: src/{p}: `{h}`: an impl that names Isolate inside a body"
            ));
        }
        if let Some(b) = &it.body {
            let next = if is_mod_head(h) {
                0
            } else if is_impl_head(h) || h.split(' ').any(|w| w == "trait") {
                1
            } else {
                2
            };
            unread_lines(p, &items(b), next, out);
        }
    }
}

/// The pub items of the files outside `abi/` and `builtins.rs`, for the type closure
/// ([`type_closure`]): the types by name, the free fns, consts and statics by name, and the
/// impls by the name of their self type (each with its `cfg` text).
#[derive(Default)]
struct Defs {
    types: std::collections::BTreeMap<String, Vec<(String, Item)>>,
    free: std::collections::BTreeMap<String, Vec<(String, Item)>>,
    impls: std::collections::BTreeMap<String, Vec<(String, Item)>>,
}

impl Defs {
    fn add(&mut self, outer: &str, it: &Item) {
        let h = &it.head;
        let entry = (outer.to_string(), it.clone());
        if is_impl_head(h) {
            self.impls.entry(impl_target(h).1).or_default().push(entry);
        } else if is_pub(h) && !is_use_head(h) && !is_mod_head(h) {
            let Some(name) = item_name(h) else {
                return;
            };
            let ws: Vec<&str> = h.split(' ').collect();
            let is_type = ["struct", "enum", "trait", "union"]
                .iter()
                .any(|k| ws.contains(k))
                || is_alias_head(h);
            let map = if is_type {
                &mut self.types
            } else {
                &mut self.free
            };
            map.entry(name).or_default().push(entry);
        }
    }
}

/// The lines of the types that the `embed` lines name, and of the types that those lines name,
/// to the end (decision M3-P3-H20-CORETYPES; RV5-F2): for each name that a pub type of a file
/// outside `abi/` and `builtins.rs` has ([`Defs`]), the type's line ([`item_lines`]: its
/// public fields or variants), the pub methods and consts of its inherent impls and one line
/// per trait impl ([`impl_line`]). The words of each new line are read the same way. Not read:
/// a type of another crate (`HeapConfig`, `VerifiedUnit`, `GcStats`: their crates and ADRs
/// own them), and a name that the files outside `abi/` and `builtins.rs` give no pub type.
/// `Isolate` gets its type line here and its impls from
/// [`isolate_lines`]. A name of `skip` (the `abi` and `builtins.rs` types, which have their own
/// lines) is not read. A root `(name, true)` is an item root (a `lib.rs` re-export or an item of
/// a pub module): its free fn, const or static is read too, and a root that has no item in the
/// source text gets a line that says so. The lines are of group `g`; `done` holds the names read
/// before (the core closure runs first, so a type that a core line names is core).
fn type_closure(
    g: &str,
    roots: Vec<(String, bool)>,
    defs: &Defs,
    skip: &BTreeSet<String>,
    done: &mut (BTreeSet<String>, BTreeSet<String>),
    out: &mut Vec<String>,
) {
    let mut work = roots;
    while let Some((name, item_root)) = work.pop() {
        if skip.contains(&name) {
            continue;
        }
        let mut lines = Vec::new();
        let mut found = false;
        if done.0.insert(name.clone()) {
            for (outer, it) in defs.types.get(&name).into_iter().flatten() {
                found = true;
                lines.extend(embed_item_line(g, outer, it));
            }
            // The impls of a pub type only (a `pub(crate)` type or a generic parameter of the
            // same name is not one).
            if found && name != "Isolate" {
                for (outer, it) in defs.impls.get(&name).into_iter().flatten() {
                    let outer = format!("{outer}{}", cfg_text(&it.attrs));
                    if impl_target(&it.head).0.is_some() {
                        lines.push(format!("{g}: {outer}{}", impl_line(it)));
                        continue;
                    }
                    let ih = drop_paths(&it.head);
                    for m in items(it.body.as_deref().unwrap_or("")) {
                        if is_pub(&m.head) && !gated(&m.attrs) {
                            lines
                                .extend(member_sig(&m).map(|s| format!("{g}: {ih} :: {outer}{s}")));
                        }
                    }
                }
            }
        } else {
            found = true;
        }
        if item_root && done.1.insert(name.clone()) {
            for (outer, it) in defs.free.get(&name).into_iter().flatten() {
                found = true;
                lines.extend(embed_item_line(g, outer, it));
            }
            if !found {
                lines.push(format!(
                    "{g}: {name}: no item in the source text (a macro makes it: \
                     M3-P3-H20-GOLDEN-LIMIT)"
                ));
            }
        }
        for l in &lines {
            work.extend(words(l).map(|w| (w.to_string(), false)));
        }
        out.extend(lines);
    }
}

/// True for the files whose items (not only their `impl Isolate` blocks) the golden reads.
fn is_core_file(p: &str) -> bool {
    p.starts_with("abi/") || ["isolate.rs", "builtins.rs", "lib.rs"].contains(&p)
}

/// The file of the module `m` that `lib.rs` declares (`m.rs` or `m/mod.rs`), if it is read.
fn mod_file<'a>(stripped: &'a [(String, Vec<Item>)], m: &str) -> Option<&'a (String, Vec<Item>)> {
    stripped
        .iter()
        .find(|(p, _)| *p == format!("{m}.rs") || *p == format!("{m}/mod.rs"))
}

/// The `path:` lines of the `pub use` items of a module at path `at` (`ep9_vm` or
/// `ep9_vm::versions`), and their item roots for [`type_closure`]: each re-export of an item of
/// a module that `lib.rs` declares (`local`), except `abi` and `builtins` (their lines are
/// core). A glob re-export is an `unread:` line: the golden cannot list its names.
fn use_paths(
    at: &str,
    it: &Item,
    local: &[String],
    out: &mut Vec<String>,
    roots: &mut Vec<(String, bool)>,
) {
    for n in use_names(&use_text(it)) {
        let n = norm(&n);
        if n.ends_with('*') {
            out.push(format!("unread: {at}: `pub use {n}`: a glob re-export"));
            continue;
        }
        let (path, alias) = match n.split_once(" as ") {
            Some((p, a)) => (p.trim().to_string(), a.trim().to_string()),
            None => (n.clone(), n.rsplit("::").next().unwrap().to_string()),
        };
        let shown = if at == "ep9_vm" {
            n.clone()
        } else {
            drop_paths(&n)
        };
        out.push(format!("path: {at}::{alias} = {shown}"));
        let segs: Vec<&str> = path
            .trim_start_matches("crate::")
            .trim_start_matches("self::")
            .split("::")
            .collect();
        let module = segs[0];
        if segs.len() > 1
            && local.iter().any(|m| m == module)
            && module != "abi"
            && module != "builtins"
        {
            roots.push((segs[segs.len() - 1].to_string(), true));
        }
    }
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
    let lib: &[Item] = stripped
        .iter()
        .find(|(p, _)| p == "lib.rs")
        .map_or(&[], |(_, its)| its.as_slice());
    // The `cfg` text of each module that `lib.rs` declares (a target `cfg` is read: `gated`).
    let mod_cfgs: Vec<(String, String)> = lib
        .iter()
        .filter(|it| it.body.is_none())
        .filter_map(|it| {
            let name = it.head.strip_prefix("pub ").unwrap_or(&it.head);
            let name = name.strip_prefix("mod ")?;
            Some((name.to_string(), cfg_text(&it.attrs)))
        })
        .collect();
    let local: Vec<String> = lib
        .iter()
        .filter(|it| is_mod_head(&it.head))
        .filter_map(|it| it.head.rsplit(' ').next().map(String::from))
        .collect();
    let mut out = Vec::new();
    let mut defs = Defs::default();
    let mut abi_mod_uses: BTreeSet<String> = BTreeSet::new();
    for (p, its) in &stripped {
        let top = p.split('/').next().unwrap_or("").trim_end_matches(".rs");
        let outer = mod_cfgs
            .iter()
            .find(|(m, _)| m == top)
            .map_or("", |(_, c)| c.as_str());
        let mut flat = Vec::new();
        module_items(its, outer, &mut flat);
        isolate_lines(&flat, &mut out);
        unread_lines(p, its, 0, &mut out);
        if !(p.starts_with("abi/") || p == "builtins.rs") {
            for (o, it) in &flat {
                defs.add(o, it);
                // A free pub fn, const or static that names `Isolate` or a raw type (RV5-F1
                // M5), in any module: a line, so the raw-type guard reads it.
                if let Some(l) = embed_item_line("embed (not core)", o, it)
                    && !is_alias_head(&it.head)
                    && !["struct", "enum", "trait", "union"]
                        .iter()
                        .any(|k| it.head.split(' ').any(|w| w == *k))
                    && names_embed_type(&l)
                {
                    out.push(l);
                }
            }
        }
        if !is_core_file(p) || p == "lib.rs" {
            continue;
        }
        let g = group(p);
        let is_abi = p.starts_with("abi/") || p == "builtins.rs";
        for it in its {
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
            if is_impl_head(&it.head) && has_word(&it.head, "Isolate") {
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
    // The public paths of `lib.rs` (every `pub use` and `pub mod`; RV5-F1 M5) and the items of
    // its pub modules other than `abi`: the item roots of the type closure.
    let mut item_roots = Vec::new();
    for it in lib.iter().filter(|it| !gated(&it.attrs)) {
        if it.head.starts_with("pub use ") {
            use_paths("ep9_vm", it, &local, &mut out, &mut item_roots);
        } else if it.head.starts_with("pub mod ") {
            let m = it.head.trim_start_matches("pub mod ").to_string();
            out.push(format!(
                "path: ep9_vm::{m} = {}pub mod {m}",
                kept_attrs(&it.attrs)
            ));
            if m == "abi" {
                continue;
            }
            let its: Vec<Item> = match &it.body {
                Some(b) => items(b),
                None => mod_file(&stripped, &m).map_or(Vec::new(), |(_, its)| its.clone()),
            };
            let mut flat = Vec::new();
            module_items(&its, "", &mut flat);
            for (_, m_it) in &flat {
                if m_it.head.starts_with("pub use ") {
                    use_paths(
                        &format!("ep9_vm::{m}"),
                        m_it,
                        &local,
                        &mut out,
                        &mut item_roots,
                    );
                } else if is_pub(&m_it.head) && !is_impl_head(&m_it.head) {
                    item_roots.extend(item_name(&m_it.head).map(|n| (n, true)));
                }
            }
            for m_it in &its {
                if m_it.head.starts_with("pub mod ") && m_it.body.is_none() && !gated(&m_it.attrs) {
                    out.push(format!(
                        "unread: ep9_vm::{m}: `{}`: a module file of a pub module",
                        m_it.head
                    ));
                }
            }
        }
    }
    // The type closure: core from the core lines, then the rest (RV5-F2).
    let skip: BTreeSet<String> = pub_types.union(&pub_traits).cloned().collect();
    let core_skip: BTreeSet<String> = skip
        .iter()
        .cloned()
        .chain(NOT_CORE_TYPES.map(String::from))
        .collect();
    let mut done = Default::default();
    let core_roots: Vec<(String, bool)> = out
        .iter()
        .filter(|l| l.starts_with("embed: "))
        .flat_map(|l| words(l).map(|w| (w.to_string(), false)).collect::<Vec<_>>())
        .collect();
    let mut closure = Vec::new();
    type_closure(
        "embed",
        core_roots,
        &defs,
        &core_skip,
        &mut done,
        &mut closure,
    );
    let mut other_roots: Vec<(String, bool)> = out
        .iter()
        .chain(&closure)
        .filter(|l| l.starts_with(NOT_CORE))
        .flat_map(|l| words(l).map(|w| (w.to_string(), false)).collect::<Vec<_>>())
        .collect();
    other_roots.extend(item_roots);
    other_roots.extend(NOT_CORE_TYPES.map(|n| (n.to_string(), false)));
    other_roots.reverse();
    type_closure(
        "embed (not core)",
        other_roots,
        &defs,
        &skip,
        &mut done,
        &mut closure,
    );
    out.extend(closure);
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

// ---- the golden ------------------------------------------------------------------------------

const HEADER: &str = "\
# H20 core: the public names and signatures of the builtin ABI (docs/adr/H20-builtin-abi.md,
# Freeze table), as the extractor reads them from the source text: a tripwire, not a complete
# list (H20 Decision, the tripwire paragraph; H20-Q6). Frozen at M0 exit: after it, a
# changed or removed core line is a change to a Frozen part; a new line is an addition (part 2,
# append only). The `embed (not core):` lines are the embedder API of H22, H23, H19 that the
# extractor reads: not core, but a new one needs a review (H20-Q4). Made by
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
/// embedder forms of isolate.rs), the table types, the public paths, and the types that the
/// entry points name (RV5-F2: `Exception`, `IsolateConfig` with its fields, `CompileResult`,
/// `CompileFailure`, `CompilerHooks`, `IsolateError`).
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
        "embed: #[derive(Clone, Debug, PartialEq, Eq)] pub struct Exception { pub message: String,",
        "embed: #[derive(Clone, Debug)] pub struct IsolateConfig { pub heap: HeapConfig,",
        "pub promise_rejection_tracker: Option<PromiseRejectionTracker> }",
        "embed: #[derive(Clone, Copy, Debug)] pub struct CompilerHooks {",
        "embed: pub type CompileResult = Result<VerifiedUnit, CompileFailure>",
        "embed: #[derive(Clone, Debug, PartialEq, Eq)] pub enum CompileFailure {",
        "embed: #[derive(Debug)] pub enum IsolateError {",
        "embed: pub struct Isolate { .. }",
        "path: ep9_vm::Value = ep9_value::Value",
        "path: ep9_vm::versions = pub mod versions",
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
    let embed: Vec<&String> = lines
        .iter()
        .filter(|l| l.starts_with("embed: impl Isolate :: "))
        .collect();
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

/// The lines that name a raw type ([`RAW_TYPES`]; the word, so not `JsValue`,
/// `IntrinsicValue` or `HeapConfig`): every line but the `path:` lines (`ep9_vm::Value` is a
/// re-export of the type, not a producer) and the `unread:` lines (they fail on their own).
fn raw_value_lines(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter(|l| !l.starts_with("path: ") && !l.starts_with("unread: "))
        .filter(|l| {
            l.split(|c: char| !is_ident(c))
                .any(|w| RAW_TYPES.contains(&w))
        })
        .cloned()
        .collect()
}

/// H20-Q4: no line of the golden names a raw `Value` or a type that gives one
/// ([`RAW_TYPES`]): no default-build `Isolate` method, impl that names `Isolate`, free pub
/// item that names `Isolate`, re-exported item, item of a pub module or type that one of them
/// names takes or gives one (M3-P3 fix round 6, RV5-F1). It reads the lines only, so it does
/// not see the escape classes of the file header. Two exact lines are allowed:
/// `string_view` (it stays: the H20 Decision, "Embedders get `Local` too"; it checks its
/// `Value`) and `IsolateError`, whose variant is named `Heap` (it holds a `HeapError`). A
/// changed line of either is not allowed.
#[test]
fn no_embed_line_names_a_raw_type() {
    let raw = raw_value_lines(&extract(&sources()));
    assert_eq!(
        raw,
        [
            "embed (not core): impl Isolate :: pub fn string_view(&mut self, Value) -> Option<JsStr<'_>>",
            "embed: #[derive(Debug)] pub enum IsolateError { Heap(HeapError), Config(&'static str), BuiltinTable(String) }",
        ]
    );
}

/// No `unread:` line: the source has none of the forms that the extractor knows it cannot read
/// (a type alias of `Isolate` or a renamed raw type, a `use` that renames one, an impl that
/// names `Isolate` in a body, a glob re-export, a module file of a pub module). The escape
/// classes of the file header give no `unread:` line (a tripwire, H20-Q6).
#[test]
fn the_extractor_reads_every_form() {
    let unread: Vec<String> = extract(&sources())
        .into_iter()
        .filter(|l| l.starts_with("unread: "))
        .collect();
    assert!(unread.is_empty(), "{unread:#?}");
}

/// The features that `cfg_value` reads as off ([`OFF_FEATURES`]) are ep9_vm features that no
/// workspace crate turns on in its default build or in a dependency line (the Cargo.toml files
/// are read, not changed), and `jit`, which the default build of ep9_cli turns on, is seen as
/// on (M3-P3 fix round 6, RV5-F1 M1). dep-guard 11 keeps the off features out of the ep9
/// graph.
#[test]
fn only_the_off_features_are_off() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut manifests = Vec::new();
    for dir in ["crates", "tools"] {
        let mut names: Vec<PathBuf> = std::fs::read_dir(root.join(dir))
            .unwrap()
            .map(|e| e.unwrap().path().join("Cargo.toml"))
            .filter(|p| p.is_file())
            .collect();
        names.sort();
        manifests.extend(names);
    }
    assert!(manifests.len() >= 10, "{manifests:?}");
    let mut on = BTreeSet::new();
    let mut declared = BTreeSet::new();
    for m in &manifests {
        let text = std::fs::read_to_string(m).unwrap();
        let (features, deps) = cargo_features(&text);
        let is_vm = text.contains("\nname = \"ep9_vm\"");
        if is_vm {
            declared.extend(features.iter().map(|(k, _)| k.clone()));
        }
        // The default features of this crate, expanded.
        let mut work = vec!["default".to_string()];
        let mut seen = BTreeSet::new();
        while let Some(f) = work.pop() {
            if !seen.insert(f.clone()) {
                continue;
            }
            for (k, v) in &features {
                if *k != f {
                    continue;
                }
                for e in v {
                    match e.split_once('/') {
                        Some((dep, feat)) if dep.trim_end_matches('?') == "ep9_vm" => {
                            on.insert(feat.to_string());
                        }
                        Some(_) => {}
                        None if is_vm => {
                            on.insert(e.clone());
                            work.push(e.clone());
                        }
                        None => work.push(e.clone()),
                    }
                }
            }
        }
        on.extend(deps);
    }
    for f in OFF_FEATURES {
        assert!(
            declared.contains(f),
            "{f} is not an ep9_vm feature: {declared:?}"
        );
        assert!(
            !on.contains(f),
            "a workspace crate turns on ep9_vm/{f} by default"
        );
    }
    assert!(on.contains("jit"), "{on:?}");
    assert_eq!(cfg_value("feature = \"jit\""), None);
}

/// The `[features]` entries of a Cargo.toml text, and the features of each dependency line on
/// `ep9_vm` (`ep9_vm = { .., features = [..] }` or a `[..dependencies.ep9_vm]` table). A form
/// it cannot read panics.
fn cargo_features(text: &str) -> (Vec<(String, Vec<String>)>, Vec<String>) {
    let lines: Vec<&str> = text
        .lines()
        .map(|l| l.split('#').next().unwrap_or("").trim())
        .collect();
    let list = |s: &str| -> Vec<String> {
        s.trim()
            .trim_start_matches('[')
            .trim_end_matches(']')
            .split(',')
            .map(|x| x.trim().trim_matches('"').to_string())
            .filter(|x| !x.is_empty())
            .collect()
    };
    let mut features = Vec::new();
    let mut deps = Vec::new();
    let mut section = "";
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i];
        i += 1;
        if l.starts_with('[') {
            section = l;
            continue;
        }
        let Some((k, v)) = l.split_once('=') else {
            continue;
        };
        let (k, mut v) = (k.trim(), v.trim().to_string());
        // A list over more lines.
        if v.starts_with('[') && !v.contains(']') {
            while i < lines.len() && !v.contains(']') {
                v.push_str(lines[i]);
                i += 1;
            }
        }
        if section == "[features]" {
            features.push((k.to_string(), list(&v)));
        } else if section.ends_with("dependencies.ep9_vm]") && k == "features" {
            deps.extend(list(&v));
        } else if k == "ep9_vm" && section.contains("dependencies") {
            assert!(v.starts_with('{') && v.ends_with('}'), "ep9_vm = {v}");
            if let Some((_, f)) = v.split_once("features") {
                let f = f.trim_start().trim_start_matches('=');
                let end = f.find(']').expect("a features list");
                deps.extend(list(&f[..=end]));
            }
        }
    }
    (features, deps)
}

/// The negative control for the other `Isolate` methods (FP2-GOLDEN-RAW): a new pub method in
/// a default-build `impl Isolate` block, in `isolate.rs` or in another file, is a new line,
/// and a raw `Value` form fails `no_embed_line_names_a_raw_type`. So is a block
/// under a target `cfg` (Codex C2-FP1: `target_pointer_width = "64"` is on for this host),
/// in a block, an inline module or a module of `lib.rs`, and its line shows the `cfg`. A
/// block under a `cfg` that is in no default build (`test`, `spike-native`, `all(test, ..)`),
/// and a file of a module that `lib.rs` gates so (`tests`, `native`), add none.
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

/// The new golden lines and the new raw lines when `edit` changes the source files.
fn mutate(edit: impl FnOnce(&mut Vec<(String, String)>)) -> (Vec<String>, Vec<String>) {
    let mut files = sources();
    edit(&mut files);
    let (gone, new) = diff(&golden_lines(), &extract(&files));
    assert!(gone.is_empty(), "{gone:?}");
    let raw = raw_value_lines(&new);
    (new, raw)
}

/// Appends `text` to the source file `p` (under `src/`).
fn append(files: &mut [(String, String)], p: &str, text: &str) {
    let (_, t) = files.iter_mut().find(|(q, _)| q == p).unwrap();
    t.push_str(text);
}

/// The negative controls of M3-P3 fix round 6 (RV5-F1 M1-M5, RV5-S1): each way that the
/// verifiers used to bring a `&Heap` producer back with no golden line is now a new line that
/// the raw-type guard fails on, or an `unread:` line. One mutation at a time, on the real
/// source text.
#[test]
fn each_way_to_add_a_raw_producer_fails_the_gate() {
    let iso = "isolate.rs";
    // M1: a block under a feature that a default build turns on (`jit`, from ep9_cli), or
    // that may be on (`become`). An off feature (`ic-stats`) adds no line.
    let (new, raw) = mutate(|f| {
        append(
            f,
            iso,
            "\n#[cfg(feature = \"jit\")]\nimpl Isolate {\n    pub fn heap_jit(&self) -> &Heap { todo!() }\n}\n",
        );
        append(
            f,
            iso,
            "\n#[cfg(feature = \"become\")]\nimpl Isolate {\n    pub fn heap_become(&self) -> &Heap { todo!() }\n}\n",
        );
        append(
            f,
            iso,
            "\n#[cfg(feature = \"ic-stats\")]\nimpl Isolate {\n    pub fn heap_stats(&self) -> &Heap { todo!() }\n}\n",
        );
    });
    let want = [
        "embed (not core): impl Isolate :: #[cfg(feature = \"become\")] pub fn heap_become(&self) -> &Heap",
        "embed (not core): impl Isolate :: #[cfg(feature = \"jit\")] pub fn heap_jit(&self) -> &Heap",
    ];
    assert_eq!(new, want);
    assert_eq!(raw, want);
    // M2: a `where` clause on the block.
    let (new, raw) = mutate(|f| {
        append(
            f,
            iso,
            "\nimpl Isolate where Isolate: Sized {\n    pub fn heap_where(&self) -> &Heap { todo!() }\n}\n",
        );
    });
    let want = ["embed (not core): impl Isolate :: pub fn heap_where(&self) -> &Heap"];
    assert_eq!(new, want);
    assert_eq!(raw, want);
    // M3: a type alias of `Isolate` (an `impl IsoAlias` block), and a `use` that renames it.
    let (new, _) = mutate(|f| {
        append(
            f,
            iso,
            "\ntype IsoAlias = Isolate;\nimpl IsoAlias {\n    pub fn heap_alias(&self) -> &Heap { todo!() }\n}\n",
        );
    });
    assert_eq!(
        new,
        [
            "unread: src/isolate.rs: `type IsoAlias = Isolate`: a type alias of Isolate or a raw type"
        ]
    );
    let (new, _) = mutate(|f| {
        append(
            f,
            "string.rs",
            "\nuse crate::Isolate as Iso;\nimpl Iso {\n    pub fn heap_use(&self) -> &Heap { todo!() }\n}\n",
        );
    });
    assert_eq!(
        new,
        [
            "unread: src/string.rs: `use crate::Isolate as Iso`: a use that renames Isolate or a raw type"
        ]
    );
    let (new, _) = mutate(|f| {
        append(f, "string.rs", "\npub type H = ep9_heap::Heap;\n");
    });
    assert_eq!(
        new,
        [
            "unread: src/string.rs: `pub type H = ep9_heap::Heap`: a type alias of Isolate or a raw type"
        ]
    );
    // M4: trait impls for `Isolate`, and an impl that takes it.
    let (new, raw) = mutate(|f| {
        append(
            f,
            iso,
            "\nimpl std::ops::Deref for Isolate {\n    type Target = Heap;\n    fn deref(&self) -> &Heap { todo!() }\n}\n",
        );
        append(
            f,
            "string.rs",
            "\nimpl AsRef<ep9_heap::Heap> for crate::Isolate {\n    fn as_ref(&self) -> &ep9_heap::Heap { todo!() }\n}\n",
        );
        append(
            f,
            "string.rs",
            "\nimpl<'a> From<&'a crate::Isolate> for &'a ep9_heap::Heap {\n    fn from(i: &'a crate::Isolate) -> Self { todo!() }\n}\n",
        );
    });
    let want = [
        "embed (not core): impl AsRef<ep9_heap::Heap> for Isolate { fn as_ref(&self) -> &ep9_heap::Heap }",
        "embed (not core): impl std::ops::Deref for Isolate { type Target = Heap; fn deref(&self) -> &Heap }",
        "embed (not core): impl<'a> From<&'a Isolate> for &'a ep9_heap::Heap { fn from(&'a Isolate) -> Self }",
    ];
    assert_eq!(new, want);
    assert_eq!(raw, want);
    // M5: a free pub fn in a new module that lib.rs re-exports, in a pub module, and in a
    // private module that nothing re-exports.
    let (new, raw) = mutate(|f| {
        append(f, "lib.rs", "\nmod helpers;\npub use helpers::heap_of;\n");
        f.push((
            "helpers.rs".to_string(),
            "use crate::Isolate;\npub fn heap_of(iso: &Isolate) -> &ep9_heap::Heap { todo!() }\n"
                .to_string(),
        ));
    });
    assert_eq!(
        new,
        [
            "embed (not core): pub fn heap_of(&Isolate) -> &ep9_heap::Heap",
            "path: ep9_vm::heap_of = helpers::heap_of",
        ]
    );
    assert_eq!(
        raw,
        ["embed (not core): pub fn heap_of(&Isolate) -> &ep9_heap::Heap"]
    );
    let (new, raw) = mutate(|f| {
        append(
            f,
            "versions.rs",
            "\npub fn heap_v(iso: &crate::Isolate) -> &ep9_heap::Heap { todo!() }\n",
        );
        append(
            f,
            "frame.rs",
            "\npub static HEAP_OF: fn(&crate::Isolate) -> &ep9_heap::Heap = |_| todo!();\n",
        );
    });
    let want = [
        "embed (not core): pub fn heap_v(&Isolate) -> &ep9_heap::Heap",
        "embed (not core): pub static HEAP_OF: fn(&Isolate) -> &ep9_heap::Heap",
    ];
    assert_eq!(new, want);
    assert_eq!(raw, want);
    // A re-exported fn whose signature names a local type that holds the raw type: the type
    // closure reads the type.
    let (new, raw) = mutate(|f| {
        append(f, "lib.rs", "\nmod helpers;\npub use helpers::heap_ref;\n");
        f.push((
            "helpers.rs".to_string(),
            "pub struct HeapRef<'a>(pub &'a ep9_heap::Heap);\n\
             pub fn heap_ref(n: u8) -> HeapRef<'static> { todo!() }\n"
                .to_string(),
        ));
    });
    assert_eq!(
        new,
        [
            "embed (not core): pub fn heap_ref(u8) -> HeapRef<'static>",
            "embed (not core): pub struct HeapRef<'a>(pub &'a ep9_heap::Heap)",
            "path: ep9_vm::heap_ref = helpers::heap_ref",
        ]
    );
    assert_eq!(
        raw,
        ["embed (not core): pub struct HeapRef<'a>(pub &'a ep9_heap::Heap)"]
    );
    // The same through an `Isolate` method and through a core entry point's type.
    let (new, raw) = mutate(|f| {
        append(
            f,
            iso,
            "\nimpl Isolate {\n    pub fn heap_ref(&self) -> HeapRef<'_> { todo!() }\n}\npub struct HeapRef<'a>(pub &'a Heap);\n",
        );
    });
    assert_eq!(
        new,
        [
            "embed (not core): impl Isolate :: pub fn heap_ref(&self) -> HeapRef<'_>",
            "embed (not core): pub struct HeapRef<'a>(pub &'a Heap)",
        ]
    );
    assert_eq!(
        raw,
        ["embed (not core): pub struct HeapRef<'a>(pub &'a Heap)"]
    );
    let mut files = sources();
    let (_, t) = files.iter_mut().find(|(p, _)| p == iso).unwrap();
    let from = "    pub terminated: bool,\n";
    assert!(t.contains(from));
    *t = t.replacen(
        from,
        "    pub terminated: bool,\n    pub heap: Option<&'static Heap>,\n",
        1,
    );
    let (gone, new) = diff(&golden_lines(), &extract(&files));
    assert_eq!(gone.len(), 1, "{gone:?}");
    assert!(
        gone[0].starts_with("embed: #[derive(Clone, Debug, PartialEq, Eq)] pub struct Exception {")
    );
    assert_eq!(raw_value_lines(&new), new);
    // An impl that names `Isolate` in a fn body, and a glob re-export.
    let (new, _) = mutate(|f| {
        append(
            f,
            "string.rs",
            "\nfn hidden() {\n    impl crate::Isolate {\n        pub fn heap_in_fn(&self) -> &ep9_heap::Heap { todo!() }\n    }\n}\n",
        );
        append(f, "lib.rs", "\npub use string::*;\n");
    });
    assert_eq!(
        new,
        [
            "unread: ep9_vm: `pub use string::*`: a glob re-export",
            "unread: src/string.rs: `impl crate::Isolate`: an impl that names Isolate inside a body",
        ]
    );
}

/// The `cfg` reader on fixed input (Codex C2-FP1): `false` only for a predicate that is false
/// in every default build of every target; a target or profile name is unknown (`None`),
/// so its items stay in the golden.
#[test]
fn the_cfg_reader_knows_the_default_build() {
    for (p, v) in [
        ("test", Some(false)),
        ("feature = \"spike-native\"", Some(false)),
        ("feature = \"ic-stats\"", Some(false)),
        ("feature = \"jit\"", None),
        ("not(feature = \"jit\")", None),
        ("feature = \"become\"", None),
        ("any(test, feature = \"jit\")", None),
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
        ("all(not(test), not(feature = \"become\"))", None),
        (
            "all(not(test), not(feature = \"spike-native\"))",
            Some(true),
        ),
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
